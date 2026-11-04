"""The camera, through Linux's video interface (Video4Linux2) directly: open, ask for a format, map its buffers
and take pictures as they come. No capture library.

Picture formats: YUYV (uncompressed; turned into RGB here) when the camera offers it at a useful size and
speed, else MJPEG (each picture a JPEG; Pillow decodes it). Structure layouts and request numbers are those of
linux/videodev2.h on 64-bit Linux (tests check them against the header when a C compiler is around).
"""
from __future__ import annotations

import errno
import fcntl
import glob
import io
import mmap
import os
import select
import struct
from dataclasses import dataclass, field
from functools import lru_cache
from typing import Iterator

import numpy as np
from PIL import Image

# Request numbers: _IOC(direction, 'V', number, size of the structure).
VIDIOC_QUERYCAP = 0x80685600
VIDIOC_ENUM_FMT = 0xC0405602
VIDIOC_S_FMT = 0xC0D05605
VIDIOC_REQBUFS = 0xC0145608
VIDIOC_QUERYBUF = 0xC0585609
VIDIOC_QBUF = 0xC058560F
VIDIOC_DQBUF = 0xC0585611
VIDIOC_STREAMON = 0x40045612
VIDIOC_STREAMOFF = 0x40045613
VIDIOC_S_PARM = 0xC0CC5616
VIDIOC_ENUM_FRAMESIZES = 0xC02C564A
VIDIOC_ENUM_FRAMEINTERVALS = 0xC034564B

BUF_TYPE_VIDEO_CAPTURE = 1
BUF_FLAG_ERROR = 0x40        # the picture in the buffer may be damaged (the camera goes on all the same)
MEMORY_MMAP = 1
CAP_VIDEO_CAPTURE = 0x1
CAP_STREAMING = 0x04000000
CAP_DEVICE_CAPS = 0x80000000
FRMSIZE_TYPE_DISCRETE = 1
FRMIVAL_TYPE_DISCRETE = 1


def fourcc(s: str) -> int:
    return s.encode()[0] | s.encode()[1] << 8 | s.encode()[2] << 16 | s.encode()[3] << 24


YUYV = fourcc("YUYV")
MJPG = fourcc("MJPG")
WANT = (640, 480)            # enough for a hand at arm's length or across a room; small to process


class CameraError(OSError):
    pass


@dataclass
class Device:
    path: str
    name: str
    formats: dict[int, list[tuple[int, int, float]]] = field(default_factory=dict)   # fourcc -> (w, h, fps)


def _ioctl(fd: int, request: int, buf: bytearray) -> bytearray:
    while True:
        try:
            fcntl.ioctl(fd, request, buf, True)
            return buf
        except InterruptedError:
            continue


def devices() -> list[Device]:
    """Cameras that can stream pictures, best first: Apple's Studio Display, then any other."""
    found = []
    for node in sorted(glob.glob("/sys/class/video4linux/video*"), key=lambda p: int(p.rsplit("video", 1)[1])):
        path = "/dev/" + os.path.basename(node)
        try:
            with open(os.path.join(node, "name"), encoding="utf-8", errors="replace") as f:
                name = f.read().strip()
        except OSError:
            name = os.path.basename(node)
        try:
            dev = probe(path, name)
        except OSError:
            continue
        if dev is not None:
            found.append(dev)
    return sorted(found, key=lambda d: (0 if "studio display" in d.name.lower() else 1))


def probe(path: str, name: str = "") -> Device | None:
    """The camera at `path` and the formats it offers, or None if it isn't one that streams pictures (a
    camera's second node carries metadata; infrared cameras offer only greyscale)."""
    fd = os.open(path, os.O_RDWR | os.O_NONBLOCK)
    try:
        cap = _ioctl(fd, VIDIOC_QUERYCAP, bytearray(104))
        caps, device_caps = struct.unpack_from("<II", cap, 84)
        if caps & CAP_DEVICE_CAPS:
            caps = device_caps
        if not (caps & CAP_VIDEO_CAPTURE and caps & CAP_STREAMING):
            return None
        card = bytes(cap[16:48]).split(b"\0", 1)[0].decode("utf-8", "replace")
        dev = Device(path, name or card)
        for fmt in _formats(fd):
            if fmt in (YUYV, MJPG):
                dev.formats[fmt] = _sizes(fd, fmt)
        return dev if dev.formats else None
    finally:
        os.close(fd)


def _formats(fd: int) -> list[int]:
    out, i = [], 0
    while True:
        buf = bytearray(64)
        struct.pack_into("<II", buf, 0, i, BUF_TYPE_VIDEO_CAPTURE)
        try:
            _ioctl(fd, VIDIOC_ENUM_FMT, buf)
        except OSError:
            return out
        out.append(struct.unpack_from("<I", buf, 44)[0])
        i += 1


def _sizes(fd: int, fmt: int) -> list[tuple[int, int, float]]:
    out, i = [], 0
    while i < 64:
        buf = bytearray(44)
        struct.pack_into("<II", buf, 0, i, fmt)
        try:
            _ioctl(fd, VIDIOC_ENUM_FRAMESIZES, buf)
        except OSError:
            break
        kind = struct.unpack_from("<I", buf, 8)[0]
        if kind == FRMSIZE_TYPE_DISCRETE:
            w, h = struct.unpack_from("<II", buf, 12)
            out.append((w, h, _best_fps(fd, fmt, w, h)))
        else:                   # a range: say the size we want, if it's in it
            minw, maxw, _sw, minh, maxh, _sh = struct.unpack_from("<6I", buf, 12)
            w, h = min(max(WANT[0], minw), maxw), min(max(WANT[1], minh), maxh)
            out.append((w, h, 30.0))
            break
        i += 1
    return out


def _best_fps(fd: int, fmt: int, w: int, h: int) -> float:
    best, i = 0.0, 0
    while i < 32:
        buf = bytearray(52)
        struct.pack_into("<IIII", buf, 0, i, fmt, w, h)
        try:
            _ioctl(fd, VIDIOC_ENUM_FRAMEINTERVALS, buf)
        except OSError:
            break
        if struct.unpack_from("<I", buf, 16)[0] == FRMIVAL_TYPE_DISCRETE:
            num, den = struct.unpack_from("<II", buf, 20)
            if num:
                best = max(best, den / num)
        else:
            num, den = struct.unpack_from("<II", buf, 20)            # the fastest end of a range
            best = max(best, den / num if num else 30.0)
            break
        i += 1
    return best or 30.0


def choose(dev: Device) -> tuple[int, int, int]:
    """Format and size: uncompressed when it comes at 640x480 or near and at 15 pictures a second or more;
    else MJPEG, at the size nearest 640x480 (bigger rather than smaller)."""
    def nearest(sizes: list[tuple[int, int, float]]) -> tuple[int, int, float]:
        return min(sizes, key=lambda s: (s[0] < 480, abs(s[0] * s[1] - WANT[0] * WANT[1]), -s[2]))
    yuyv = [s for s in dev.formats.get(YUYV, []) if s[2] >= 15 and 320 <= s[0] <= 1280]
    if yuyv:
        w, h, _ = nearest(yuyv)
        return YUYV, w, h
    if dev.formats.get(MJPG):
        w, h, _ = nearest(dev.formats[MJPG])
        return MJPG, w, h
    w, h, _ = nearest(dev.formats[YUYV])
    return YUYV, w, h


class Camera:
    """One open camera, streaming. frames() gives pictures as they arrive (raw bytes: decode() makes them RGB
    pictures), and stops after `idle_s` without one (the camera was unplugged, say)."""

    def __init__(self, dev: Device, fps: int = 30, buffers: int = 4) -> None:
        self.dev = dev
        self.fmt, self.width, self.height = choose(dev)
        self.fd = os.open(dev.path, os.O_RDWR | os.O_NONBLOCK)
        self.maps: list[mmap.mmap] = []
        try:
            self._setup(fps, buffers)
        except Exception:
            self.close()
            raise

    def _setup(self, fps: int, count: int) -> None:
        f = bytearray(208)
        struct.pack_into("<I", f, 0, BUF_TYPE_VIDEO_CAPTURE)
        struct.pack_into("<IIII", f, 8, self.width, self.height, self.fmt, 1)        # field: none
        _ioctl(self.fd, VIDIOC_S_FMT, f)
        self.width, self.height, self.fmt = struct.unpack_from("<III", f, 8)         # what the camera took
        self.stride = struct.unpack_from("<I", f, 24)[0] or self.width * 2
        p = bytearray(204)
        struct.pack_into("<I", p, 0, BUF_TYPE_VIDEO_CAPTURE)
        struct.pack_into("<II", p, 12, 1, fps)                                         # time per frame 1/fps
        try:
            _ioctl(self.fd, VIDIOC_S_PARM, p)
        except OSError:
            pass                                                                        # its own rate, then
        r = bytearray(20)
        struct.pack_into("<III", r, 0, count, BUF_TYPE_VIDEO_CAPTURE, MEMORY_MMAP)
        _ioctl(self.fd, VIDIOC_REQBUFS, r)
        n = struct.unpack_from("<I", r, 0)[0]
        if n < 2:
            raise CameraError(errno.ENOMEM, "the camera gave too few picture buffers")
        for i in range(n):
            b = self._buffer(i)
            _ioctl(self.fd, VIDIOC_QUERYBUF, b)
            offset, length = struct.unpack_from("<I", b, 64)[0], struct.unpack_from("<I", b, 72)[0]
            self.maps.append(mmap.mmap(self.fd, length, mmap.MAP_SHARED, mmap.PROT_READ | mmap.PROT_WRITE,
                                       offset=offset))
            _ioctl(self.fd, VIDIOC_QBUF, b)
        _ioctl(self.fd, VIDIOC_STREAMON, bytearray(struct.pack("<I", BUF_TYPE_VIDEO_CAPTURE)))

    @staticmethod
    def _buffer(i: int) -> bytearray:
        b = bytearray(88)
        struct.pack_into("<II", b, 0, i, BUF_TYPE_VIDEO_CAPTURE)
        struct.pack_into("<I", b, 60, MEMORY_MMAP)
        return b

    def frames(self, idle_s: float = 3.0) -> Iterator[bytes]:
        while True:
            ready, _, _ = select.select([self.fd], [], [], idle_s)
            if not ready:
                return
            b = self._buffer(0)
            try:
                _ioctl(self.fd, VIDIOC_DQBUF, b)
            except BlockingIOError:
                continue
            except OSError:
                return                      # unplugged
            i = struct.unpack_from("<I", b, 0)[0]
            used, flags = struct.unpack_from("<II", b, 8)
            # A picture the camera says may be damaged, or an uncompressed one cut short, is passed over: the
            # next one is whole.
            whole = used and not flags & BUF_FLAG_ERROR and (self.fmt != YUYV or used >= self.stride * self.height)
            data = bytes(self.maps[i][:used]) if whole else b""
            try:
                _ioctl(self.fd, VIDIOC_QBUF, b)
            except OSError:
                return
            if data:
                yield data

    def decode(self, data: bytes, max_width: int = 960) -> Image.Image:
        if self.fmt == YUYV:
            return yuyv_to_image(data, self.width, self.height, self.stride)
        return mjpeg_to_image(data, max_width)

    def close(self) -> None:
        try:
            _ioctl(self.fd, VIDIOC_STREAMOFF, bytearray(struct.pack("<I", BUF_TYPE_VIDEO_CAPTURE)))
        except OSError:
            pass
        for m in self.maps:
            try:
                m.close()
            except (BufferError, ValueError):
                pass
        self.maps = []
        try:
            os.close(self.fd)
        except OSError:
            pass


def yuyv_to_image(data: bytes, w: int, h: int, stride: int = 0) -> Image.Image:
    """Packed YUYV (Y0 U Y1 V for each two pixels; ITU-R BT.601, studio range) to RGB."""
    stride = stride or w * 2
    raw = np.frombuffer(data, dtype=np.uint8, count=stride * h).reshape(h, stride)[:, : w * 2]
    quad = raw.reshape(h, w // 2, 4).astype(np.float32)
    y = quad[:, :, (0, 2)].reshape(h, w) - 16.0
    u = np.repeat(quad[:, :, 1] - 128.0, 2, axis=1)
    v = np.repeat(quad[:, :, 3] - 128.0, 2, axis=1)
    y *= 1.164
    rgb = np.empty((h, w, 3), dtype=np.float32)
    rgb[:, :, 0] = y + 1.596 * v
    rgb[:, :, 1] = y - 0.392 * u - 0.813 * v
    rgb[:, :, 2] = y + 2.017 * u
    return Image.fromarray(np.clip(rgb, 0, 255).astype(np.uint8), "RGB")


def yuyv_luma(data: bytes, w: int, h: int, step: int = 8, stride: int = 0) -> np.ndarray:
    """A small greyscale picture straight from the YUYV bytes (every `step`th pixel): to see whether
    anything moved, without converting the picture."""
    stride = stride or w * 2
    raw = np.frombuffer(data, dtype=np.uint8, count=stride * h).reshape(h, stride)
    return raw[::step, : w * 2 : 2 * step].astype(np.float32)


@lru_cache(maxsize=1)
def standard_huffman_tables() -> bytes:
    """The JPEG standard's Huffman tables, as a DHT segment: many cameras leave them out of each MJPEG picture
    (they're the same every time). Taken from a JPEG Pillow writes, which uses them."""
    buf = io.BytesIO()
    Image.new("RGB", (16, 16)).save(buf, "JPEG", quality=75)
    data, out, i = buf.getvalue(), bytearray(), 2
    while i + 4 <= len(data) and data[i] == 0xFF:
        marker, length = data[i + 1], struct.unpack_from(">H", data, i + 2)[0]
        if marker == 0xC4:
            out += data[i:i + 2 + length]
        if marker == 0xDA:
            break
        i += 2 + length
    return bytes(out)


def with_huffman_tables(jpeg: bytes) -> bytes:
    """The picture, with the standard Huffman tables put in before the scan when it has none."""
    sos = jpeg.find(b"\xff\xda")
    if sos < 0 or jpeg.find(b"\xff\xc4", 0, sos) >= 0:
        return jpeg
    return jpeg[:sos] + standard_huffman_tables() + jpeg[sos:]


def mjpeg_to_image(data: bytes, max_width: int = 960) -> Image.Image:
    img = Image.open(io.BytesIO(with_huffman_tables(data)))
    if img.width > max_width:
        # Decoded at a half or a quarter of its size directly (the JPEG decoder can): much quicker.
        img.draft("RGB", (max_width, max_width * img.height // img.width))
    return img.convert("RGB")

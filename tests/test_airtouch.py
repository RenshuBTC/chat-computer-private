"""AirTouch: the network engine, hand tracking, gestures, the camera's interface, the pointer, and what Chat Computer
does with the gestures.

Most tests need nothing else. A few use Google's two hand models and MediaPipe's test pictures from
$CHATCOMPUTER_TEST_MODELS/airtouch and .../airtouch-pictures (skipped without them). tests/data/airtouch_reference.json
holds MediaPipe's own results for those pictures (its HandLandmarker, full models), to check ours against.
"""
import asyncio
import io
import json
import math
import os
import shutil
import struct
import subprocess
import time
from pathlib import Path

import numpy as np
import pytest
from PIL import Image

from chatcomputer.airtouch import camera, gestures, hands, keyboard, nn, tflite
from chatcomputer.airtouch.gestures import Engine, pose_of

HERE = Path(__file__).resolve().parent
REFERENCE = json.loads((HERE / "data" / "airtouch_reference.json").read_text())["pictures"]
MODELS = Path(os.environ.get("CHATCOMPUTER_TEST_MODELS",
                             str(Path.home() / ".cache" / "chatcomputer-test" / "models")))
PALM, LANDMARKS = MODELS / "airtouch" / "palm_detection_lite.tflite", MODELS / "airtouch" / "hand_landmark_lite.tflite"
PICTURES = MODELS / "airtouch-pictures"
need_models = pytest.mark.skipif(not (PALM.exists() and LANDMARKS.exists() and PICTURES.is_dir()),
                                 reason="the hand models and pictures aren't here")


# ================================================================== the network engine, one operator at a time
def _net(op: str, inputs: list[np.ndarray | None], shapes: list[tuple], options: dict, out_shape: tuple) -> nn.Net:
    """A one-operator network: tensor 0 is the input; the others are constants."""
    tensors = [tflite.Tensor("in", shapes[0], np.float32, None)]
    for i, (arr, shape) in enumerate(zip(inputs[1:], shapes[1:]), 1):
        tensors.append(tflite.Tensor(f"c{i}", shape, np.float32, arr))
    tensors.append(tflite.Tensor("out", out_shape, np.float32, None))
    ops = [tflite.Operator(op, list(range(len(inputs))), [len(tensors) - 1], options)]
    return nn.Net(tflite.Model(tensors, ops, [0], [len(tensors) - 1]))


def _same(size, k, s):
    out = -(-size // s)
    total = max((out - 1) * s + k - size, 0)
    return total // 2, total - total // 2


def _naive_conv(x, w, b, s, depthwise=False):
    """TFLite SAME convolution, written out (x: H, W, C)."""
    k = w.shape[1]
    (t, bo), (left, r) = _same(x.shape[0], k, s), _same(x.shape[1], k, s)
    xp = np.pad(x, ((t, bo), (left, r), (0, 0)))
    oh, ow = -(-x.shape[0] // s), -(-x.shape[1] // s)
    out = np.zeros((oh, ow, w.shape[3] if depthwise else w.shape[0]), np.float64)
    for y in range(oh):
        for xx in range(ow):
            patch = xp[y * s:y * s + k, xx * s:xx * s + k]
            if depthwise:
                out[y, xx] = (patch * w[0]).sum(axis=(0, 1)) + b
            else:
                out[y, xx] = np.tensordot(w, patch, axes=([1, 2, 3], [0, 1, 2])) + b
    return out


@pytest.mark.parametrize("stride,k", [(1, 3), (2, 3), (2, 5), (1, 1)])
def test_convolution_is_tflites(stride, k):
    rng = np.random.default_rng(stride * 10 + k)
    x = rng.standard_normal((1, 11, 9, 3)).astype(np.float32)
    w = rng.standard_normal((4, k, k, 3)).astype(np.float32)
    b = rng.standard_normal(4).astype(np.float32)
    want = _naive_conv(x[0], w, b, stride)
    net = _net("CONV_2D", [x, w, b], [x.shape, w.shape, b.shape],
               {"padding": "SAME", "stride_w": stride, "stride_h": stride, "activation": None}, (1,) + want.shape)
    assert np.allclose(net(x)[0][0], want, atol=1e-4)


@pytest.mark.parametrize("stride,k", [(1, 3), (2, 3), (1, 5), (2, 5)])
def test_depthwise_convolution_is_tflites(stride, k):
    rng = np.random.default_rng(stride * 10 + k)
    x = rng.standard_normal((1, 12, 10, 6)).astype(np.float32)
    w = rng.standard_normal((1, k, k, 6)).astype(np.float32)
    b = rng.standard_normal(6).astype(np.float32)
    want = np.clip(_naive_conv(x[0], w, b, stride, depthwise=True), 0, 6)
    net = _net("DEPTHWISE_CONV_2D", [x, w, b], [x.shape, w.shape, b.shape],
               {"padding": "SAME", "stride_w": stride, "stride_h": stride, "depth_multiplier": 1, "activation": "RELU6"},
               (1,) + want.shape)
    assert np.allclose(net(x)[0][0], want, atol=1e-4)


@pytest.mark.parametrize("align,half", [(False, False), (False, True), (True, False)])
def test_bilinear_resize_samples_where_tflite_does(align, half):
    rng = np.random.default_rng(1)
    x = rng.standard_normal((1, 5, 7, 2)).astype(np.float32)
    size = np.array([9, 13], dtype=np.int32)
    net = _net("RESIZE_BILINEAR", [x, size], [x.shape, (2,)], {"align_corners": align, "half_pixel_centers": half},
               (1, 9, 13, 2))
    got = net(x)[0][0]

    def coords(out, inp):
        scale = (inp - 1) / (out - 1) if align else inp / out
        return [((o + 0.5) * scale - 0.5) if half else o * scale for o in range(out)]
    want = np.zeros((9, 13, 2))
    for oy, fy in enumerate(coords(9, 5)):
        for ox, fx in enumerate(coords(13, 7)):
            y0, x0 = max(math.floor(fy), 0), max(math.floor(fx), 0)
            y1, x1 = min(math.ceil(fy), 4), min(math.ceil(fx), 6)
            dy, dx = fy - y0, fx - x0
            v = x[0]
            want[oy, ox] = (v[y0, x0] * (1 - dy) * (1 - dx) + v[y1, x0] * dy * (1 - dx) + v[y0, x1] * (1 - dy) * dx
                            + v[y1, x1] * dy * dx)
    assert np.allclose(got, want, atol=1e-5)


def test_pooling_prelu_padding_and_the_rest():
    rng = np.random.default_rng(2)
    x = rng.standard_normal((1, 6, 6, 3)).astype(np.float32)
    pool = _net("MAX_POOL_2D", [x], [x.shape], {"padding": "SAME", "stride_w": 2, "stride_h": 2, "filter_w": 2,
                                                 "filter_h": 2, "activation": None}, (1, 3, 3, 3))
    assert np.array_equal(pool(x)[0][0], x[0].reshape(3, 2, 3, 2, 3).max(axis=(1, 3)))
    alpha = rng.standard_normal((1, 1, 3)).astype(np.float32)
    prelu = _net("PRELU", [x, alpha], [x.shape, alpha.shape], {}, x.shape)
    assert np.allclose(prelu(x)[0], np.where(x >= 0, x, x * alpha))
    pads = np.array([[0, 0], [0, 0], [0, 0], [0, 2]], dtype=np.int32)
    pad = _net("PAD", [x, pads], [x.shape, (4, 2)], {}, (1, 6, 6, 5))
    assert pad(x)[0].shape == (1, 6, 6, 5) and np.array_equal(pad(x)[0][..., :3], x)
    axes = np.array([1, 2], dtype=np.int32)
    mean = _net("MEAN", [x, axes], [x.shape, (2,)], {"keep_dims": False}, (1, 3))
    assert np.allclose(mean(x)[0], x.mean(axis=(1, 2)))
    w, b = rng.standard_normal((4, 108)).astype(np.float32), rng.standard_normal(4).astype(np.float32)
    fc = _net("FULLY_CONNECTED", [x, w, b], [x.shape, w.shape, b.shape], {"activation": None}, (1, 4))
    assert np.allclose(fc(x)[0], x.reshape(1, -1) @ w.T + b, atol=1e-4)


@need_models
def test_the_model_files_are_read():
    palm = tflite.load(PALM.read_bytes())
    lm = tflite.load(LANDMARKS.read_bytes())
    assert palm.tensors[palm.inputs[0]].shape == (1, 192, 192, 3)
    assert [palm.tensors[i].shape for i in palm.outputs] == [(1, 2016, 18), (1, 2016, 1)]
    assert lm.tensors[lm.inputs[0]].shape == (1, 224, 224, 3)
    assert [lm.tensors[i].shape for i in lm.outputs] == [(1, 63), (1, 1), (1, 1), (1, 63)]
    with pytest.raises(ValueError):
        tflite.load(b"\0" * 64)


# ================================================================== hand tracking
def test_the_anchors_are_mediapipes():
    a = hands.ssd_anchors()
    assert a.shape == (2016, 2)
    assert np.allclose(a[0], [0.5 / 24, 0.5 / 24]) and np.allclose(a[1], a[0])
    assert np.allclose(a[1152], [0.5 / 12, 0.5 / 12]) and np.allclose(a[1157], a[1152])
    assert np.allclose(a[-1], [11.5 / 12, 11.5 / 12])


def _warp_reference(img: np.ndarray, roi: hands.Roi, size: int, replicate: bool) -> np.ndarray:
    """OpenCV's bilinear warp of the crop, written out (pixel centres at whole numbers)."""
    h, w = img.shape[:2]
    out = np.zeros((size, size, 3))
    cos, sin = math.cos(roi.rotation), math.sin(roi.rotation)
    for v in range(size):
        for u in range(size):
            a, b = (u / size - 0.5) * roi.side, (v / size - 0.5) * roi.side
            x, y = roi.cx + a * cos - b * sin, roi.cy + a * sin + b * cos
            x0, y0 = math.floor(x), math.floor(y)
            acc = np.zeros(3)
            for yy, wy in ((y0, 1 - (y - y0)), (y0 + 1, y - y0)):
                for xx, wx in ((x0, 1 - (x - x0)), (x0 + 1, x - x0)):
                    if replicate:
                        px = img[min(max(yy, 0), h - 1), min(max(xx, 0), w - 1)]
                    else:
                        px = img[yy, xx] if 0 <= yy < h and 0 <= xx < w else np.zeros(3)
                    acc += px * wy * wx
            out[v, u] = acc
    return out / 255


@pytest.mark.parametrize("roi,replicate", [(hands.Roi(30.5, 22.0, 20.0, 0.0), False),
                                           (hands.Roi(25.0, 20.0, 18.0, 0.7), True),
                                           (hands.Roi(5.0, 4.0, 30.0, -2.2), True)])
def test_the_crop_is_opencvs_warp(roi, replicate):
    rng = np.random.default_rng(3)
    img = rng.integers(0, 256, (40, 60, 3), dtype=np.uint8)
    got = hands.crop(Image.fromarray(img), roi, 16, edge="replicate" if replicate else "zero")
    want = _warp_reference(img.astype(np.float64), roi, 16, replicate)
    # Pillow computes in 8 bits: one step of 255, and at most two on the picture's edge.
    assert np.max(np.abs(got - want)) <= 2.5 / 255


def test_weighted_suppression_merges_overlapping_palms():
    cx = np.array([0.50, 0.52, 0.20], dtype=np.float32)
    cy = np.array([0.50, 0.50, 0.30], dtype=np.float32)
    wh = np.array([0.20, 0.20, 0.10], dtype=np.float32)
    scores = np.array([0.9, 0.6, 0.8], dtype=np.float32)
    kps = np.zeros((3, 7, 2), dtype=np.float32)
    kps[:, :, 0] = cx[:, None]
    out = list(hands._weighted_nms(cx, cy, wh, wh, scores, kps))
    assert [round(s, 2) for s, _, _ in out] == [0.9, 0.8]
    (x0, y0, x1, y1), kp = out[0][1], out[0][2]
    merged = (0.50 * 0.9 + 0.52 * 0.6) / 1.5
    assert math.isclose((x0 + x1) / 2, merged, abs_tol=1e-5) and math.isclose(kp[0, 0], merged, abs_tol=1e-5)
    assert math.isclose((y0 + y1) / 2, 0.5, abs_tol=1e-5)


def test_the_next_crop_turns_with_the_hand():
    joints = np.array(REFERENCE["pointing_up.jpg"]["joints"], dtype=np.float32)
    tracker = hands.HandTracker.__new__(hands.HandTracker)
    upright = tracker.next_roi(joints, (358, 376))
    assert abs(upright.rotation) < 0.5                      # the hand points up: nearly no turn
    turned = joints.copy()
    turned[:, 0], turned[:, 1] = 300 - joints[:, 1], joints[:, 0]        # the same hand, turned a quarter
    roi = tracker.next_roi(turned, (600, 600))
    assert abs(abs(hands.normalize_radians(roi.rotation - upright.rotation)) - math.pi / 2) < 0.1
    assert math.isclose(roi.side, upright.side, rel_tol=0.02)


@need_models
def test_hands_are_found_where_mediapipe_finds_them():
    tracker = hands.HandTracker(PALM.read_bytes(), LANDMARKS.read_bytes())
    for name, ref in REFERENCE.items():
        img = Image.open(PICTURES / name).convert("RGB")
        want = np.array(ref["joints"], dtype=np.float32)
        size = np.linalg.norm(want[9, :2] - want[0, :2])
        # Its palm is among the palms found (several pictures have two hands; MediaPipe answered with one).
        centre = want[list(gestures.PALM), :2].mean(axis=0)
        palms = tracker.find_palms(img)
        assert any(abs(p.box[0] - centre[0]) < p.box[2] / 2 and abs(p.box[1] - centre[1]) < p.box[3] / 2
                   for p in palms), name
        # Its joints, from a crop around it: the lite models against MediaPipe's full ones, within a tenth of
        # the hand's size on average.
        hand = tracker.landmarks(img, tracker.next_roi(want, img.size))
        err = np.linalg.norm(hand.joints[:, :2] - want[:, :2], axis=1)
        assert err.mean() < 0.1 * size and hand.presence > 0.8, (name, err.mean(), size)


@pytest.mark.skipif(not (MODELS / "airtouch" / "hand_landmarker.task").exists() or not PICTURES.is_dir(),
                    reason="MediaPipe's full models aren't here")
def test_with_mediapipes_own_models_the_answer_is_mediapipes():
    import zipfile
    z = zipfile.ZipFile(MODELS / "airtouch" / "hand_landmarker.task")
    # Its task graph turns the palm's crop by "90" in radians (its own older graph says 90 degrees).
    tracker = hands.HandTracker(z.read("hand_detector.tflite"), z.read("hand_landmarks_detector.tflite"), target_angle=90.0)
    for name in ("pointing_up.jpg", "fist.jpg", "thumb_up.jpg", "right_hands_rotated.jpg"):
        tracker.reset()
        hand = tracker.process(Image.open(PICTURES / name).convert("RGB"))
        err = np.linalg.norm(hand.joints[:, :2] - np.array(REFERENCE[name]["joints"])[:, :2], axis=1)
        assert err.max() < 2.0, (name, err.max())


@need_models
def test_a_hand_is_followed_without_looking_for_it_again():
    tracker = hands.HandTracker(PALM.read_bytes(), LANDMARKS.read_bytes())
    img = Image.open(PICTURES / "pointing_up.jpg").convert("RGB")
    looks = []
    real = tracker.find_palms
    tracker.find_palms = lambda im: looks.append(1) or real(im)
    for _ in range(3):
        assert tracker.process(img) is not None
    assert len(looks) == 1                      # palm detection once; then the joints give the next crop
    blank = Image.new("RGB", img.size, (128, 128, 128))
    assert tracker.process(blank) is None and tracker.roi is None


@need_models
def test_two_hands_are_followed_each_with_its_own_number():
    tracker = hands.HandTracker(PALM.read_bytes(), LANDMARKS.read_bytes())
    img = Image.open(PICTURES / "right_hands.jpg").convert("RGB")
    first = tracker.track(img, search=True)
    assert len(first) == 2 and first[0][0] != first[1][0]
    assert all(h.right > 0.9 for _, h in first)                      # both right hands, as MediaPipe says
    looks = []
    real = tracker.find_palms
    tracker.find_palms = lambda im: looks.append(1) or real(im)
    again = tracker.track(img, search=True)                         # two followed: no looking for more
    assert [i for i, _ in again] == [i for i, _ in first] and looks == []
    one = tracker.track(img.crop((0, 0, 360, 382)).resize((720, 382)), search=False)
    assert len(one) <= 2
    tracker.reset()
    woman = tracker.track(Image.open(PICTURES / "hand-woman-man.jpg").convert("RGB"), search=True)
    ref = np.array(REFERENCE["hand-woman-man.jpg"]["joints"])[:, :2]
    best = min(woman, key=lambda ih: np.linalg.norm(ih[1].joints[:, :2] - ref, axis=1).mean())[1]
    assert best.right < 0.2                                         # MediaPipe: "Left"



# ================================================================== gestures
def _hand(name: str, at=(320.0, 240.0), size=100.0, turn=0.0) -> np.ndarray:
    """A real hand shape (MediaPipe's joints for that picture) made `size` pixels big (wrist to middle
    knuckle) with its palm's centre at `at`, turned by `turn` radians."""
    j = np.array(REFERENCE[name]["joints"], dtype=np.float64)
    centre = j[list(gestures.PALM), :2].mean(axis=0)
    k = size / np.linalg.norm(j[9, :2] - j[0, :2])
    out = j.copy()
    d = (j[:, :2] - centre) * k
    c, s = math.cos(turn), math.sin(turn)
    out[:, 0] = at[0] + d[:, 0] * c - d[:, 1] * s
    out[:, 1] = at[1] + d[:, 0] * s + d[:, 1] * c
    out[:, 2] = j[:, 2] * k
    return out.astype(np.float32)


def _right(name: str) -> float:
    return 1.0 if REFERENCE[name]["handedness"] == "Right" else 0.0


# A hand built from a simple skeleton, in hand sizes: a right hand with its palm to the camera and fingers up
# (thumb on the camera's right), each finger bent towards the palm by `curl` degrees in all (45% at the
# knuckle), then turned by `rot`. MediaPipe's pictures don't have the poses the newer gestures need (a palm
# turned up, typing hands), so these are made.
_BASES = {"index": (0.32, -0.92), "middle": (0.06, -1.0), "ring": (-0.18, -0.95), "little": (-0.40, -0.82)}
_DIRS = {"index": (0.12, -1.0), "middle": (0.0, -1.0), "ring": (-0.1, -1.0), "little": (-0.22, -1.0)}
_LENGTHS = {"index": (0.42, 0.24, 0.2), "middle": (0.47, 0.28, 0.2), "ring": (0.44, 0.27, 0.2),
            "little": (0.34, 0.2, 0.18)}
_THUMBS = {"out": [(0.25, -0.18, -0.05), (0.5, -0.32, -0.08), (0.68, -0.44, -0.1), (0.85, -0.55, -0.1)],
           "in": [(0.2, -0.2, -0.08), (0.3, -0.4, -0.15), (0.25, -0.55, -0.22), (0.12, -0.62, -0.25)]}


def _rot_x(deg):
    a = math.radians(deg)
    return np.array([[1, 0, 0], [0, math.cos(a), -math.sin(a)], [0, math.sin(a), math.cos(a)]])


def _rot_y(deg):
    a = math.radians(deg)
    return np.array([[math.cos(a), 0, math.sin(a)], [0, 1, 0], [-math.sin(a), 0, math.cos(a)]])


FACING = np.eye(3)
BACK = _rot_y(180)
PALM_UP = _rot_x(120) @ _rot_y(180)        # held out palm up, seen by a camera a little above it
TYPING = _rot_x(120)                       # palm down, fingers towards the screen: typing in the air
POINT = {"index": 0, "middle": 220, "ring": 230, "little": 230}
PINCH = {"index": 40, "middle": 200, "ring": 220, "little": 220}


def _model(right=True, curl=0.0, thumb="out", rot=FACING, at=(320.0, 240.0), size=80.0) -> np.ndarray:
    curls = curl if isinstance(curl, dict) else {n: curl for n in gestures.FINGERS}
    j = np.zeros((21, 3))
    palm_side = np.array([0.0, 0.0, -1.0])
    for name, (mcp, pip, dip, tip) in gestures.FINGERS.items():
        pos = np.array([*_BASES[name], 0.0])
        j[mcp] = pos
        d = np.array([*_DIRS[name], 0.0])
        d /= np.linalg.norm(d)
        for k, (length, share) in enumerate(zip(_LENGTHS[name], (0.45, 0.8, 1.0))):
            a = math.radians(curls.get(name, 0.0) * share)
            pos = pos + length * (math.cos(a) * d + math.sin(a) * palm_side)
            j[(pip, dip, tip)[k]] = pos
    if thumb == "pinch":                    # the thumb's tip on the index finger's
        base, end = np.array(_THUMBS["out"][0]), j[8] + np.array([0.04, 0.03, 0.0])
        j[1] = base
        for k, f in enumerate((0.33, 0.66, 1.0)):
            j[2 + k] = base + (end - base) * f + np.array([0.08, 0.0, -0.05]) * math.sin(math.pi * f)
    else:
        j[1:5] = _THUMBS[thumb]
    if not right:
        j[:, 0] *= -1                       # a left hand is a right hand's mirror image
    j = j @ rot.T * size
    j[:, :2] += np.array(at) - j[list(gestures.PALM), :2].mean(axis=0)
    return j.astype(np.float32)


def _one(joints, right=1.0) -> dict:
    return {1: (joints, right)}


def _feed(engine, frames, t0=0.0, fps=30.0):
    out, t = [], t0
    for hands_seen in frames:
        out += engine.update(t, hands_seen)
        t += 1 / fps
    return out, t


def _dots(out):
    return [a[1] for a in out if a[0] == "dots"]


@pytest.mark.parametrize("name,kind", [("fist.jpg", "fist"), ("thumb_up.jpg", "yes"), ("right_hands.jpg", "open"),
                                        ("left_hands_rotated.jpg", "open"), ("male_full_height_hands.jpg", "open"),
                                        ("man-woman-okay.jpg", "pinch"), ("pointing_up.jpg", "point"),
                                        ("victory.jpg", "point")])
def test_hand_shapes_are_told_apart(name, kind):
    p = pose_of(np.array(REFERENCE[name]["joints"], dtype=np.float32))
    got = ("open" if p.open else "yes" if p.thumb_up and p.thumb_only else "fist" if p.fist
           else "pinch" if p.pinch < gestures.PINCH_ON and p.reach > gestures.PINCH_REACH else "point")
    assert got == kind


def test_which_way_the_palm_faces_comes_from_which_hand_it_is():
    def pose(name):
        return pose_of(np.array(REFERENCE[name]["joints"], dtype=np.float32), _right(name))
    assert pose("pointing_up.jpg").facing and pose("victory.jpg").facing          # palms shown to the camera
    assert not pose("right_hands.jpg").facing and not pose("left_hands.jpg").facing   # the backs of hands
    up = pose("hand-woman-man.jpg")                                               # a hand held out, palm up
    assert up.palm_up and not up.facing
    assert pose_of(np.array(REFERENCE["pointing_up.jpg"]["joints"], dtype=np.float32)).normal is None
    for rot, facing, palm_up in ((FACING, True, False), (BACK, False, False), (PALM_UP, False, True),
                                 (TYPING, False, False)):
        for right in (True, False):
            p = pose_of(_model(right=right, rot=rot), 1.0 if right else 0.0)
            assert (p.facing, p.palm_up) == (facing, palm_up)


def test_a_dot_follows_the_index_finger_from_where_the_pointer_was():
    e = Engine(1920, 1080)
    e.cursor = [700.0, 400.0]
    frames = [_one(_model(curl=POINT, thumb="in", at=(320 + 3 * i, 240 + 1.5 * i))) for i in range(40)]
    out, _ = _feed(e, frames)
    assert ("hand", True) in out
    assert len(_dots(out)[0]) == 1 and _dots(out)[0][0][:2] == pytest.approx((700.0, 400.0))
    x, y, kind = _dots(out)[-1][0]
    # 117 pixels to the right in the camera's picture, with the hand 80 pixels big: 1.46 of the 5 hand sizes
    # the screen is across, to the left (you face the camera: your right is its left), and down.
    assert x == pytest.approx(700 - 117 / 80 / 5 * 1920, abs=40) and y > 400 + 150 and kind == "point"
    moves = [a for a in out if a[0] == "move"]
    assert len(moves) > 30 and moves[-1][1:] == pytest.approx((x, y))


def test_each_hand_has_its_own_dot_on_its_own_side():
    e = Engine(1920, 1080)
    both = {1: (_model(right=True, curl=POINT, thumb="in", at=(220, 260)), 1.0),
            2: (_model(right=False, curl=POINT, thumb="in", at=(420, 260)), 0.0)}
    out, _ = _feed(e, [both] * 10)
    (rx, ry, _), (lx, ly, _) = _dots(out)[-1]
    # Your left hand is on the camera's right: its dot is left of your right hand's, as far apart as the index
    # fingers are (in the 5 hand sizes across the screen).
    apart = float(both[2][0][8, 0] - both[1][0][8, 0])
    assert lx < rx and rx - lx == pytest.approx(apart / 80 / 5 * 1920, abs=10) and ry == pytest.approx(ly, abs=5)


def test_a_still_hand_keeps_its_dot_still():
    e = Engine(1920, 1080)
    rng = np.random.default_rng(4)
    base = _model(curl=POINT, thumb="in")
    frames = [_one(base + rng.normal(0, 0.8, (21, 3)).astype(np.float32)) for _ in range(60)]
    out, _ = _feed(e, frames)
    pts = np.array([d[0][:2] for d in _dots(out)[10:]])
    assert np.ptp(pts[:, 0]) < 15 and np.ptp(pts[:, 1]) < 15


def test_a_quick_pinch_taps_where_the_dot_was():
    e = Engine(1920, 1080)
    _, t = _feed(e, [_one(_model(curl=POINT, thumb="in"))] * 10)
    where = tuple(e.cursor)
    out, t = _feed(e, [_one(_model(curl=PINCH, thumb="pinch"))] * 8, t0=t)          # a quarter of a second
    assert not any(a[0] == "move" for a in out)                 # the dot holds still while the fingers meet
    (x, y, kind), = _dots(out)[-1]
    assert (x, y) == pytest.approx(where) and kind == "pinch"
    out, t = _feed(e, [_one(_model(curl=POINT, thumb="in"))] * 8, t0=t)
    clicks = [a for a in out if a[0] == "click"]
    assert len(clicks) == 1 and clicks[0][1:] == pytest.approx(where)


def test_a_fist_resting_on_the_finger_isnt_a_pinch():
    e = Engine(1920, 1080)
    out, t = _feed(e, [_hand("pointing_up.jpg")] * 5 + [_hand("fist.jpg")] * 10 + [_hand("pointing_up.jpg")] * 5)
    assert not any(a[0] == "click" for a in out)


def test_pinching_and_moving_scrolls_like_dragging_the_page():
    e = Engine(1920, 1080)
    _, t = _feed(e, [_one(_model(curl=POINT, thumb="in"))] * 5)
    out, t = _feed(e, [_one(_model(curl=PINCH, thumb="pinch", at=(320, 240 - 4 * i))) for i in range(25)], t0=t)
    out2, t = _feed(e, [_one(_model(curl=POINT, thumb="in", at=(320, 140)))] * 5, t0=t)
    dy = sum(a[2] for a in out if a[0] == "scroll")
    assert dy >= 10                                   # the hand went up: the page goes up, it scrolls down
    assert not any(a[0] == "click" for a in out + out2)
    assert any(d and d[0][2] == "scroll" for d in _dots(out))


def test_an_open_palm_held_up_stops_once_and_the_back_of_a_hand_doesnt():
    e = Engine(1920, 1080)
    out, t = _feed(e, [_one(_model())] * 40)                       # 1.3 s
    assert [a for a in out if a[0] == "gesture"] == [("gesture", "palm")]
    out, t = _feed(e, [_one(_model(curl=POINT, thumb="in"))] * 15 + [_one(_model())] * 25, t0=t)
    assert [a for a in out if a[0] == "gesture"] == [("gesture", "palm")]        # again, once lowered and raised
    out, _ = _feed(Engine(1920, 1080), [_one(_model(rot=BACK))] * 40)
    assert not any(a[0] == "gesture" for a in out)
    out, _ = _feed(Engine(1920, 1080), [_one(_hand("right_hands.jpg"), 1.0)] * 40)  # MediaPipe's backs of hands
    assert not any(a[0] == "gesture" for a in out)


def test_a_waving_open_hand_doesnt_stop_anything():
    e = Engine(1920, 1080)
    out, _ = _feed(e, [_one(_model(at=(320 + 120 * math.sin(i / 3), 240))) for i in range(45)])
    assert not any(a[0] == "gesture" for a in out)


def test_thumbs_up_and_down_answer():
    e = Engine(1920, 1080)
    out, t = _feed(e, [_hand("thumb_up.jpg")] * 25)
    assert [a for a in out if a[0] == "gesture"] == [("gesture", "yes")]
    e2 = Engine(1920, 1080)
    out, _ = _feed(e2, [_hand("thumb_up.jpg", turn=math.pi)] * 25)
    assert [a for a in out if a[0] == "gesture"] == [("gesture", "no")]


def test_hands_that_leave_take_their_dots():
    e = Engine(1920, 1080)
    _, t = _feed(e, [_one(_model(curl=POINT, thumb="in"))] * 5)
    out, t = _feed(e, [{}] * 10, t0=t)                 # a third of a second: maybe a bad picture
    assert not any(a[0] == "hand" for a in out) and _dots(out)[-1] != []
    out, t = _feed(e, [{}] * 15, t0=t)
    assert ("hand", False) in out and ("dots", []) in out


def test_a_hand_lost_for_a_picture_keeps_its_dot():
    e = Engine(1920, 1080)
    hand = _model(curl=POINT, thumb="in")
    out, t = _feed(e, [_one(hand)] * 10)
    before = _dots(out)[-1][0]
    out, t = _feed(e, [{}] * 2 + [{7: (hand, 1.0)}] * 3, t0=t)      # found again, with a new number
    assert not any(a[0] == "hand" for a in out)
    assert list(e.hands) == [7] and _dots(out)[-1][0][:2] == pytest.approx(before[:2], abs=3)


def test_the_dots_stay_on_the_screen_and_the_space_follows_the_hand():
    e = Engine(800, 600)
    out, t = _feed(e, [_one(_model(curl=POINT, thumb="in", at=(320 - 15 * i, 240), size=60)) for i in range(30)])
    x, y, _ = _dots(out)[-1][0]
    assert x == 799 and 0 <= y <= 599                  # the hand went far to your right: the dot is at the edge
    back = [_one(_model(curl=POINT, thumb="in", at=(320 - 15 * 29 + 12 * i, 240), size=60)) for i in range(1, 11)]
    out, _ = _feed(e, back, t0=t)
    assert _dots(out)[-1][0][0] < 799 - 200            # and comes away from it as soon as the hand comes back


def _beckon(rot, right=1.0):
    return [_one(_model(rot=rot, curl=c), right) for c in (40, 90, 150, 200, 220, 220, 150, 80, 20)]


def test_beckoning_up_brings_the_keyboard_and_down_takes_it_away():
    e = Engine(1920, 1080)
    up, down = _one(_model(rot=PALM_UP)), _one(_model(rot=TYPING))
    out, t = _feed(e, [up] * 10 + _beckon(PALM_UP) + [up] * 10)
    assert [a for a in out if a[0] == "gesture"] == [("gesture", "keyboard up")]
    assert not any(a[0] == "click" for a in out)
    out, t = _feed(e, [down] * 10 + _beckon(TYPING) + [down] * 10, t0=t)        # turned over: palm down
    assert [a for a in out if a[0] == "gesture"] == [("gesture", "keyboard down")]
    out, t = _feed(e, _beckon(TYPING) + [down] * 5, t0=t)                        # once only, until opened again
    assert not any(a[0] == "gesture" for a in out)
    out, t = _feed(e, [down] * 40 + _beckon(TYPING), t0=t)
    assert [a for a in out if a[0] == "gesture"] == [("gesture", "keyboard down")]


def test_closing_a_hand_isnt_a_beckon_unless_its_palm_is_up_or_down_and_its_quick():
    curl = (40, 90, 150, 220, 220, 100, 0)
    for frames in ([_one(_model())] * 10 + [_one(_model(curl=c)) for c in curl],                 # palm to the screen
                   [_one(_model(rot=PALM_UP), None)] * 10 + [_one(_model(rot=PALM_UP, curl=c), None) for c in curl],
                   [_one(_model(rot=PALM_UP, curl=c * 220 / 90)) for c in range(91)]):        # slowly, over 3 s
        out, _ = _feed(Engine(1920, 1080), frames)
        assert not any(a[0] == "gesture" and a[1].startswith("keyboard") for a in out)


# ------------------------------------------------------------------ typing
def _keyboard():
    kb = keyboard.Keyboard()
    kb.layout(1920, 432)
    x, y, w, h = kb.area()
    return kb, (x, 648 + y, w, h, keyboard.UNITS, keyboard.ROWS)


def _typing(tap=None, depth=40, dx=0.0, dy=0.0, spread=0.0):
    """Two hands typing in the air, the right one on the camera's left; `tap` = (hand, finger) bent down."""
    bent = {"R": {n: 15 for n in gestures.FINGERS}, "L": {n: 15 for n in gestures.FINGERS}}
    if tap:
        bent[tap[0]][tap[1]] = 15 + depth
    return {1: (_model(right=True, curl=bent["R"], rot=TYPING, at=(270 + dx - spread, 300 + dy), size=70), 1.0),
            2: (_model(right=False, curl=bent["L"], rot=TYPING, at=(370 + dx + spread, 300 + dy), size=70), 0.0)}


def _tap(hand, finger, **kw):
    return [_typing((hand, finger), d, **kw) for d in (15, 30, 40, 40, 25, 10, 0)] + [_typing(**kw)] * 6


def _under(kb, dots):
    return [(kb.hit(x, y - 648).label if kb.hit(x, y - 648) else None) for x, y, _k in dots]


def _typed(kb, out):
    return [kb.hit(a[1], a[2] - 648).label for a in out if a[0] == "key"]


@pytest.mark.parametrize("apart", [-10, 0, 15])
def test_with_the_keyboard_up_each_index_finger_has_its_dot_where_hands_rest(apart):
    kb, spec = _keyboard()
    e = Engine(1920, 1080)
    e.set_keyboard(spec)
    out, _ = _feed(e, [_typing(spread=apart)] * 15)
    dots = _dots(out)[-1]
    assert _under(kb, dots) == ["j", "f"] and {k for _x, _y, k in dots} == {"key"}      # two dots, on j and f...
    j, f = (next(k for k in kb.keys if k.label == c) for c in "jf")
    assert dots[0][0] == pytest.approx(j.x + j.w / 2, abs=3) and dots[1][0] == pytest.approx(f.x + f.w / 2, abs=3)
    assert dots[0][1] == pytest.approx(648 + j.y + j.h / 2, abs=3)        # ...right in their middles, however
                                                                         # far apart the hands are held


@pytest.mark.parametrize("hand,finger,key", [("R", "index", "j"), ("R", "middle", "k"), ("R", "ring", "l"),
                                             ("R", "little", "return"), ("L", "index", "f"), ("L", "middle", "d"),
                                             ("L", "ring", "s"), ("L", "little", "a")])
def test_every_finger_types_its_key_the_index_fingers_with_dots(hand, finger, key):
    kb, spec = _keyboard()
    e = Engine(1920, 1080)
    e.set_keyboard(spec)
    _, t = _feed(e, [_typing()] * 15)
    out, _ = _feed(e, _tap(hand, finger), t0=t)
    assert _typed(kb, out) == [key]                   # the fingers without dots type the keys beside their index's
    pressed = any("press" in [k for _x, _y, k in d] for d in _dots(out))
    assert pressed == (finger == "index")


def test_moving_the_hands_moves_their_keys_and_types_nothing():
    kb, spec = _keyboard()
    e = Engine(1920, 1080)
    e.set_keyboard(spec)
    _, t = _feed(e, [_typing()] * 15)
    out, t = _feed(e, [_typing(dy=-d) for d in range(0, 16, 2)] + [_typing(dy=-15)] * 10, t0=t)   # up a row
    assert not any(a[0] == "key" for a in out)
    assert _under(kb, _dots(out)[-1]) == ["u", "r"]
    typed = []
    for hand, finger in (("R", "index"), ("R", "little"), ("L", "little")):
        out, t = _feed(e, _tap(hand, finger, dy=-15), t0=t)
        typed += _typed(kb, out)
    assert typed == ["u", "p", "q"]


def test_a_finger_held_down_types_once_and_pinching_doesnt_click():
    kb, spec = _keyboard()
    e = Engine(1920, 1080)
    e.set_keyboard(spec)
    _, t = _feed(e, [_typing()] * 15)
    out, t = _feed(e, [_typing(("R", "index"), d) for d in (15, 30, 40)] + [_typing(("R", "index"), 40)] * 45, t0=t)
    out2, t = _feed(e, [_typing()] * 10 + _tap("R", "index"), t0=t)
    assert len([a for a in out if a[0] == "key"]) == 1 and len([a for a in out2 if a[0] == "key"]) == 1
    out, _ = _feed(e, [{1: (_model(curl=PINCH, thumb="pinch", rot=TYPING, at=(270, 300), size=70), 1.0)}] * 15, t0=t)
    assert not any(a[0] in ("click", "move", "scroll") for a in out)


def test_putting_the_keyboard_away_brings_back_a_dot_per_hand():
    kb, spec = _keyboard()
    e = Engine(1920, 1080)
    e.set_keyboard(spec)
    _, t = _feed(e, [_typing()] * 10)
    e.set_keyboard(None)
    out, _ = _feed(e, [_typing()] * 5, t0=t)
    assert [k for _x, _y, k in _dots(out)[-1]] == ["point", "point"]


def test_one_euro_filter_smooths_still_and_follows_moving():
    f = gestures.OneEuro(min_cutoff=1.2, beta=0.9)
    rng = np.random.default_rng(5)
    still = [f(np.array([1.0 + rng.normal(0, 0.01)]), i / 30)[0] for i in range(60)]
    assert np.std(still[20:]) < 0.004
    f.reset()
    moving = [f(np.array([i / 30 * 2.0]), i / 30)[0] for i in range(60)]
    assert abs(moving[-1] - 59 / 30 * 2.0) < 0.25            # lags a little behind a quick move


# ================================================================== the keyboard
def test_the_keys_are_where_the_ipads_are():
    """At the size of Apple's pictures each key is exactly where it was measured there, every page alike."""
    kb = keyboard.Keyboard()
    for page in keyboard.PAGES:
        kb.page = page
        mw = keyboard.MARGIN[0] + keyboard.REF_W + keyboard.MARGIN[2]
        mh = keyboard.MARGIN[1] + keyboard.REF_H + keyboard.MARGIN[3]
        keys = kb.layout(mw, mh)
        assert kb.scale == pytest.approx(1.0) and kb.origin == pytest.approx(keyboard.MARGIN[:2])
        assert [len(r) for r in keyboard.PAGES[page]] == [len(r) for r in keyboard.KEY_EDGES]
        i = 0
        for (top, bottom), edges in zip(keyboard.ROW_EDGES, keyboard.KEY_EDGES):
            for left, right in edges:
                k = keys[i]
                assert (k.x - kb.origin[0], k.x + k.w - kb.origin[0]) == pytest.approx((left, right))
                assert (k.y - kb.origin[1], k.y + k.h - kb.origin[1]) == pytest.approx((top, bottom))
                i += 1
    # The iPad's proportions: a letter key 40.2 wide in a 47.5 step; Tab and Delete 52.3; Caps Lock 66.4;
    # Return 85.7; Shift 88.1 and 64.5; the space bar 292.3; rows 40 high, 46.7 apart.
    by = {k.label: k for k in keyboard.Keyboard().layout(587.1 + 11.2, 180.12 + 25.0)}
    assert by["q"].w == pytest.approx(40.28, abs=0.01) and by["w"].x - by["q"].x == pytest.approx(47.7, abs=0.1)
    assert by["space"].w == pytest.approx(292.31, abs=0.01) and by["return"].w == pytest.approx(85.67, abs=0.01)


def test_the_keyboard_keeps_the_ipads_shape_on_any_screen():
    kb = keyboard.Keyboard()
    for w, h in ((1920, 432), (2560, 605), (800, 600)):
        kb.layout(w, h)
        x, y, aw, ah = kb.area()
        assert aw / ah == pytest.approx(keyboard.REF_W / keyboard.REF_H)
        assert x >= 0 and x + aw <= w and y >= 0 and y + ah <= h
        assert abs((x + aw / 2) - w / 2) < 1                  # in the middle
    assert keyboard.Keyboard.size_for(2560, 605)[1] == pytest.approx(605)


def test_keys_are_found_where_they_are_drawn():
    kb = keyboard.Keyboard()
    kb.layout(1150, 400)
    for k in kb.keys:
        assert kb.hit(k.x + k.w / 2, k.y + k.h / 2) is k
    ax, ay, aw, ah = kb.area()
    assert kb.hit(ax + 10, ay - 30) is None and kb.hit(ax + 10, ay + ah + 30) is None
    q, w = (next(k for k in kb.keys if k.label == c) for c in "qw")
    gap = (q.x + q.w + w.x) / 2
    assert kb.hit(gap - 1, q.y + 5) is q and kb.hit(gap + 1, q.y + 5) is w     # a gap is split between its keys


def test_shift_caps_lock_and_the_double_space():
    kb = keyboard.Keyboard()
    kb.layout(1150, 400)
    key = {k.label: k for k in kb.keys}
    shift = next(k for k in kb.keys if k.action == ("shift",))
    assert kb.press(shift, 0.0) is None and kb.shift == "once"
    kb.layout(1150, 400)
    assert "A" in [k.label for k in kb.keys]                 # the letters show as capitals
    assert kb.press(key["a"], 1.0) == ("text", "A") and kb.press(key["b"], 1.5) == ("text", "b")
    kb.press(shift, 10.0)
    kb.press(shift, 10.2)                                    # a double tap on Shift: Caps Lock
    assert kb.shift == "lock"
    assert [kb.press(key[c], 11.0) for c in "ok"] == [("text", "O"), ("text", "K")]
    kb.press(shift, 12.0)
    assert kb.shift == "off" and kb.press(key["a"], 13.0) == ("text", "a")
    caps = next(k for k in kb.keys if k.action == ("caps",))
    kb.press(caps, 14.0)
    assert kb.shift == "lock" and kb.press(key["z"], 14.5) == ("text", "Z")
    kb.press(caps, 15.0)
    space = key["space"]
    assert [kb.press(k, t) for k, t in ((key["a"], 16.0), (space, 16.2), (space, 16.4))] == \
        [("text", "a"), ("text", " "), ("replace", ". ")]      # a double tap on Space: a full stop and a space
    assert [kb.press(space, t) for t in (20.0, 20.2)] == [("text", " "), ("text", " ")]   # not after a space


def test_the_other_pages_and_the_keys_that_do_things():
    kb = keyboard.Keyboard()
    kb.layout(1150, 400)
    by = {k.label: k for k in kb.keys}
    for label, action in (("delete", ("key", "BackSpace")), ("return", ("key", "Return")), ("tab", ("key", "Tab")),
                          ("hide keyboard", ("hide",)), ("dictate", ("dictate",))):
        assert kb.press(by[label], 0) == action
    q = (by["q"].x + 5, by["q"].y + 5)
    assert kb.press(by[".?123"], 0) is None and kb.page == "numbers"
    kb.layout(1150, 400)
    assert kb.press(kb.hit(*q), 0) == ("text", "1")         # the letter keys' second characters, in their places
    by = {k.label: k for k in kb.keys}
    assert [kb.press(by[c], 0) for c in "@%"] == [("text", "@"), ("text", "%")]
    kb.press(next(k for k in kb.keys if k.label == "#+="), 0)
    kb.layout(1150, 400)
    assert kb.press(kb.hit(*q), 0) == ("text", "[")
    kb.press(next(k for k in kb.keys if k.label == "ABC"), 0)
    kb.layout(1150, 400)
    assert kb.page == "letters" and kb.press(kb.hit(*q), 0) == ("text", "q")
    kb.press(next(k for k in kb.keys if k.label == "emoji"), 0)
    kb.layout(1150, 400)
    assert kb.press(kb.hit(*q), 0) == ("text", "😀")
    assert kb.press(kb.keys[-6], 0) is None and kb.page == "letters"      # its corner key: back to the letters


# ------------------------------------------------------------------ drawing it
class _Area:
    """Stands in for the window's drawing area: remembers what was asked to be drawn again."""

    def __init__(self):
        self.rects, self.full = [], 0

    def queue_draw_area(self, x, y, w, h):
        self.rects.append((x, y, w, h))

    def queue_draw(self):
        self.full += 1


def _layer():
    pytest.importorskip("cairo")
    from chatcomputer.ui.airtouch_layer import AirLayer
    return AirLayer(_Area(), lambda: 24.0)


def _pixels(layer, w=1280, h=800):
    import cairo
    surf = cairo.ImageSurface(cairo.FORMAT_RGB24, w, h)
    layer.draw(None, cairo.Context(surf))
    surf.flush()
    return np.frombuffer(surf.get_data(), np.uint8).reshape(h, surf.get_stride() // 4, 4)[:, :w, :3]


def test_the_keyboard_is_drawn_black_on_white_along_the_bottom():
    layer = _layer()
    x, y, w, h = layer.show_keyboard(1280, 800)
    px, py, pw, ph = layer.panel
    assert py + ph == 800 and pw == 1280 and py < y < y + h <= 800
    assert ph <= 0.42 * 800 + 1 and w / h == pytest.approx(keyboard.REF_W / keyboard.REF_H)
    img = _pixels(layer)
    assert img[: int(py) - 1].max() == 0                      # nothing drawn above the keyboard
    panel = img[int(py) + 4:]
    assert (panel == 255).all(axis=2).mean() > 0.6 and (panel == 0).all(axis=2).mean() > 0.01
    for label in ("q", "tab", "space"):
        k = next(k for k in layer.kb.keys if k.label == label)
        inset = int(k.h * 0.25)                                 # inside the rounded corners
        face = img[int(py + k.y) + 3: int(py + k.y + k.h) - 3, int(k.x) + inset: int(k.x + k.w) - inset]
        assert (face == 255).all(axis=2).mean() > 0.6                                  # a white key...
        assert (face == 0).all(axis=2).any() == (label != "space")                    # ...with its marks on it


def test_dots_are_drawn_and_only_their_squares_redrawn():
    layer = _layer()
    layer.set_dots([(100.0, 100.0, "point"), (300.0, 120.0, "pinch")])
    img = _pixels(layer)
    rim = int(layer._radius("point")) + 1
    assert (img[100, 100] == 0).all() and (img[100, 100 + rim] == 255).all()       # black dot, white rim
    assert all(w <= 40 and h <= 40 for _x, _y, w, h in layer.area.rects)
    layer.area.rects = []
    layer.set_dots([(110.0, 100.0, "point")])
    xs = sorted(x for x, _y, _w, _h in layer.area.rects)
    assert xs[0] < 100 < xs[-1] + 40 and any(x > 250 for x in xs)                 # where they were, and are


@pytest.mark.parametrize("rem", [16.0, 24.0, 40.0])
def test_what_is_drawn_again_covers_every_kind_of_dot(rem):
    # A ring (pinching, scrolling) reaches further than a dot: what's drawn again when it moves must cover all of
    # it, or bits of the ring stay behind on the screen (and the new one shows cut off).
    cairo = pytest.importorskip("cairo")
    from chatcomputer.ui.airtouch_layer import AirLayer
    for kind in ("point", "key", "press", "pinch", "scroll"):
        layer = AirLayer(_Area(), lambda: rem)
        layer.set_dots([(200.4, 199.6, kind)])
        surf = cairo.ImageSurface(cairo.FORMAT_RGB24, 400, 400)
        cr = cairo.Context(surf)
        cr.set_source_rgb(0.5, 0.5, 0.5)
        cr.paint()
        layer.draw(None, cr)
        surf.flush()
        img = np.frombuffer(surf.get_data(), np.uint8).reshape(400, surf.get_stride() // 4, 4)[:, :400, :3].astype(int)
        ys, xs = np.nonzero(np.abs(img - 128).max(axis=2) > 1)
        (x, y, w, h), = layer.area.rects
        assert x <= xs.min() and xs.max() < x + w and y <= ys.min() and ys.max() < y + h, (kind, rem)


def test_pressing_a_key_says_what_to_type_and_lights_it():
    layer = _layer()
    layer.show_keyboard(1280, 800)
    px, py, _pw, _ph = layer.panel
    q = next(k for k in layer.kb.keys if k.label == "q")
    assert layer.press(px + q.x + q.w / 2, py + q.y + q.h / 2) == ("text", "q")
    assert layer.kb.keys[layer.flash].label == "q"
    lit = _pixels(layer)[int(py + q.y + q.h / 2), int(q.x + 6)]
    assert (lit == 0).all()                                     # pressed: the key turns black
    shift = next(k for k in layer.kb.keys if k.action == ("shift",))
    assert layer.press(px + shift.x + 5, py + shift.y + 5) is None
    assert "Q" in [k.label for k in layer.kb.keys]
    assert layer.press(5, 5) is None                            # above the keyboard: nothing
    layer.set_dots([(px + q.x + q.w / 2, py + q.y + q.h / 2, "key")])
    assert layer.kb.keys[next(iter(layer.hover))].label == "Q"


# ================================================================== the camera
def test_the_video_requests_are_the_kernels(tmp_path):
    cc = shutil.which("cc") or shutil.which("gcc")
    if not cc or not Path("/usr/include/linux/videodev2.h").exists():
        pytest.skip("no C compiler or kernel headers")
    src = tmp_path / "v.c"
    src.write_text('#include <stdio.h>\n#include <linux/videodev2.h>\nint main(void){printf("%lu %lu %lu %lu %lu %lu '
                   '%lu %lu %lu %lu %lu %lu", (unsigned long)VIDIOC_QUERYCAP, (unsigned long)VIDIOC_ENUM_FMT, '
                   '(unsigned long)VIDIOC_S_FMT, (unsigned long)VIDIOC_REQBUFS, (unsigned long)VIDIOC_QUERYBUF, '
                   '(unsigned long)VIDIOC_QBUF, (unsigned long)VIDIOC_DQBUF, (unsigned long)VIDIOC_STREAMON, '
                   '(unsigned long)VIDIOC_STREAMOFF, (unsigned long)VIDIOC_S_PARM, (unsigned long)VIDIOC_ENUM_FRAMESIZES, '
                   '(unsigned long)VIDIOC_ENUM_FRAMEINTERVALS);return 0;}')
    subprocess.run([cc, "-o", str(tmp_path / "v"), str(src)], check=True, capture_output=True)
    got = [int(v) for v in subprocess.run([str(tmp_path / "v")], capture_output=True, text=True).stdout.split()]
    assert got == [camera.VIDIOC_QUERYCAP, camera.VIDIOC_ENUM_FMT, camera.VIDIOC_S_FMT, camera.VIDIOC_REQBUFS,
                   camera.VIDIOC_QUERYBUF, camera.VIDIOC_QBUF, camera.VIDIOC_DQBUF, camera.VIDIOC_STREAMON,
                   camera.VIDIOC_STREAMOFF, camera.VIDIOC_S_PARM, camera.VIDIOC_ENUM_FRAMESIZES,
                   camera.VIDIOC_ENUM_FRAMEINTERVALS]
    assert camera.YUYV == 0x56595559 and camera.MJPG == 0x47504A4D


def test_yuyv_pictures_become_the_right_colours():
    # White, black, and BT.601 red (Y 81, U 90, V 240), each two pixels wide.
    quads = [(235, 128, 235, 128), (16, 128, 16, 128), (81, 90, 81, 240)]
    row = bytes(v for q in quads for v in q)
    img = np.asarray(camera.yuyv_to_image(row * 2, 6, 2))
    assert img.shape == (2, 6, 3)
    assert np.all(img[0, 0] >= 254) and np.all(img[0, 2] <= 1)
    assert img[0, 4][0] > 240 and img[0, 4][1] < 15 and img[0, 4][2] < 15
    luma = camera.yuyv_luma(row * 2, 6, 2, step=2)
    assert luma.shape == (1, 3) and list(luma[0]) == [235, 16, 81]


def test_mjpeg_pictures_without_their_tables_are_read():
    buf = io.BytesIO()
    Image.new("RGB", (64, 48), (200, 30, 30)).save(buf, "JPEG", quality=90)
    data, out, i = buf.getvalue(), bytearray(b"\xff\xd8"), 2
    while data[i] == 0xFF and data[i + 1] != 0xDA:           # drop the Huffman tables, as many cameras do
        n = struct.unpack_from(">H", data, i + 2)[0]
        if data[i + 1] != 0xC4:
            out += data[i:i + 2 + n]
        i += 2 + n
    out += data[i:]
    assert b"\xff\xc4" not in out
    img = camera.mjpeg_to_image(bytes(out))
    assert img.size == (64, 48) and img.getpixel((32, 24))[0] > 180


def test_the_format_chosen_is_the_quick_one():
    dev = camera.Device("/dev/video0", "Studio Display Camera",
                        {camera.YUYV: [(1920, 1080, 5.0), (640, 480, 30.0)], camera.MJPG: [(1920, 1080, 30.0)]})
    assert camera.choose(dev) == (camera.YUYV, 640, 480)
    dev.formats[camera.YUYV] = [(1920, 1080, 5.0)]            # uncompressed only slowly: the compressed one
    assert camera.choose(dev) == (camera.MJPG, 1920, 1080)
    dev.formats[camera.MJPG] = [(1920, 1080, 30.0), (1280, 720, 30.0), (640, 360, 30.0)]
    assert camera.choose(dev) == (camera.MJPG, 640, 360)


def test_no_cameras_means_none():
    assert camera.devices() == [] or all(isinstance(d, camera.Device) for d in camera.devices())


def test_damaged_pictures_from_the_camera_are_passed_over(monkeypatch):
    # The camera marks a picture it may have damaged; an uncompressed one cut short can't be read either. Each
    # buffer still goes back to the camera.
    cam = object.__new__(camera.Camera)
    cam.fd, cam.fmt, cam.width, cam.height, cam.stride = -1, camera.YUYV, 4, 2, 8
    cam.maps = [bytes(range(16)), bytes(range(16, 32))]
    script = [(0, 16, camera.BUF_FLAG_ERROR), (1, 10, 0), (0, 16, 0), (1, 16, 0)]   # damaged, short, whole, whole
    queued = []

    def ioctl(fd, request, buf):
        if request == camera.VIDIOC_DQBUF:
            if not script:
                raise OSError(19, "No such device")            # unplugged: the pictures end
            i, used, flags = script.pop(0)
            struct.pack_into("<II", buf, 0, i, camera.BUF_TYPE_VIDEO_CAPTURE)
            struct.pack_into("<II", buf, 8, used, flags)
        elif request == camera.VIDIOC_QBUF:
            queued.append(struct.unpack_from("<I", buf, 0)[0])
        return buf
    monkeypatch.setattr(camera, "_ioctl", ioctl)
    monkeypatch.setattr(camera, "select", type("Select", (), {"select": staticmethod(lambda r, w, x, t: (r, w, x))}))
    assert list(cam.frames()) == [bytes(range(16)), bytes(range(16, 32))]
    assert queued == [0, 1, 0, 1]


# ================================================================== the pointer
@pytest.mark.skipif(not os.environ.get("DISPLAY"), reason="no X display")
def test_the_pointer_moves_through_xtest():
    from chatcomputer.airtouch.xinput import XInput
    try:
        x = XInput()
    except OSError:
        pytest.skip("can't reach the X display")
    with x:
        w, h = x.screen_size()
        x.move(w // 3, h // 4)
        assert x.position() == (w // 3, h // 4)
        x.scroll(0, 2)
        x.show_cursor(False)              # the arrow hidden, and back (XFIXES)
        x.show_cursor(True)


class _Mouse:
    """A pretend display pointer that a pretend mouse can move."""

    def __init__(self):
        self.pos, self.arrow, self.did = (960, 540), [], []

    def position(self):
        return self.pos

    def move(self, x, y):
        self.pos = (round(x), round(y))
        self.did.append(("move", self.pos))

    def click(self, x, y, button=1):
        self.pos = (round(x), round(y))
        self.did.append(("click", self.pos))

    def scroll(self, dx, dy):
        self.did.append(("scroll", (dx, dy)))

    def show_cursor(self, show):
        self.arrow.append(show)


def test_the_arrow_hides_while_hands_move_the_pointer_and_a_mouse_gets_it_back():
    from chatcomputer.airtouch.xinput import MOUSE_YIELD_S, SharedPointer
    m = _Mouse()
    p = SharedPointer(m)
    p.move(0.0, 100, 100)
    assert m.arrow == [False] and m.pos == (100, 100)
    p.move(0.1, 120, 100)
    m.pos = (500, 500)                             # a mouse moved it
    p.move(0.2, 140, 100)
    assert m.arrow == [False, True] and m.pos == (500, 500)       # the arrow is back, and the hand waits
    p.move(0.2 + MOUSE_YIELD_S - 0.1, 160, 100)
    assert m.pos == (500, 500)
    p.click(0.2 + MOUSE_YIELD_S - 0.05, 170, 100)                  # a pinch is meant: it clicks anyway
    assert m.did[-1] == ("click", (170, 100))
    p.move(0.3 + MOUSE_YIELD_S, 180, 100)                          # the mouse has been still: the hand again
    assert m.pos == (180, 100) and m.arrow[-1] is False
    p.release()
    assert m.arrow[-1] is True


def test_a_mouse_that_goes_on_moving_keeps_the_pointer():
    from chatcomputer.airtouch.xinput import MOUSE_YIELD_S, SharedPointer
    m = _Mouse()
    p = SharedPointer(m)
    p.move(0.0, 100, 100)
    m.pos = (500, 500)                                             # a mouse moved it
    p.move(0.1, 120, 100)
    for i in range(1, 6):                                          # and goes on moving it, for longer than the wait
        m.pos = (500 + 20 * i, 500)
        p.move(0.1 + i * MOUSE_YIELD_S / 2, 120 + i, 100)
        assert m.pos == (500 + 20 * i, 500), i
    assert m.arrow == [False, True]
    p.move(0.2 + 3.5 * MOUSE_YIELD_S, 180, 100)                    # still for long enough: the hand again
    assert m.pos == (180, 100) and m.arrow[-1] is False


# ================================================================== the whole process, with a pretend camera
class _FakeCamera:
    """MJPEG pictures (each a JPEG file), made from a test picture moved across a bigger frame."""
    fmt, width, height, stride = camera.MJPG, 640, 480, 0

    def __init__(self, frames):
        self._frames = frames

    def frames(self, idle_s=3.0):
        for f in self._frames:
            time.sleep(1 / 30)
            yield f

    def decode(self, data, max_width=960):
        return camera.mjpeg_to_image(data)

    def close(self):
        pass


def _frame(name: str, dx: int, dy: int) -> bytes:
    pic = Image.open(PICTURES / name).convert("RGB")
    canvas = Image.new("RGB", (640, 480), (90, 90, 90))
    canvas.paste(pic, (140 + dx, 60 + dy))
    buf = io.BytesIO()
    canvas.save(buf, "JPEG", quality=92)
    return buf.getvalue()


@need_models
def test_the_airtouch_process_follows_the_finger_and_sees_a_thumbs_up(monkeypatch):
    from chatcomputer.airtouch import worker, xinput
    sent = []
    mouse = _Mouse()
    frames = ([_frame("pointing_up.jpg", 6 * i, 3 * i) for i in range(20)] +
              [_frame("thumb_up.jpg", 0, 0) for _ in range(40)])

    class Pointer(_Mouse):
        def __init__(self, *a):
            pass

        def __getattr__(self, name):
            return getattr(mouse, name)

        def screen_size(self):
            return 1920, 1080

        def close(self):
            pass

    class Conn:
        """Like a pipe: a message waits until it's read."""
        waiting = []

        def send(self, msg):
            sent.append(msg)

        def poll(self, timeout=0):
            return bool(self.waiting)

        def recv(self):
            return self.waiting.pop(0)

    conn = Conn()
    monkeypatch.setattr(xinput, "XInput", Pointer)
    monkeypatch.setattr(camera, "devices", lambda: [camera.Device("/dev/video9", "Pretend", {camera.MJPG: [(640, 480, 30.0)]})])

    def fake_camera(dev):
        cam = _FakeCamera(frames)
        real = cam.frames

        def once(idle_s=3.0):
            yield from real()
            conn.waiting.append(("stop",))          # then Chat Computer says stop
        cam.frames = once
        return cam
    monkeypatch.setattr(camera, "Camera", fake_camera)
    worker.main(conn, {"palm_model": str(PALM), "landmark_model": str(LANDMARKS), "bounds": (0, 0, 1920, 1080)})
    states = [m[1]["state"] for m in sent if m[0] == "status"]
    assert states[:2] == ["starting", "running"]
    assert ("hand", True) in sent
    moves = [pos for kind, pos in mouse.did if kind == "move"]
    assert len(moves) > 3 and moves[-1][0] < moves[0][0]       # the hand went right in the picture: the pointer left
    assert ("gesture", "yes") in sent
    dots = [m[1] for m in sent if m[0] == "dots"]
    assert dots and all(len(d) <= 2 for d in dots) and any(d and d[0][2] == "point" for d in dots)
    assert mouse.arrow[0] is False and mouse.arrow[-1] is True      # hidden while it moved; shown when it stopped


@need_models
def test_a_damaged_picture_is_passed_over_and_the_camera_goes_on(monkeypatch):
    # A picture cut short, or that isn't one, now and then: the next ones are looked at, without stopping the
    # camera to start it again (and losing the hand).
    from chatcomputer.airtouch import worker, xinput
    sent, opened = [], []
    whole = [_frame("pointing_up.jpg", 6 * i, 3 * i) for i in range(16)]
    frames = [whole[0][: len(whole[0]) // 2], b"\xff\xd8 not a picture"] + whole[:8] + [whole[8][:3000]] + whole[8:]

    class Pointer(_Mouse):
        def __init__(self, *a):
            super().__init__()

        def screen_size(self):
            return 1920, 1080

        def close(self):
            pass

    class Conn:
        waiting = []

        def send(self, msg):
            sent.append(msg)

        def poll(self, timeout=0):
            return bool(self.waiting)

        def recv(self):
            return self.waiting.pop(0)

    conn = Conn()
    monkeypatch.setattr(xinput, "XInput", Pointer)
    monkeypatch.setattr(camera, "devices", lambda: [camera.Device("/dev/video9", "Pretend", {camera.MJPG: [(640, 480, 30.0)]})])

    def fake_camera(dev):
        cam = _FakeCamera(frames if not opened else whole)       # (the second time, should it be started again)
        opened.append(cam)
        real = cam.frames

        def once(idle_s=3.0):
            yield from real()
            conn.waiting.append(("stop",))
        cam.frames = once
        return cam
    monkeypatch.setattr(camera, "Camera", fake_camera)
    worker.main(conn, {"palm_model": str(PALM), "landmark_model": str(LANDMARKS), "bounds": (0, 0, 1920, 1080)})
    states = [m[1]["state"] for m in sent if m[0] == "status"]
    assert states == ["starting", "running", "no camera"] and len(opened) == 1
    assert ("hand", True) in sent and ("hand", False) not in sent


# ================================================================== what Chat Computer does with it
def _controller(tmp_path, monkeypatch):
    from chatcomputer.config import Paths, Settings
    from chatcomputer.controller import Controller
    from chatcomputer.platform.fake import FakePlatform
    from chatcomputer.screen import RecordingScreen
    monkeypatch.setenv("HOME", str(tmp_path / "home"))
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "home" / "ChatComputer"))
    # AirTouch is off unless asked for: these are about it once it's on.
    return Controller(Paths().ensure(), Settings(user_name="Ren", airtouch=True), FakePlatform(), RecordingScreen(),
                      voice=False, index=False)


class _AirTouch:
    """Stands in for AirTouch's process."""

    def __init__(self):
        self.keyboards, self.stopped = [], False

    def set_keyboard(self, spec):
        self.keyboards.append(spec)

    def stop(self):
        self.stopped = True


def test_an_open_hand_stops_chatcomputer_talking_and_wakes_it(tmp_path, monkeypatch):
    c = _controller(tmp_path, monkeypatch)
    stopped = []

    class Speaker:
        speaking = True

        def stop(self):
            stopped.append(True)
            self.speaking = False

        def say(self, text):
            stopped.append(text)

    async def run():
        c.loop = asyncio.get_running_loop()
        c.speaker = Speaker()
        c._on_airtouch(("dots", [(100.0, 200.0, "point"), (300.4, 200.0, "pinch")]))
        c._on_airtouch(("gesture", "palm"))
        await asyncio.sleep(0.05)
        c.asleep = True
        c._on_airtouch(("gesture", "palm"))
        await asyncio.sleep(0.05)
    asyncio.run(run())
    assert ("dots", [(100, 200, "point"), (300, 200, "pinch")]) in c.screen.events
    assert stopped[0] is True and not c.asleep


def test_thumbs_up_answers_a_question(tmp_path, monkeypatch):
    c = _controller(tmp_path, monkeypatch)

    async def run():
        c.loop = asyncio.get_running_loop()
        c.pending_confirm = c.loop.create_future()
        c._on_airtouch(("gesture", "yes"))
        await asyncio.sleep(0.05)
        return c.pending_confirm.result()
    assert asyncio.run(run()) == "yes"
    assert any(e[0] == "user" and "thumbs up" in e[1][1] for e in c.screen.events)


def _key_at(c, label):
    """Where a key of the keyboard on the pretend screen is (display pixels)."""
    k = next(k for k in c.screen.keyboard.keys if k.label == label)
    return k.x + k.w / 2, c.screen.KEYBOARD_TOP + k.y + k.h / 2


def test_beckons_bring_the_keyboard_and_take_it_away_and_fingers_type_on_it_in_order(tmp_path, monkeypatch):
    c = _controller(tmp_path, monkeypatch)
    at = _AirTouch()

    async def run():
        c.loop = asyncio.get_running_loop()
        c.airtouch = at
        c._on_airtouch(("gesture", "keyboard up"))
        await asyncio.sleep(0.05)
        assert c.keyboard_up and at.keyboards[-1][4:] == (keyboard.UNITS, keyboard.ROWS)
        c._on_airtouch(("gesture", "keyboard up"))           # already up: stays up
        await asyncio.sleep(0.05)
        assert c.keyboard_up and len(at.keyboards) == 1
        for label in ("h", "i", "space", "space", "return"):
            c._on_airtouch(("key",) + _key_at(c, label))
        await asyncio.sleep(0.3)
        c._on_airtouch(("key", 5.0, 5.0))                 # not on the keyboard: nothing
        c._on_airtouch(("gesture", "keyboard down"))
        await asyncio.sleep(0.1)
    asyncio.run(run())
    typed = [call for call in c.platform.calls if call[0] in ("type_text", "key")]
    assert typed == [("type_text", ("h",)), ("type_text", ("i",)), ("type_text", (" ",)), ("key", ("BackSpace", 1)),
                     ("type_text", (". ",)), ("key", ("Return", 1))]
    assert not c.keyboard_up and at.keyboards[-1] is None
    assert ("keyboard", True) in c.screen.events and ("keyboard", False) in c.screen.events


def test_the_keyboards_own_keys_hide_it_and_dictate(tmp_path, monkeypatch):
    c = _controller(tmp_path, monkeypatch)
    at = _AirTouch()

    async def run():
        c.loop = asyncio.get_running_loop()
        c.airtouch = at
        await c.set_keyboard(True)
        c._on_airtouch(("key",) + _key_at(c, "dictate"))      # the microphone key: what's said next is typed
        await asyncio.sleep(0.1)
        c._on_utterance("see you at six", 1.2)
        await asyncio.sleep(0.1)
        c._on_utterance("thanks", 0.5)                      # and then it's for Claude again
        await asyncio.sleep(0.1)
        c._on_airtouch(("key",) + _key_at(c, "hide keyboard"))
        await asyncio.sleep(0.1)
    handled = []

    async def handle(text):
        handled.append(text)
    c.handle = handle
    asyncio.run(run())
    assert ("type_text", ("see you at six",)) in c.platform.calls and handled == ["thanks"]
    assert any(e[0] == "notice" and "Say what to type" in e[1][0] for e in c.screen.events)
    assert not c.keyboard_up and at.keyboards[-1] is None


def test_the_keyboard_can_be_asked_for_with_or_without_airtouch(tmp_path, monkeypatch):
    c = _controller(tmp_path, monkeypatch)

    async def run():
        c.loop = asyncio.get_running_loop()
        # Without AirTouch, it's tapped on the touchscreen.
        assert await c.set_keyboard(True) == "The keyboard is up: tap its keys on the screen." and c.keyboard_up
        await c.set_keyboard(False)
        c.airtouch = _AirTouch()
        assert "fingers in the air" in await c.set_keyboard(True)
        c.airtouch.status = "x"
        await c._stop_airtouch()
    asyncio.run(run())
    assert not c.keyboard_up and c.screen.events[-1] == ("keyboard", False)
    assert ("dots", []) in c.screen.events


def test_the_first_hand_explains_airtouch_once(tmp_path, monkeypatch):
    c = _controller(tmp_path, monkeypatch)

    async def run():
        c.loop = asyncio.get_running_loop()
        for _ in range(2):
            c._on_airtouch(("hand", True))
        c._on_airtouch(("status", {"state": "running", "camera": "Studio Display Camera"}))
    asyncio.run(run())
    notices = [e for e in c.screen.events if e[0] == "notice" and "AirTouch" in e[1][0]]
    assert len(notices) == 1 and c._airtouch_info()["camera"] == "Studio Display Camera"
    assert "keyboard" in notices[0][1][0] and "dot" in notices[0][1][0]


def test_turning_airtouch_off_stops_it(tmp_path, monkeypatch):
    c = _controller(tmp_path, monkeypatch)
    c.airtouch_enabled = True
    at = _AirTouch()

    async def run():
        c.loop = asyncio.get_running_loop()
        c.airtouch = at
        c.settings.airtouch = False
        c._settings_changed(["airtouch"])
        await asyncio.sleep(0.1)
    asyncio.run(run())
    assert at.stopped and c.airtouch is None and c._airtouch_info()["state"].startswith("off")

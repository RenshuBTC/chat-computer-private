"""Speaking: a natural local voice (Kokoro), turning the model's streamed text into sentences.

The model's text is cleaned for the ear (no markdown symbols, code or long URLs) and cut into
sentences as it streams, so Chat Computer starts talking before the model has finished writing.
"""
from __future__ import annotations

import logging
import re
import shutil
import subprocess
from typing import Iterator

import numpy as np

log = logging.getLogger("chatcomputer.tts")
OUT_RATE = 24000

ABBREV = {"mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "etc", "e.g", "i.e", "approx", "no", "inc",
          "ltd", "co", "jan", "feb", "mar", "apr", "jun", "jul", "aug", "sep", "sept", "oct", "nov", "dec", "a.m", "p.m"}
EMOJI = re.compile("[\U0001F000-\U0001FAFF\U00002600-\U000027BF\U0001F1E6-\U0001F1FF‍️]")


def speakable(text: str) -> str:
    """Markdown → words to say."""
    t = text
    t = re.sub(r"```.*?(```|$)", " ", t, flags=re.S)                 # code blocks: not read aloud
    t = re.sub(r"`([^`]*)`", r"\1", t)
    t = re.sub(r"!\[[^\]]*\]\([^)]*\)", " ", t)                        # images
    t = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", r"\1", t)                   # links: say the text
    t = re.sub(r"https?://(www\.)?([^/\s]+)\S*", r"\2", t)              # bare URLs: say the site
    t = re.sub(r"^\s*\|.*\|\s*$", " ", t, flags=re.M)                  # tables are for the screen
    t = re.sub(r"^\s{0,3}#{1,6}\s*", "", t, flags=re.M)                 # headings
    t = re.sub(r"^\s*([-*+]|\d+[.)])\s+", "", t, flags=re.M)            # list markers
    t = re.sub(r"^\s*>\s?", "", t, flags=re.M)
    t = re.sub(r"(\*\*|__|~~)", "", t)
    t = re.sub(r"(?<!\w)[*_](\S[^*_]*?)[*_](?!\w)", r"\1", t)
    t = EMOJI.sub("", t)
    t = t.replace("→", " to ").replace("—", ", ").replace("–", " to ").replace("&", " and ")
    t = re.sub(r"\s*\n\s*", "\n", t)
    return re.sub(r"[ \t]+", " ", t).strip()


class SentenceSplitter:
    """Feed streamed text; get back complete sentences ready to speak."""

    MIN_FIRST_CHARS = 20     # a first piece shorter than this isn't worth the wait for the next one

    def __init__(self, first_chunk_chars: int = 50) -> None:
        self.buf = ""
        self.in_code = False
        self.first = True
        self.first_chunk_chars = first_chunk_chars

    def feed(self, delta: str) -> list[str]:
        self.buf += delta
        return list(self._drain(final=False))

    def flush(self) -> list[str]:
        out = list(self._drain(final=True))
        rest = speakable(self.buf) if not self.in_code else ""
        self.buf = ""
        if rest:
            out.append(rest)
        return [s for s in out if re.search(r"\w", s)]

    def _drain(self, final: bool) -> Iterator[str]:
        while True:
            # Skip fenced code entirely.
            if self.in_code:
                end = self.buf.find("```")
                if end < 0:
                    return
                self.buf = self.buf[end + 3:]
                self.in_code = False
                continue
            start = self.buf.find("```")
            cut = self._find_cut()
            if start >= 0 and (cut is None or start < cut):
                before = self.buf[:start]
                self.buf = self.buf[start + 3:]
                self.in_code = True
                s = speakable(before)
                if s:
                    yield s
                continue
            if cut is None:
                return
            piece, self.buf = self.buf[:cut], self.buf[cut:]
            s = speakable(piece)
            if s and re.search(r"\w", s):
                self.first = False
                yield s

    def _find_cut(self) -> int | None:
        b = self.buf
        for m in re.finditer(r"([.!?…]+[\"')\]]*)(\s+)|\n", b):
            if m.group(0) == "\n" or m.group(0).startswith("\n"):
                if b[: m.start()].strip():
                    return m.end()
                continue
            end_punct = m.end(1)
            before = b[: m.start(1)]
            word = re.split(r"\s", before.strip())[-1].lower().rstrip(".") if before.strip() else ""
            if m.group(1).startswith(".") and (word in ABBREV or re.fullmatch(r"[a-z]", word)):
                continue
            if m.group(1) == "." and re.search(r"\d$", before) and re.match(r"\d", b[m.end():m.end() + 1] or ""):
                continue
            return end_punct
        # Start talking sooner: a long first sentence is cut at its first comma (or ; or :) after a few words.
        # Speaking can't start before its first piece is made, which takes longer the longer it is.
        if self.first and len(b) > self.first_chunk_chars:
            for m in re.finditer(r"[,;:]\s", b[: self.first_chunk_chars + 50]):
                if m.end() >= self.MIN_FIRST_CHARS:
                    return m.end()
        return None


class TTS:
    """Kokoro voice if its model is installed, otherwise espeak-ng."""

    def __init__(self, onnx_path: str | None, voices_path: str | None, voice: str = "af_heart", speed: float = 1.0,
                 threads: int = 0) -> None:
        self.voice = voice
        self.speed = speed
        self.kokoro = None
        if onnx_path and voices_path:
            try:
                import onnxruntime as ort
                from kokoro_onnx import Kokoro
                opts = ort.SessionOptions()
                # 0: one thread per processor core, onnxruntime's choice. A fixed 4 took nearly twice as long on a
                # two-core PC (threads waiting for each other) and couldn't use more cores on a bigger one.
                opts.intra_op_num_threads = threads
                opts.inter_op_num_threads = 1
                sess = ort.InferenceSession(onnx_path, sess_options=opts, providers=["CPUExecutionProvider"])
                self.kokoro = Kokoro.from_session(sess, voices_path)
                if voice not in self.kokoro.get_voices():
                    self.voice = "af_heart"
            except Exception:
                log.exception("Kokoro voice unavailable; using espeak-ng")
                self.kokoro = None
        self.espeak = shutil.which("espeak-ng") or shutil.which("espeak")

    @property
    def available(self) -> bool:
        return self.kokoro is not None or bool(self.espeak)

    def voices(self) -> list[str]:
        return sorted(self.kokoro.get_voices()) if self.kokoro else []

    def _speed(self) -> float:
        try:
            s = float(self.speed)
        except (TypeError, ValueError):
            return 1.0
        return 1.0 if np.isnan(s) else s

    def synth(self, text: str) -> np.ndarray:
        """24 kHz mono float32 audio for one sentence."""
        text = text.strip()
        if not text:
            return np.zeros(0, dtype=np.float32)
        speed = self._speed()
        if self.kokoro is not None:
            # A voice or speed Kokoro doesn't have (a settings change, a hand-edited file) would make every sentence
            # fail over to espeak: the usual voice, and a speed in Kokoro's range (0.5 to 2.0), are used instead.
            voice = self.voice if self.voice in self.kokoro.get_voices() else "af_heart"
            try:
                audio, sr = self.kokoro.create(text, voice=voice, speed=min(2.0, max(0.5, speed)), lang="en-us")
                return _resample(np.asarray(audio, dtype=np.float32), sr, OUT_RATE)
            except Exception:
                log.exception("Kokoro failed on %r", text[:60])
        if self.espeak:
            r = subprocess.run([self.espeak, "--stdout", "-s", str(int(175 * min(2.0, max(0.5, speed)))), "--", text],
                               capture_output=True, timeout=30)
            if r.returncode == 0 and r.stdout:
                return _wav_to_float(r.stdout)
        return np.zeros(0, dtype=np.float32)


def _resample(a: np.ndarray, sr: int, to: int) -> np.ndarray:
    if sr == to or a.size == 0:
        return a.astype(np.float32, copy=False)
    try:
        import soxr
        return soxr.resample(a, sr, to).astype(np.float32)
    except Exception:
        n = int(round(a.size * to / sr))
        return np.interp(np.linspace(0, a.size - 1, n), np.arange(a.size), a).astype(np.float32)


def _wav_to_float(wav: bytes) -> np.ndarray:
    import io
    import wave
    with wave.open(io.BytesIO(wav)) as w:
        sr = w.getframerate()
        frames = w.readframes(w.getnframes())
        data = np.frombuffer(frames, dtype=np.int16).astype(np.float32) / 32768.0
        if w.getnchannels() > 1:
            data = data.reshape(-1, w.getnchannels()).mean(axis=1)
    return _resample(data, sr, OUT_RATE)

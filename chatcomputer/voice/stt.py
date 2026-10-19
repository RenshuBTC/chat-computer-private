"""Speech recognition on the PC with faster-whisper."""
from __future__ import annotations

import logging
import os
import re
from typing import Any

import numpy as np

log = logging.getLogger("chatcomputer.stt")

# Phrases Whisper tends to invent from silence or noise.
HALLUCINATIONS = {
    "thank you", "thank you.", "thanks for watching", "thanks for watching!", "thank you for watching",
    "you", "bye", "bye.", "bye-bye", "okay.", "so", "uh", "um", "hmm", "mm", "the end", ".", "...",
    "subtitles by the amara.org community", "please subscribe", "like and subscribe", "i'm sorry",
}


class STT:
    def __init__(self, model_dir: str, language: str = "en", threads: int | None = None, hint: str = "") -> None:
        from faster_whisper import WhisperModel
        threads = threads or max(2, min(8, (os.cpu_count() or 4) // 2))
        self.model = WhisperModel(model_dir, device="cpu", compute_type="int8", cpu_threads=threads, num_workers=1)
        self.language = None if language in ("", "auto") else language
        self.hint = hint

    def transcribe(self, audio: np.ndarray, fast: bool = False) -> tuple[str, dict[str, Any]]:
        """Text for 16 kHz mono float32 audio, and some confidence info."""
        if audio.size < 1600:
            return "", {"reason": "too short"}
        segments, info = self.model.transcribe(
            audio, language=self.language, beam_size=1 if fast else 3, best_of=1, temperature=0.0,
            condition_on_previous_text=False, without_timestamps=True, vad_filter=False,
            initial_prompt=self.hint or None, suppress_blank=True, no_speech_threshold=0.6,
        )
        texts, nsp, lp = [], [], []
        for s in segments:
            texts.append(s.text)
            nsp.append(s.no_speech_prob)
            lp.append(s.avg_logprob)
        text = re.sub(r"\s+", " ", "".join(texts)).strip()
        meta = {"no_speech": max(nsp) if nsp else 1.0, "logprob": min(lp) if lp else -9.0,
                "language": getattr(info, "language", None)}
        if not fast and is_noise(text, meta, audio.size / 16000):
            return "", {**meta, "reason": "noise"}
        return text, meta


def is_noise(text: str, meta: dict[str, Any], seconds: float) -> bool:
    t = text.strip().lower()
    if not t or not re.search(r"\w", t):
        return True
    if t in HALLUCINATIONS and (meta.get("no_speech", 0) > 0.3 or seconds < 1.2 or meta.get("logprob", 0) < -0.7):
        return True
    if meta.get("no_speech", 0) > 0.75 and meta.get("logprob", 0) < -0.8:
        return True
    return False

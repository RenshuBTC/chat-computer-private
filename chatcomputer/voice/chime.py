"""The sounds of a timer, an alarm and a reminder: short bell-like tones, made here (no sound files), for the voice's
player (24 kHz mono)."""
from __future__ import annotations

import numpy as np

RATE = 24000
NOTES = {   # Hz: a reminder is two soft notes, a timer two brighter ones, an alarm three rising
    "reminder": (784.0, 1046.5),
    "timer": (1046.5, 1318.5),
    "alarm": (880.0, 1108.7, 1318.5),
}


def _bell(freq: float, seconds: float, loud: float) -> np.ndarray:
    t = np.arange(int(RATE * seconds), dtype=np.float32) / RATE
    tone = np.sin(2 * np.pi * freq * t) + 0.35 * np.sin(2 * np.pi * 2 * freq * t) + 0.12 * np.sin(2 * np.pi * 3 * freq * t)
    attack = np.minimum(1.0, t / 0.005)              # no click at the start
    return (loud * attack * np.exp(-t * 4.5) * tone / 1.47).astype(np.float32)


def chime(kind: str, loud: float = 0.32) -> np.ndarray:
    gap = np.zeros(int(RATE * 0.07), dtype=np.float32)
    parts: list[np.ndarray] = []
    for f in NOTES.get(kind, NOTES["timer"]):
        parts += [_bell(f, 0.42, loud), gap]
    return np.concatenate(parts)

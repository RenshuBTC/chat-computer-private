"""Streaming speech detection with the Silero VAD model (16 kHz, 32 ms frames)."""
from __future__ import annotations

import numpy as np

FRAME = 512          # samples per frame at 16 kHz = 32 ms
CONTEXT = 64
RATE = 16000


class StreamingVAD:
    def __init__(self, model_path: str) -> None:
        import onnxruntime as ort
        opts = ort.SessionOptions()
        opts.inter_op_num_threads = 1
        opts.intra_op_num_threads = 1
        opts.log_severity_level = 4
        self.session = ort.InferenceSession(model_path, providers=["CPUExecutionProvider"], sess_options=opts)
        self.reset()

    def reset(self) -> None:
        self.h = np.zeros((1, 1, 128), dtype=np.float32)
        self.c = np.zeros((1, 1, 128), dtype=np.float32)
        self.context = np.zeros(CONTEXT, dtype=np.float32)

    def __call__(self, frame: np.ndarray) -> float:
        """Probability (0-1) that this 512-sample frame contains speech."""
        x = np.concatenate([self.context, frame.astype(np.float32, copy=False)])[None, :]
        prob, self.h, self.c = self.session.run(None, {"input": x, "h": self.h, "c": self.c})
        self.context = frame[-CONTEXT:].astype(np.float32, copy=True)
        return float(prob[0])

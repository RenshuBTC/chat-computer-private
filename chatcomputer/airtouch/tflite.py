"""Reads TensorFlow Lite model files: the format the hand model comes in.

A .tflite file is a FlatBuffer. This reads it directly, using the table layouts in TFLite's schema.fbs (the
order of each table's fields is what matters), into plain tensors and operators that nn.py runs. No TFLite
or FlatBuffers library is used.
"""
from __future__ import annotations

import struct
from dataclasses import dataclass, field
from typing import Any

import numpy as np

# BuiltinOperator codes (schema.fbs) of the operators the hand models use, and a few close relatives.
OPS = {0: "ADD", 1: "AVERAGE_POOL_2D", 2: "CONCATENATION", 3: "CONV_2D", 4: "DEPTHWISE_CONV_2D", 6: "DEQUANTIZE",
       9: "FULLY_CONNECTED", 14: "LOGISTIC", 17: "MAX_POOL_2D", 18: "MUL", 19: "RELU", 21: "RELU6", 22: "RESHAPE",
       23: "RESIZE_BILINEAR", 28: "TANH", 34: "PAD", 40: "MEAN", 41: "SUB", 54: "PRELU", 67: "TRANSPOSE_CONV",
       97: "RESIZE_NEAREST_NEIGHBOR", 117: "HARD_SWISH"}
# TensorType -> numpy dtype.
DTYPES = {0: np.float32, 1: np.float16, 2: np.int32, 3: np.uint8, 4: np.int64, 6: np.bool_, 7: np.int16, 9: np.int8,
          10: np.float64}
PADDING = {0: "SAME", 1: "VALID"}
ACTIVATION = {0: None, 1: "RELU", 2: "RELU_N1_TO_1", 3: "RELU6", 4: "TANH"}


class _Table:
    """One FlatBuffer table: its fields are found through its vtable, by slot number."""
    __slots__ = ("buf", "pos", "vt", "vlen")

    def __init__(self, buf: bytes, pos: int) -> None:
        self.buf = buf
        self.pos = pos
        self.vt = pos - struct.unpack_from("<i", buf, pos)[0]
        self.vlen = struct.unpack_from("<H", buf, self.vt)[0]

    def _field(self, slot: int) -> int:
        o = 4 + 2 * slot
        return struct.unpack_from("<H", self.buf, self.vt + o)[0] if o < self.vlen else 0

    def scalar(self, slot: int, fmt: str, default: Any = 0) -> Any:
        o = self._field(slot)
        return struct.unpack_from("<" + fmt, self.buf, self.pos + o)[0] if o else default

    def _target(self, slot: int) -> int | None:
        o = self._field(slot)
        if not o:
            return None
        p = self.pos + o
        return p + struct.unpack_from("<I", self.buf, p)[0]

    def table(self, slot: int) -> "_Table | None":
        p = self._target(slot)
        return _Table(self.buf, p) if p is not None else None

    def string(self, slot: int) -> str:
        p = self._target(slot)
        if p is None:
            return ""
        n = struct.unpack_from("<I", self.buf, p)[0]
        return bytes(self.buf[p + 4:p + 4 + n]).decode("utf-8", "replace")

    def _vector(self, slot: int) -> tuple[int, int]:
        p = self._target(slot)
        return (0, 0) if p is None else (p + 4, struct.unpack_from("<I", self.buf, p)[0])

    def scalars(self, slot: int, fmt: str) -> list[Any]:
        start, n = self._vector(slot)
        return list(struct.unpack_from("<%d%s" % (n, fmt), self.buf, start)) if n else []

    def tables(self, slot: int) -> list["_Table"]:
        start, n = self._vector(slot)
        out = []
        for i in range(n):
            p = start + 4 * i
            out.append(_Table(self.buf, p + struct.unpack_from("<I", self.buf, p)[0]))
        return out

    def raw(self, slot: int) -> bytes:
        start, n = self._vector(slot)
        return bytes(self.buf[start:start + n]) if n else b""


@dataclass
class Tensor:
    name: str
    shape: tuple[int, ...]
    dtype: Any
    data: np.ndarray | None          # constant (weights), or None for a value computed while running
    scale: list[float] = field(default_factory=list)
    zero_point: list[int] = field(default_factory=list)


@dataclass
class Operator:
    op: str
    inputs: list[int]                # tensor indices; -1 is an absent optional input
    outputs: list[int]
    options: dict[str, Any]


@dataclass
class Model:
    tensors: list[Tensor]
    operators: list[Operator]
    inputs: list[int]
    outputs: list[int]


def _options(op: str, t: _Table | None) -> dict[str, Any]:
    """The options table of an operator, by its type (field order from schema.fbs)."""
    if t is None:
        return {}
    if op == "CONV_2D":
        return {"padding": PADDING[t.scalar(0, "b")], "stride_w": t.scalar(1, "i"), "stride_h": t.scalar(2, "i"),
                "activation": ACTIVATION[t.scalar(3, "b")], "dilation_w": t.scalar(4, "i", 1),
                "dilation_h": t.scalar(5, "i", 1)}
    if op == "DEPTHWISE_CONV_2D":
        return {"padding": PADDING[t.scalar(0, "b")], "stride_w": t.scalar(1, "i"), "stride_h": t.scalar(2, "i"),
                "depth_multiplier": t.scalar(3, "i"), "activation": ACTIVATION[t.scalar(4, "b")],
                "dilation_w": t.scalar(5, "i", 1), "dilation_h": t.scalar(6, "i", 1)}
    if op in ("MAX_POOL_2D", "AVERAGE_POOL_2D"):
        return {"padding": PADDING[t.scalar(0, "b")], "stride_w": t.scalar(1, "i"), "stride_h": t.scalar(2, "i"),
                "filter_w": t.scalar(3, "i"), "filter_h": t.scalar(4, "i"), "activation": ACTIVATION[t.scalar(5, "b")]}
    if op in ("ADD", "MUL", "SUB"):
        return {"activation": ACTIVATION[t.scalar(0, "b")]}
    if op == "CONCATENATION":
        return {"axis": t.scalar(0, "i"), "activation": ACTIVATION[t.scalar(1, "b")]}
    if op == "FULLY_CONNECTED":
        return {"activation": ACTIVATION[t.scalar(0, "b")], "keep_num_dims": bool(t.scalar(2, "B"))}
    if op == "RESHAPE":
        return {"new_shape": t.scalars(0, "i")}
    if op in ("RESIZE_BILINEAR", "RESIZE_NEAREST_NEIGHBOR"):
        return {"align_corners": bool(t.scalar(2, "B")), "half_pixel_centers": bool(t.scalar(3, "B"))}
    if op == "MEAN":
        return {"keep_dims": bool(t.scalar(0, "B"))}
    if op == "TRANSPOSE_CONV":
        return {"padding": PADDING[t.scalar(0, "b")], "stride_w": t.scalar(1, "i"), "stride_h": t.scalar(2, "i"),
                "activation": ACTIVATION[t.scalar(3, "b")]}
    return {}


def load(data: bytes) -> Model:
    """The main subgraph of a .tflite file, with every constant tensor read (float16 ones as float32)."""
    if data[4:8] != b"TFL3":
        raise ValueError("not a TensorFlow Lite model")
    root = _Table(data, struct.unpack_from("<I", data, 0)[0])
    codes = []
    for oc in root.tables(1):
        code = max(oc.scalar(0, "b"), oc.scalar(3, "i"))
        codes.append(OPS.get(code, f"op{code}") if not oc.string(1) else "custom:" + oc.string(1))
    buffers = []
    for b in root.tables(4):
        raw = b.raw(0)
        if not raw and b.scalar(1, "Q"):            # stored after the FlatBuffer, for big models
            off, size = b.scalar(1, "Q"), b.scalar(2, "Q")
            raw = bytes(data[off:off + size])
        buffers.append(raw)
    sub = root.tables(2)[0]
    tensors = []
    for t in sub.tables(0):
        shape = tuple(t.scalars(0, "i"))
        dtype = DTYPES.get(t.scalar(1, "b"), np.float32)
        raw = buffers[t.scalar(2, "I")] if t.scalar(2, "I") < len(buffers) else b""
        arr = None
        if raw:
            arr = np.frombuffer(raw, dtype=dtype).reshape(shape) if shape else np.frombuffer(raw, dtype=dtype)
            if arr.dtype == np.float16:
                arr = arr.astype(np.float32)
        q = t.table(4)
        tensors.append(Tensor(t.string(3), shape, dtype, arr, q.scalars(2, "f") if q else [],
                              q.scalars(3, "q") if q else []))
    ops = []
    for o in sub.tables(3):
        name = codes[o.scalar(0, "I")]
        ops.append(Operator(name, o.scalars(1, "i"), o.scalars(2, "i"), _options(name, o.table(4))))
    return Model(tensors, ops, sub.scalars(1, "i"), sub.scalars(2, "i"))

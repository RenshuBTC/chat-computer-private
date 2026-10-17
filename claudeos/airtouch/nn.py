"""Runs a TensorFlow Lite network (read by tflite.py) with numpy: the hand models' layers, nothing else.

Tensors are NHWC float32 with a batch of one. Each operator is turned into a small function once, when the
network is loaded (weights rearranged for numpy's matrix multiply), so running it is a list of calls. The
arithmetic follows TFLite's reference kernels: SAME padding puts the odd pixel after, bilinear resizing uses
its sample positions.
"""
from __future__ import annotations

from typing import Any, Callable

import numpy as np
from numpy.lib.stride_tricks import sliding_window_view

from .tflite import Model, Operator


def _activate(x: np.ndarray, act: str | None) -> np.ndarray:
    if act is None:
        return x
    if act == "RELU":
        return np.maximum(x, 0, out=x)
    if act == "RELU6":
        return np.clip(x, 0, 6, out=x)
    if act == "RELU_N1_TO_1":
        return np.clip(x, -1, 1, out=x)
    if act == "TANH":
        return np.tanh(x, out=x)
    raise ValueError(f"activation {act}")


def _same_pad(size: int, k: int, stride: int, dilation: int = 1) -> tuple[int, int]:
    eff = (k - 1) * dilation + 1
    out = -(-size // stride)
    total = max((out - 1) * stride + eff - size, 0)
    return total // 2, total - total // 2


def _pad_hw(x: np.ndarray, kh: int, kw: int, sh: int, sw: int, padding: str, value: float = 0.0) -> np.ndarray:
    """x is (H, W, C). Padded for SAME (TFLite: the odd pixel goes after); as it is for VALID."""
    if padding != "SAME":
        return x
    t, b = _same_pad(x.shape[0], kh, sh)
    left, r = _same_pad(x.shape[1], kw, sw)
    if not (t or b or left or r):
        return x
    return np.pad(x, ((t, b), (left, r), (0, 0)), constant_values=value)


class Net:
    def __init__(self, model: Model) -> None:
        self.model = model
        self.consts: dict[int, np.ndarray] = {i: t.data for i, t in enumerate(model.tensors) if t.data is not None}
        self.steps: list[tuple[Callable[..., np.ndarray], list[int], int]] = []
        for op in model.operators:
            if op.op == "DEQUANTIZE" and op.inputs[0] in self.consts:
                # Weights stored as float16 and widened when the model runs: widened once, here (tflite.py
                # already made them float32).
                self.consts[op.outputs[0]] = self.consts[op.inputs[0]]
                continue
            self.steps.append((self._compile(op), [i for i in op.inputs if i >= 0], op.outputs[0]))

    def __call__(self, x: np.ndarray) -> list[np.ndarray]:
        vals: dict[int, np.ndarray] = dict(self.consts)
        vals[self.model.inputs[0]] = np.asarray(x, dtype=np.float32)
        for fn, ins, out in self.steps:
            vals[out] = fn(*(vals[i] for i in ins))
        return [vals[i] for i in self.model.outputs]

    # ---------------------------------------------------------------- operators
    def _const(self, i: int) -> np.ndarray:
        if i not in self.consts:
            raise ValueError(f"tensor {self.model.tensors[i].name} should be a constant")
        return self.consts[i]

    def _compile(self, op: Operator) -> Callable[..., np.ndarray]:
        o, kind = op.options, op.op
        if kind == "DEQUANTIZE":                 # float16 weights were made float32 when read
            return lambda x: x
        if kind == "CONV_2D":
            return self._conv(op)
        if kind == "DEPTHWISE_CONV_2D":
            return self._depthwise(op)
        if kind == "ADD":
            return lambda a, b: _activate(np.add(a, b), o.get("activation"))
        if kind == "SUB":
            return lambda a, b: _activate(np.subtract(a, b), o.get("activation"))
        if kind == "MUL":
            return lambda a, b: _activate(np.multiply(a, b), o.get("activation"))
        if kind == "PRELU":
            def prelu(x: np.ndarray, alpha: np.ndarray) -> np.ndarray:
                # x + (alpha - 1) * min(x, 0): alpha * x below zero, x above. Seven times quicker than np.where.
                y = np.minimum(x, 0)
                y *= alpha - 1
                y += x
                return y
            return prelu
        if kind == "RELU":
            return lambda x: np.maximum(x, 0)
        if kind == "RELU6":
            return lambda x: np.clip(x, 0, 6)
        if kind == "LOGISTIC":
            return lambda x: 1.0 / (1.0 + np.exp(-x))
        if kind == "TANH":
            return np.tanh
        if kind == "MAX_POOL_2D":
            return self._pool(op, np.max, -np.inf)
        if kind == "AVERAGE_POOL_2D":
            return self._pool(op, np.mean, 0.0)
        if kind == "PAD":
            pads = self._const(op.inputs[1])
            width = [tuple(int(v) for v in p) for p in pads]
            return lambda x, _p: np.pad(x, width)
        if kind == "RESHAPE":
            shape = [int(v) for v in (self._const(op.inputs[1]) if len(op.inputs) > 1 and op.inputs[1] >= 0
                                      else o["new_shape"])]
            return lambda x, *_: x.reshape(shape)
        if kind == "CONCATENATION":
            axis = o["axis"]
            return lambda *xs: _activate(np.concatenate(xs, axis=axis), o.get("activation"))
        if kind == "MEAN":
            axes = tuple(int(a) for a in np.atleast_1d(self._const(op.inputs[1])))
            return lambda x, _a: np.mean(x, axis=axes, keepdims=o.get("keep_dims", False))
        if kind == "FULLY_CONNECTED":
            w = np.ascontiguousarray(self._const(op.inputs[1]).T)
            b = self.consts.get(op.inputs[2]) if len(op.inputs) > 2 and op.inputs[2] >= 0 else None
            act = o.get("activation")

            def fc(x: np.ndarray, *_: Any) -> np.ndarray:
                y = x.reshape(-1, w.shape[0]) @ w
                if b is not None:
                    y += b
                return _activate(y, act)
            return fc
        if kind == "RESIZE_BILINEAR":
            return self._resize(op)
        raise NotImplementedError(f"the hand model uses {kind}, which isn't supported")

    def _conv(self, op: Operator) -> Callable[..., np.ndarray]:
        o = op.options
        w = self._const(op.inputs[1])                    # (Cout, kh, kw, Cin)
        cout, kh, kw, cin = w.shape
        b = self.consts.get(op.inputs[2]) if len(op.inputs) > 2 and op.inputs[2] >= 0 else None
        sh, sw, act = o["stride_h"], o["stride_w"], o.get("activation")
        if o.get("dilation_h", 1) != 1 or o.get("dilation_w", 1) != 1:
            raise NotImplementedError("dilated convolution")
        if kh == kw == 1 and sh == sw == 1:
            wm = np.ascontiguousarray(w.reshape(cout, cin).T)

            def pointwise(x: np.ndarray, *_: Any) -> np.ndarray:
                n, h, wd, _c = x.shape
                y = x.reshape(h * wd, cin) @ wm
                if b is not None:
                    y += b
                return _activate(y, act).reshape(1, h, wd, cout)
            return pointwise
        wm = np.ascontiguousarray(w.transpose(1, 2, 3, 0).reshape(kh * kw * cin, cout))
        padding = o["padding"]

        def conv(x: np.ndarray, *_: Any) -> np.ndarray:
            xp = _pad_hw(x[0], kh, kw, sh, sw, padding)
            win = sliding_window_view(xp, (kh, kw), axis=(0, 1))[::sh, ::sw]   # (H', W', Cin, kh, kw)
            h, wd = win.shape[:2]
            cols = win.transpose(0, 1, 3, 4, 2).reshape(h * wd, kh * kw * cin)
            y = cols @ wm
            if b is not None:
                y += b
            return _activate(y, act).reshape(1, h, wd, cout)
        return conv

    def _depthwise(self, op: Operator) -> Callable[..., np.ndarray]:
        o = op.options
        w = self._const(op.inputs[1])                    # (1, kh, kw, C * depth_multiplier)
        if o.get("depth_multiplier", 1) not in (0, 1):
            raise NotImplementedError("depth multiplier above one")
        _, kh, kw, c = w.shape
        wk = np.ascontiguousarray(w[0])                  # (kh, kw, C)
        b = self.consts.get(op.inputs[2]) if len(op.inputs) > 2 and op.inputs[2] >= 0 else None
        sh, sw, act, padding = o["stride_h"], o["stride_w"], o.get("activation"), o["padding"]

        def depthwise(x: np.ndarray, *_: Any) -> np.ndarray:
            xp = _pad_hw(x[0], kh, kw, sh, sw, padding)
            win = sliding_window_view(xp, (kh, kw), axis=(0, 1))[::sh, ::sw]   # (H', W', C, kh, kw)
            y = np.einsum("hwcij,ijc->hwc", win, wk)
            if b is not None:
                y += b
            return _activate(y, act)[None]
        return depthwise

    def _pool(self, op: Operator, reduce: Callable[..., np.ndarray], fill: float) -> Callable[..., np.ndarray]:
        o = op.options
        kh, kw, sh, sw = o["filter_h"], o["filter_w"], o["stride_h"], o["stride_w"]

        def pool(x: np.ndarray) -> np.ndarray:
            xp = _pad_hw(x[0], kh, kw, sh, sw, o["padding"], fill)
            win = sliding_window_view(xp, (kh, kw), axis=(0, 1))[::sh, ::sw]
            return _activate(reduce(win, axis=(3, 4)), o.get("activation"))[None]
        return pool

    def _resize(self, op: Operator) -> Callable[..., np.ndarray]:
        o = op.options
        size = [int(v) for v in self._const(op.inputs[1])]
        align, half = o.get("align_corners", False), o.get("half_pixel_centers", False)

        def axis(out: int, inp: int) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
            # TFLite's reference: the sample position, its two neighbours (clamped), and the weight.
            scale = (inp - 1) / (out - 1) if align and out > 1 else inp / out
            pos = (np.arange(out) + 0.5) * scale - 0.5 if half else np.arange(out) * scale
            lo = np.maximum(np.floor(pos), 0).astype(np.int64)
            hi = np.minimum(np.ceil(pos), inp - 1).astype(np.int64)
            return lo, hi, (pos - lo).astype(np.float32)

        def resize(x: np.ndarray, *_: Any) -> np.ndarray:
            img = x[0]
            y0, y1, fy = axis(size[0], img.shape[0])
            x0, x1, fx = axis(size[1], img.shape[1])
            fy, fx = fy[:, None, None], fx[None, :, None]
            top = img[y0][:, x0] * (1 - fx) + img[y0][:, x1] * fx
            bottom = img[y1][:, x0] * (1 - fx) + img[y1][:, x1] * fx
            return (top * (1 - fy) + bottom * fy)[None].astype(np.float32)
        return resize

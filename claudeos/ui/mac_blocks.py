"""Calling the blocks WebKit hands to ClaudeOS's delegate methods on a Mac (decision and completion handlers).

WebKit makes these blocks without a type signature PyObjC can read, and PyObjC won't call a block without
one: "cannot call block without a signature". On GitHub's Macs that made ClaudeOS quit as soon as its first page
started loading (Apple silicon), or left every page loading forever (Intel). So when PyObjC can't call the block,
it's called the way C calls a block: its invoke function, with the block itself as the first argument. (Giving
the block a signature through __block_signature__ was tried there too; PyObjC 12 refuses a plain encoding.)

No AppKit here, so the tests can run this on any computer with a pretend block.
"""
from __future__ import annotations

import ctypes
import logging
from typing import Any, Callable

log = logging.getLogger("claudeos.ui")


class _BlockLayout(ctypes.Structure):
    """The start of every block in memory (Apple's Block ABI): isa, flags, reserved, invoke, descriptor."""
    _fields_ = [("isa", ctypes.c_void_p), ("flags", ctypes.c_int32), ("reserved", ctypes.c_int32),
                ("invoke", ctypes.c_void_p), ("descriptor", ctypes.c_void_p)]


def _object_id(obj: Any) -> int:
    import objc  # type: ignore
    return int(objc.pyobjc_id(obj))


_said = False


def call_block(block: Any, signature: bytes, *args: Any, object_id: Callable[[Any], int] = _object_id) -> str:
    """Calls block(*args). signature is the block's type encoding, for the record: b"v@?q" returns nothing and
    takes one NSInteger, b"v@?@" takes one object. For the direct call, whole numbers go as C longs (NSInteger)
    and anything else as an Objective-C object. Returns how the block was called: "pyobjc" or "direct"."""
    global _said
    try:
        block(*args)
        return "pyobjc"
    except TypeError as exc:
        if "signature" not in str(exc):
            raise
        if not _said:
            _said = True
            log.info("calling WebKit's blocks directly (%s)", exc)
    ptr = object_id(block)
    layout = _BlockLayout.from_address(ptr)
    types: list[Any] = [ctypes.c_void_p]
    values: list[Any] = [ctypes.c_void_p(ptr)]
    for a in args:
        if isinstance(a, int):
            types.append(ctypes.c_long)
            values.append(ctypes.c_long(int(a)))
        else:
            types.append(ctypes.c_void_p)
            values.append(ctypes.c_void_p(object_id(a)))
    ctypes.CFUNCTYPE(None, *types)(layout.invoke)(*values)
    return "direct"

"""The hidden layer: how ClaudeOS talks to the machine (Wi-Fi, Bluetooth, sound, power,
screen input). Linux is the engine next to Windows, `MacPlatform` the one on a Mac; `FakePlatform`
stands in for tests and previews.
"""
from __future__ import annotations

import os
import sys

from .base import Platform, CommandResult


def get_platform(name: str | None = None) -> Platform:
    name = name or os.environ.get("CLAUDEOS_PLATFORM") or ("mac" if sys.platform == "darwin" else "linux")
    if name == "fake":
        from .fake import FakePlatform
        return FakePlatform()
    if name == "mac":
        from .mac import MacPlatform
        return MacPlatform()
    from .linux import LinuxPlatform
    return LinuxPlatform()


__all__ = ["Platform", "CommandResult", "get_platform"]

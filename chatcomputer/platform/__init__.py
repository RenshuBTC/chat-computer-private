"""The hidden layer: how Chat Computer talks to the machine (Wi-Fi, Bluetooth, sound, power,
screen input). Chat Computer is Linux underneath on every computer: `LinuxPlatform` on a PC, next to Windows
(Ubuntu), and `MacPlatform` on an Apple silicon Mac, next to macOS (Fedora Asahi Remix). `FakePlatform`
stands in for tests and previews.
"""
from __future__ import annotations

import os

from .base import Platform, CommandResult


def get_platform(name: str | None = None) -> Platform:
    from .linux import is_apple_silicon
    name = name or os.environ.get("CHATCOMPUTER_PLATFORM") or ("mac" if is_apple_silicon() else "linux")
    if name == "fake":
        from .fake import FakePlatform
        return FakePlatform()
    if name == "mac":
        from .linux import MacPlatform
        return MacPlatform()
    from .linux import LinuxPlatform
    return LinuxPlatform()


__all__ = ["Platform", "CommandResult", "get_platform"]

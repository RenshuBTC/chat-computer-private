import os
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))


@pytest.fixture
def clean_env(tmp_path, monkeypatch):
    """A clean environment like the user's machine: no Claude/Anthropic variables from this
    build machine, and a private HOME."""
    for k in list(os.environ):
        if k.startswith(("CLAUDE", "ANTHROPIC", "CLAUDECODE")) or k in ("HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy"):
            monkeypatch.delenv(k, raising=False)
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("HOME", str(home))
    monkeypatch.setenv("XDG_DATA_HOME", str(home / ".local" / "share"))
    monkeypatch.setenv("NO_PROXY", "127.0.0.1,localhost")
    return home

import atexit
import os
import shutil
import sys
import tempfile
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

# One trash for the whole run, in a folder of its own, set here rather than per test: send2trash goes through GLib
# where it can, and GLib works out (and keeps) the trash folder the first time anything looks at it — which can be
# while a test module is imported. Every test moves HOME to a folder of its own; if XDG_DATA_HOME moved too, a file
# trashed by one test would land in the trash of whichever test went first, where undo (journal.find_in_trash) could
# not find it, and the suite passed or failed by the order its files ran in. A file's own path is what tells trashed
# copies apart, so one shared trash keeps every test's undo working. (test_speed.py moves it for its own test only.)
_XDG_DATA_HOME = tempfile.mkdtemp(prefix="chatcomputer-tests-xdg-")
os.environ["XDG_DATA_HOME"] = _XDG_DATA_HOME
atexit.register(lambda: shutil.rmtree(_XDG_DATA_HOME, ignore_errors=True))


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
    monkeypatch.setenv("NO_PROXY", "127.0.0.1,localhost")
    return home

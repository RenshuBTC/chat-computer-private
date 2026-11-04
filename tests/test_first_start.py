"""First start on a new install: the voice downloads itself, with no terminal needed."""
import asyncio

from chatcomputer.config import Paths, Settings
from chatcomputer.controller import Controller
from chatcomputer.platform.fake import FakePlatform
from chatcomputer.screen import RecordingScreen
from chatcomputer.voice import models as M


def _controller(tmp_path, monkeypatch):
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "ChatComputer"))
    paths = Paths().ensure()
    screen = RecordingScreen()
    c = Controller(paths, Settings(user_name="Sam Lee"), FakePlatform(), screen, voice=True)
    return c, screen, paths


def test_missing_voice_models_are_downloaded_then_voice_starts(tmp_path, monkeypatch):
    c, screen, paths = _controller(tmp_path, monkeypatch)
    assert c._voice_models_missing()
    attempts = []

    def fake_download(p, s, progress=print):
        attempts.append(1)
        if len(attempts) == 1:
            raise OSError("no internet")
        onnx, voices = M.kokoro_files(p)
        onnx.parent.mkdir(parents=True, exist_ok=True)
        onnx.write_bytes(b"x")
        voices.write_bytes(b"x")
        w = M.whisper_dir(p, s.stt_model)
        w.mkdir(parents=True, exist_ok=True)
        (w / "model.bin").write_bytes(b"x")

    started = []

    async def fake_start_voice():
        started.append(True)

    async def no_wait(_s):
        return None

    monkeypatch.setattr(M, "download", fake_download)
    monkeypatch.setattr(c, "_start_voice", fake_start_voice)
    monkeypatch.setattr(asyncio, "sleep", no_wait)

    async def run():
        c.loop = asyncio.get_running_loop()
        await c._fetch_voice_then_start()
    asyncio.run(run())

    assert len(attempts) == 2 and started == [True]
    notices = [e[1] for e in screen.events if e[0] == "notice"]
    assert any("one-time download" in str(n) for n in notices)
    assert any("need the internet" in str(n) for n in notices)
    assert not c._voice_models_missing()

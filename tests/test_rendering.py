"""How ClaudeOS's window draws pages: with the graphics chip when the system offers one for drawing,
otherwise with the processor (WebKit's GPU drawing crashes there and leaves a blank screen)."""
import pytest

gtk_app = pytest.importorskip("claudeos.ui.gtk_app")


def test_draws_with_the_processor_without_a_render_device(tmp_path):
    (tmp_path / "card0").touch()            # a display, but nothing to draw with (a virtual machine)
    env = {}
    assert gtk_app.choose_rendering(env, str(tmp_path)) == "cpu"
    assert env == {"WEBKIT_SKIA_ENABLE_CPU_RENDERING": "1", "WEBKIT_DISABLE_DMABUF_RENDERER": "1", "CLAUDEOS_GPU": "0"}


def test_uses_the_graphics_chip_when_there_is_one(tmp_path):
    (tmp_path / "card0").touch()
    (tmp_path / "renderD128").touch()
    env = {}
    assert gtk_app.choose_rendering(env, str(tmp_path), nvidia=str(tmp_path / "no-nvidia")) == "gpu"
    assert env == {}


def test_nvidia_driver_uses_the_older_way_of_passing_pictures(tmp_path):
    (tmp_path / "renderD128").touch()
    (tmp_path / "nvidia_drm").mkdir()
    env = {}
    assert gtk_app.choose_rendering(env, str(tmp_path), nvidia=str(tmp_path / "nvidia_drm")) == "gpu"
    assert env == {"WEBKIT_DISABLE_DMABUF_RENDERER": "1"}


def test_the_graphics_chip_can_be_turned_off(tmp_path):
    (tmp_path / "renderD128").touch()
    env = {"CLAUDEOS_GPU": "0"}
    assert gtk_app.choose_rendering(env, str(tmp_path), nvidia=str(tmp_path / "no-nvidia")) == "cpu"
    assert env["WEBKIT_SKIA_ENABLE_CPU_RENDERING"] == "1"


def test_switching_to_the_processor_reports_a_change_once():
    env = {"CLAUDEOS_GPU": "1"}
    assert gtk_app.use_cpu_rendering(env) is True
    assert gtk_app.use_cpu_rendering(env) is False
    assert env["CLAUDEOS_GPU"] == "0"


@pytest.mark.skipif(not __import__("shutil").which("xvfb-run"), reason="xvfb-run not available")
def test_streaming_keeps_whole_messages_and_older_ones_come_back_when_there_is_room():
    import json
    import subprocess
    import sys
    from pathlib import Path
    script = Path(__file__).with_name("screen_fit_check.py")
    r = subprocess.run(["xvfb-run", "-a", "-s", "-screen 0 1280x800x24", sys.executable, str(script)],
                       capture_output=True, text=True, timeout=120)
    res = json.loads(r.stdout.strip().splitlines()[-1])
    assert "timeout" not in res, res

    def check(state):
        gone = [i["gone"] for i in state["items"]]
        shown = [i for i in state["items"] if not i["gone"]]
        # The hidden ones are the oldest, and nothing shown starts above the room kept at the top.
        assert gone == sorted(gone, reverse=True) and shown
        assert min(i["top"] for i in shown) >= state["room"] - 1
        return gone.count(True)
    filled, streamed, smaller = (check(res[k]) for k in ("filled", "streamed", "smaller"))
    assert 0 < filled < streamed            # the long reply pushed more of the old ones off
    assert smaller < streamed               # smaller text: some came back


@pytest.mark.skipif(not __import__("shutil").which("xvfb-run"), reason="xvfb-run not available")
def test_the_conversation_comes_back_after_its_renderer_stops():
    import json
    import subprocess
    import sys
    from pathlib import Path
    script = Path(__file__).with_name("screen_recovery_check.py")
    r = subprocess.run(["xvfb-run", "-a", "-s", "-screen 0 1280x800x24", sys.executable, str(script)],
                       capture_output=True, text=True, timeout=120)
    res = json.loads(r.stdout.strip().splitlines()[-1])
    assert "Sunny, 31 degrees." in res["before"]
    assert res["after"] == res["before"]
    assert res["ready_again"] and res["drawing_with_processor"]

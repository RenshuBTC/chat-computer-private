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
def test_the_conversation_reads_and_scrolls_like_a_chat():
    import json
    import subprocess
    import sys
    from pathlib import Path
    script = Path(__file__).with_name("screen_scroll_check.py")
    r = subprocess.run(["xvfb-run", "-a", "-s", "-screen 0 1280x800x24", sys.executable, str(script)],
                       capture_output=True, text=True, timeout=120)
    res = json.loads(r.stdout.strip().splitlines()[-1])
    assert "timeout" not in res, res
    filled, streamed, up, asked = res["filled"], res["streamed"], res["scrolled_up"], res["asked"]
    # Your messages on the right in a grey bubble, upright; Claude's on the left, on the page's white.
    for state in (filled, asked):
        assert abs(state["you"]["right"]) < 1 and state["you"]["left"] > 20
        assert state["you"]["background"] == "rgb(237, 237, 237)" and state["you"]["style"] == "normal"
        assert abs(state["ai"]["left"]) < 1 and state["ai"]["background"] == "rgba(0, 0, 0, 0)"
    # More than a screenful: it can be scrolled, and new words keep the newest at the bottom.
    assert filled["height"] > filled["view"] and filled["bottom"] < 2
    assert streamed["bottom"] < 2 and streamed["height"] > filled["height"]
    # Scrolled up to read: more words don't pull it down...
    assert up["top"] == 0 and up["height"] > streamed["height"]
    # ...until a new message, which brings it to the bottom.
    assert asked["bottom"] < 2


@pytest.mark.skipif(not __import__("shutil").which("xvfb-run") or not __import__("shutil").which("xdotool"),
                    reason="xvfb-run or xdotool not available")
def test_whats_done_takes_you_back_to_where_it_was_asked():
    # A panel at the right, a sixth of the screen as in the Claude app, grouped by day. Clicking an item (a real
    # click: the AirTouch layer over everything used to take every click) scrolls back to the message that asked
    # for it; one from yesterday puts yesterday's messages back first. Typing stays in the message box.
    import json
    import subprocess
    import sys
    from pathlib import Path
    script = Path(__file__).with_name("screen_done_check.py")
    r = subprocess.run(["xvfb-run", "-a", "-s", "-screen 0 1600x900x24", sys.executable, str(script)],
                       capture_output=True, text=True, timeout=120)
    res = json.loads(r.stdout.strip().splitlines()[-1])
    assert "timeout" not in res, res
    start, today, earlier = res["start"], res["today"], res["earlier"]
    assert start["panel"] and start["display"] == "flex" and start["title"] == "What’s done"
    assert abs(start["width"] - 0.17 * start["view"]) < 2 and abs(start["left"] + start["width"] - start["view"]) < 1
    assert start["column_right"] <= start["left"] and start["box"][1] <= start["left"]
    assert [(i["cls"], i["text"]) for i in start["items"]] == [
        ("done-day", "Yesterday"), ("done-item", "Opened youtube.com"), ("done-day", "Today"),
        ("done-item", "Searched the web for “trains on Sunday”"), ("done-item", "Saved notes.txt"),
        ("done-item", "Set the volume to 40%")]
    assert start["found"] is None and start["focused"]
    # Today's: scrolled up to it, and it's marked for a moment.
    assert today["found"]["id"] == "u5" and 0 <= today["found"]["top"] < 100 and today["scroll"] < start["scroll"]
    # Yesterday's: put back above today's (with the days), then scrolled to.
    assert res["asked"][0][0] == "y1" and res["asked"][0][2]
    assert earlier["found"]["id"] == "y1" and 0 <= earlier["found"]["top"] < 100
    assert earlier["messages"] == start["messages"] + 5 and earlier["days"] == ["Yesterday", "Today"]
    assert today["focused"] and earlier["focused"]


@pytest.mark.skipif(not __import__("shutil").which("xvfb-run"), reason="xvfb-run not available")
def test_the_conversation_comes_back_after_its_renderer_stops():
    import json
    import subprocess
    import sys
    from pathlib import Path
    script = Path(__file__).with_name("screen_recovery_check.py")
    r = subprocess.run(["xvfb-run", "-a", "-s", "-screen 0 1280x800x24", sys.executable, str(script)],
                       capture_output=True, text=True, timeout=120)
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "Sunny, 31 degrees." in res["before"]
    assert res["after"] == res["before"]
    assert res["ready_again"] and res["drawing_with_processor"]


@pytest.mark.skipif(not __import__("shutil").which("xvfb-run") or not __import__("shutil").which("xdotool"),
                    reason="xvfb-run or xdotool not available")
def test_the_message_box_works_like_the_claude_apps():
    # A real text box at the bottom, typed into with real key presses: Shift+Enter starts a new line and the box
    # grows, Enter sends it all, Escape clears it, and typing after a click elsewhere still goes into it. Each
    # letter's time to reach the screen is reported (logged once a minute, see GtkScreen._watch_speed).
    import json
    import subprocess
    import sys
    from pathlib import Path
    script = Path(__file__).with_name("screen_typing_check.py")
    r = subprocess.run(["xvfb-run", "-a", "-s", "-screen 0 1280x800x24", sys.executable, str(script)],
                       capture_output=True, text=True, timeout=120)
    res = json.loads(r.stdout.strip().splitlines()[-1])
    assert "timeout" not in res, res
    empty, typed, two, sent = res["empty"], res["typed"], res["two_lines"], res["sent"]
    assert empty["focused"] and empty["value"] == "" and empty["send"] == "hidden"
    assert empty["status"] == "Listening Sonnet 5.5"
    assert typed["value"] == "hi there" and typed["drafting"] and typed["send"] == "visible"
    assert two["value"] == "hi there\nsecond line" and two["box_h"] > typed["box_h"] + 20
    assert res["sent_text"] == ["hi there\nsecond line"]
    assert sent["value"] == "" and sent["box_h"] == typed["box_h"] and not sent["drafting"]
    assert res["after_blur"]["value"] == "abc" and res["after_blur"]["focused"]
    assert res["escaped"]["value"] == ""
    assert len(res["typed_ms"]) >= 15 and all(0 <= ms < 2000 for ms in res["typed_ms"])


def _node():
    import shutil
    return shutil.which("node") or next((p for p in ("/opt/node22/bin/node",) if __import__("os").path.exists(p)), None)


@pytest.mark.skipif(_node() is None, reason="node not available")
def test_streamed_words_are_drawn_as_the_whole_message_would_be():
    # chat.js draws the finished blocks of a message once and only the block still being written again; at every
    # moment of a stream that must look exactly like the whole message drawn at once.
    import json
    import subprocess
    from pathlib import Path
    here = Path(__file__).parent
    assets = here.parent / "claudeos" / "ui" / "assets"
    r = subprocess.run([_node(), str(here / "md_stream_check.js"), str(assets / "md.js"), str(assets / "chat.js")],
                       capture_output=True, text=True, timeout=60)
    res = json.loads(r.stdout.strip().splitlines()[-1])
    assert res["checked"] > 900 and res["bad"] == 0, res["examples"]
    assert res["ol"].startswith('<ol><li>one</li></ol><ol start="2"><li>two</li></ol><ol start="3">')

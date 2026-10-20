"""How Chat Computer's window draws pages: with the graphics chip when the system offers one for drawing,
otherwise with the processor (WebKit's GPU drawing crashes there and leaves a blank screen)."""
import pytest

gtk_app = pytest.importorskip("chatcomputer.ui.gtk_app")


def test_draws_with_the_processor_without_a_render_device(tmp_path):
    (tmp_path / "card0").touch()            # a display, but nothing to draw with (a virtual machine)
    env = {}
    assert gtk_app.choose_rendering(env, str(tmp_path)) == "cpu"
    assert env == {"WEBKIT_SKIA_ENABLE_CPU_RENDERING": "1", "WEBKIT_DISABLE_DMABUF_RENDERER": "1", "CHATCOMPUTER_GPU": "0"}


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
    env = {"CHATCOMPUTER_GPU": "0"}
    assert gtk_app.choose_rendering(env, str(tmp_path), nvidia=str(tmp_path / "no-nvidia")) == "cpu"
    assert env["WEBKIT_SKIA_ENABLE_CPU_RENDERING"] == "1"


def test_switching_to_the_processor_reports_a_change_once():
    env = {"CHATCOMPUTER_GPU": "1"}
    assert gtk_app.use_cpu_rendering(env) is True
    assert gtk_app.use_cpu_rendering(env) is False
    assert env["CHATCOMPUTER_GPU"] == "0"


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
    import re
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
    # A rounded card, inset from the screen's edge, in the right sixth of the screen.
    rem = (start["view"] - start["left"] - start["width"]) / 0.75           # the card stands 0.75 rem from the edge
    assert 14 < rem < 40 and abs(start["width"] - (0.17 * start["view"] - rem)) < 2 and start["radius"].endswith("px")
    assert float(start["radius"][:-2]) > 10
    assert start["column_right"] <= start["left"] and start["box"][1] <= start["left"]
    assert [(i["cls"], i["text"]) for i in start["items"]] == [
        ("side-day", "Yesterday"), ("done-item", "Opened youtube.com"), ("side-day", "Today"),
        ("done-item", "Searched the web for “trains on Sunday”"), ("done-item", "Saved notes.txt"),
        ("done-item", "Set the volume to 40%")]
    # Each thing done says when, and under it what (the words get the panel's width); the date and the time now
    # are above them. No line down the side.
    done = [i for i in start["items"] if i["cls"] == "done-item"]
    assert all(re.fullmatch(r"[1-9]\d?:\d{2}( [AP]M)?", i["when"]) and i["stacked"] for i in done)
    assert re.search(r"\d{4}$", start["clock"][0]) and re.fullmatch(r"[1-9]\d?:\d{2}( [AP]M)?", start["clock"][1])
    assert start["line"] == "none"
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
    # The cursor in the box from the start; the round send button grey until there's something to send, then black.
    assert empty["focused"] and empty["value"] == "" and empty["send"] == "rgb(154, 154, 154)"
    # Nothing under the box (or above it while awake), and in it just the dimmed hint.
    assert empty["words"] == "Chat computer anything or type \u201c/\u201d to search" and empty["wake"] == "none" and empty["below_box"] < 20 and empty["above_box"] < 14
    assert typed["value"] == "hi there" and typed["drafting"] and typed["send"] == "rgb(0, 0, 0)"
    assert two["value"] == "hi there\nsecond line" and two["box_h"] > typed["box_h"] + 20
    assert res["sent_text"] == ["hi there\nsecond line"]
    assert sent["value"] == "" and sent["box_h"] == typed["box_h"] and not sent["drafting"]
    assert res["after_blur"]["value"] == "abc" and res["after_blur"]["focused"]
    assert res["escaped"]["value"] == ""
    assert len(res["typed_ms"]) >= 15 and all(0 <= ms < 2000 for ms in res["typed_ms"])



@pytest.mark.skipif(not __import__("shutil").which("xvfb-run") or not __import__("shutil").which("xdotool"),
                    reason="xvfb-run or xdotool not available")
def test_what_is_open_sits_between_the_sidebars_and_f1_and_escape_work():
    # Messages at the left and What's done at the right, as rounded cards inset from the screen's edges. Something
    # opened (a website) goes in a rounded frame between them, the same space apart, above your latest message,
    # Claude's latest reply and the message box. F1 makes it fill the screen and F1 again puts it back; Escape
    # closes it. A message can be pinned (it goes to the top) and opened, with real clicks.
    import json
    import subprocess
    import sys
    from pathlib import Path
    script = Path(__file__).with_name("screen_layout_check.py")
    r = subprocess.run(["xvfb-run", "-a", "-s", "-screen 0 1920x1080x24", sys.executable, str(script)],
                       capture_output=True, text=True, timeout=150)
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "timeout" not in res, res
    start, pinned, content, expanded, framed, closed = (res[k] for k in ("start", "pinned", "content", "expanded",
                                                                         "framed", "closed"))
    w, h = start["view"]
    gap = start["messages"][0]                                  # the cards' space from the screen's edges
    assert 10 < gap < 30 and start["messages"][1] == gap and h - start["messages"][3] == gap
    assert w - start["done"][2] == gap and float(start["radius"][:-2]) > 10
    # Newest first; pinned ones above, without being cut off; each with its kind's icon.
    assert [i["who"] for i in start["list"] if "msg" in i["cls"]] == ["Mum", "Jane from the bank", "Alex"]
    assert [i["who"] for i in start["pinned"]] == ["sam.k"]
    assert [i["who"] for i in pinned["pinned"]] == ["Mum", "sam.k"] and [i["icon"] for i in pinned["pinned"]] == [
        "#i-chat", "#i-social"]
    assert not any(i["clipped"] or i["title"] for i in pinned["pinned"] + pinned["list"])
    assert res["events"] == [{"type": "pin", "list": "messages", "id": "m2", "on": True},
                             {"type": "open_message", "id": "m1"}]
    # Open: the page is inside the frame, which is between the cards (the same space apart) and above the latest
    # exchange; your latest message shows just above the message box.
    assert content["mode"] == "content" and content["content"]
    f, v = content["frame"], content["content_view"]
    assert abs(f[0] - (start["messages"][2] + gap)) <= 1 and abs(f[2] - (start["done"][0] - gap)) <= 1
    assert abs(f[1] - gap) <= 1 and f[0] < v[0] and f[1] < v[1] and v[2] < f[2] and v[3] < f[3]
    assert f[3] < content["column"][1] <= content["last_you"][1] and content["last_you"][3] <= content["column"][3]
    # There, your latest message and Claude's latest reply both show in full (no "\u2026 more"): the page above leaves
    # them the room. The empty box says, dimmed, "Chat computer anything or type “/” to search".
    assert content["more"] == 0 and content["ai_h"] > 5 * content["line"] and content["ai_bottom"] <= content["column"][3] + 1
    assert content["hint"] == "Chat computer anything or type \u201c/\u201d to search"
    # "/": its list shows in front. The window lays the page over this one, so the page ends above the list; it
    # comes back down when the search ends.
    sl = res["slash"]
    assert sl["searching"] and sl["results"][3] - sl["results"][1] > 2 * sl["line"] and sl["hint"] is None
    assert sl["content_view"][3] <= sl["results"][1] and sl["content_view"][1] == v[1]
    assert not res["unslashed"]["searching"] and res["unslashed"]["content_view"] == v
    # The panels' switches, each in its corner (the conversation's only while something is open), filled in while
    # their panel shows. Both sidebars off: the page spreads into their room, leaving a narrow margin with each switch.
    tl, tr, tb = (content["toggles"][k] for k in ("left", "right", "bottom"))
    assert all(x["shown"] and x["pressed"] == "true" for x in (tl, tr, tb)) and not start["toggles"]["bottom"]["shown"]
    assert tl["x"] < start["messages"][0] + 60 and tr["x"] > start["done"][2] - 60 and tb["y"] > h - 60
    off = res["sides_off"]
    assert off["sides"] == [False, False] and off["toggles"]["left"]["pressed"] == "false"
    ov = off["content_view"]
    assert ov[0] < v[0] - 100 and ov[2] > v[2] + 100 and ov[0] > tl["x"] and ov[2] < tr["x"]
    # The conversation off: the page reaches down to its switch; typing brings the conversation back.
    bo = res["bottom_off"]
    assert not bo["composer"] and bo["content_view"][3] > ov[3] + 50 and bo["content_view"][3] < tb["y"]
    tb2 = res["typed_back"]
    assert tb2["composer"] and tb2["toggles"]["bottom"]["pressed"] == "true" and tb2["content_view"] == ov
    assert res["sides_on"]["sides"] == [True, True] and res["sides_on"]["content_view"] == v
    assert [(e["left"], e["right"], e["bottom"]) for e in res["panel_events"]] == [
        (False, True, True), (False, False, True), (False, False, False), (False, False, True)]
    # F1: the whole screen; F1 again: back in the frame. Escape: closed, back to the conversation.
    assert expanded["content_view"] == [0, 0, w, h]
    assert framed["content_view"] == v and framed["mode"] == "content"
    assert closed["mode"] == "chat" and closed["content_view"] == [] and not closed["content"] and res["closed_called"]



@pytest.mark.skipif(not __import__("shutil").which("xvfb-run") or not __import__("shutil").which("xdotool"),
                    reason="xvfb-run or xdotool not available")
def test_slash_in_the_message_box_searches_this_computer():
    # Chat Computer is the conversation: the box has no menu (a right-click opens nothing). A "/" first turns it into a
    # search of this computer's files: the matches in a list above the box (the matching letters bold), the arrow
    # keys choose, Enter opens one; nothing found leaves the line that asks Claude; Escape clears it. Anything
    # else typed is a message. Real keys and clicks; the results come from the test, as the controller sends them.
    import json
    import subprocess
    import sys
    from pathlib import Path
    script = Path(__file__).with_name("screen_search_check.py")
    r = subprocess.run(["xvfb-run", "-a", "-s", "-screen 0 1600x900x24", sys.executable, str(script)],
                       capture_output=True, text=True, timeout=150)
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "timeout" not in res, res
    assert res["right_click"]["menus"] == 0 and not res["right_click"]["searching"]
    s = res["search"]
    assert s["searching"] and s["icon"] == "block" and s["value"] == "/rep"
    assert [r["text"] for r in s["results"]] == ["report.pdf~/Documents", "Ask Claude to find \u201crep\u201d"]
    assert s["bold"] == ["rep"] and "sel" in s["results"][0]["cls"] and s["results_box"][3] <= s["box"][1]
    assert "sel" in res["search_down"]["results"][1]["cls"] and "sel" in res["search_up"]["results"][0]["cls"]
    assert not res["opened"]["searching"] and res["opened"]["value"] == ""
    assert res["events"][:2] == [{"type": "search", "q": "rep"}, {"type": "open_file", "path": "/home/ren/Documents/report.pdf"}]
    nothing = res["nothing"]
    assert [r["text"] for r in nothing["results"]] == ["Ask Claude to find \u201czzq\u201d"]
    assert nothing["note"] == "Nothing on this computer matches \u201czzq\u201d."
    assert not res["cleared"]["searching"] and res["cleared"]["value"] == "" and res["cleared"]["icon"] == "none"
    assert res["typed"] == ["hello there"] and res["message"]["focused"]


@pytest.mark.skipif(not __import__("shutil").which("xvfb-run"), reason="xvfb-run not available")
def test_the_touchscreen_works_as_an_ipad_s():
    # The window's side (stand-ins for GTK's touch events: a virtual display has no touchscreen) and the
    # conversation page's (touch events made in the page).
    import json
    import subprocess
    import sys
    from pathlib import Path
    script = Path(__file__).with_name("screen_touch_check.py")
    r = subprocess.run(["xvfb-run", "-a", "-s", "-screen 0 1600x900x24", sys.executable, str(script)],
                       capture_output=True, text=True, timeout=120)
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "timeout" not in res, res
    # A tap in the middle is the page's; one at the very edge is held back a moment (a swipe in?), then the page's.
    assert res["tap"] == [False, False] and res["tap_delivered"] == []
    assert res["edge_tap"] == [True, False] and res["edge_tap_delivered"] == [[2, "touch-begin"]]
    # A third finger: the first two are cancelled on the page; three fingers pinched closed copy.
    assert res["third"] and res["cancelled"] == [[10, "touch-cancel"], [11, "touch-cancel"]] and res["edits"] == ["Copy"]
    # Swiping up from the bottom edge closes what's open (Home); the page never sees that swipe.
    assert res["before_home"] == "content" and res["after_home"] == "chat" and all(res["home"]) and res["closed"]
    # The onscreen keyboard: keys typed where the finger lifts (slid from w to e: e), its hide key, a real
    # keyboard's key puts it away (not one Chat Computer typed), and while a real keyboard is in use it doesn't come.
    assert res["keyboard_up"] and res["keyboard_spec"] and res["key_q"] == [True, True] and res["still_up"]
    assert res["keys"] == [["text", "q"], ["text", "e"]] and res["hidden_by_key"]
    assert res["up_after_xtest"] and not res["up_after_real_key"] and res["refused_with_real_keyboard"]
    # The page: a finger on the message box brings the keyboard, one elsewhere puts it away.
    assert res["box_touch_keyboard"] and not res["keyboard_after_touch_elsewhere"]
    # Two fingers pinched: the text grows as they move (not WebKit's zoom), and the new size is kept.
    p = res["pinch"]
    assert p["zoomBlocked"] and abs(float(p["size1"][:-2]) / float(p["size0"][:-2]) - 1.5) < 0.02
    assert res["page_events"] == [{"type": "text_scale", "factor": 1.5}]
    # A sidebar item swiped left shows Pin; holding a message selects it; a double tap selects a word.
    assert res["swipe"] == {**res["swipe"], "swiped": True, "label": '"Pin"', "visible": "visible"}
    assert res["hold"]["selected"].strip() == "Hi Ren. The weather is lovely today." and res["hold"]["tapPrevented"]
    assert res["double_tap"] == "weather"
    # Messages pulled down from the top on a narrow screen, and gone with a touch elsewhere.
    assert res["messages"] == {"shown": "flex", "hiddenAfter": True}

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
    assets = here.parent / "chatcomputer" / "ui" / "assets"
    r = subprocess.run([_node(), str(here / "md_stream_check.js"), str(assets / "md.js"), str(assets / "chat.js")],
                       capture_output=True, text=True, timeout=60)
    res = json.loads(r.stdout.strip().splitlines()[-1])
    assert res["checked"] > 900 and res["bad"] == 0, res["examples"]
    assert res["ol"].startswith('<ol><li>one</li></ol><ol start="2"><li>two</li></ol><ol start="3">')


@pytest.mark.skipif(not __import__("shutil").which("xvfb-run") or not __import__("shutil").which("xdotool"),
                    reason="xvfb-run or xdotool not available")
def test_the_lock_screen_covers_everything_and_only_takes_a_pin():
    # Locked: the time and a keypad over everything (the conversation, the sidebars, the message box). Typing goes
    # only into the PIN box, which keeps only digits; Enter sends the PIN to Chat Computer to check (the "unlock" page
    # event), never a message to Claude. The keypad types and deletes; a wrong PIN says so; unlocked, it all comes
    # back.
    import json
    import subprocess
    import sys
    from pathlib import Path
    script = Path(__file__).with_name("screen_lock_check.py")
    r = subprocess.run(["xvfb-run", "-a", "-s", "-screen 0 1280x800x24", sys.executable, str(script)],
                       capture_output=True, text=True, timeout=120)
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "timeout" not in res, res
    assert not res["before"]["locked"] and res["before"]["column"]
    lk = res["locked"]
    assert lk["locked"] and lk["lock_shown"] and not lk["column"] and not lk["composer"] and lk["pin_focused"]
    assert lk["time"]
    assert res["events"] == [{"type": "unlock", "pin": "24"}] and res["sent_text"] == []
    assert res["typed"]["box"] == "" and res["pad"]["pin"] == "137"
    assert res["wrong"]["error"] == "That's not the PIN." and res["wrong"]["pin"] == ""
    un = res["unlocked"]
    assert not un["locked"] and not un["lock_shown"] and un["column"] and un["composer"]


@pytest.mark.skipif(not __import__("shutil").which("xvfb-run"), reason="xvfb-run not available")
def test_pdfs_and_where_what_s_open_is():
    import json
    import subprocess
    import sys
    from pathlib import Path
    script = Path(__file__).with_name("screen_pdf_check.py")
    r = subprocess.run(["xvfb-run", "-a", "-s", "-screen 0 1280x800x24", sys.executable, str(script)],
                       capture_output=True, text=True, timeout=150)
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "timeout" not in res and "error" not in res, res
    # Nothing open: no place for it, and nothing to print.
    assert res["rect_chat"] is None and res["open_nothing"] == {"ok": False, "error": "Nothing is open on the screen."}
    # A document printed without being shown: its words, on the paper this country uses (A4 or Letter).
    assert res["doc"]["ok"] and res["doc_shown"] == "chat"
    d = res["doc_pdf"]
    assert d["exists"] and d["pages"] == 1 and d["size_pt"] in ([595, 842], [612, 792])
    assert "Ren Shu" in d["text"] and "Curriculum Vitae" in d["text"] and "Designer" in d["text"]
    # What's open: where it is on the screen, and printed as it is.
    x, y, w, h = res["rect_open"]
    sw, sh = res["screen"]
    assert 0 <= x and 0 <= y and 200 < w <= sw and 200 < h <= sh
    assert res["open"]["ok"] and "Packing list" in res["open_pdf"]["text"] and "Passport" in res["open_pdf"]["text"]

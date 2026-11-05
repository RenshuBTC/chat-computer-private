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
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]     # (the display's warnings can come after it)
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "timeout" not in res, res
    filled, streamed, up, asked = res["filled"], res["streamed"], res["scrolled_up"], res["asked"]
    # Your messages on the right in a bubble (Claude's dark bubble), upright; Claude's on the left, on the page's own dark.
    for state in (filled, asked):
        assert abs(state["you"]["right"]) < 1 and state["you"]["left"] > 20
        assert state["you"]["background"] == "rgb(37, 37, 37)" and state["you"]["style"] == "normal"
        assert abs(state["ai"]["left"]) < 1 and state["ai"]["background"] == "rgba(0, 0, 0, 0)"
    # More than a screenful: it can be scrolled. Your latest message is at the top of the screen with its answer under
    # it (as in the Claude app), and an answer longer than the screen is followed down as it comes.
    assert filled["height"] > filled["view"] and filled["bottom"] < 2
    assert abs(filled["you_top"] - filled["margin"]) < 3 and filled["ai_below"]
    assert streamed["bottom"] < 2 and streamed["height"] > filled["height"]
    # Scrolled up to read: more words don't pull it down...
    assert up["top"] == 0 and up["height"] > streamed["height"]
    # ...until a new message, which goes to the top of the screen: nothing of it is below.
    assert asked["bottom"] < 2 and abs(asked["you_top"] - asked["margin"]) < 3


@pytest.mark.skipif(not __import__("shutil").which("xvfb-run") or not __import__("shutil").which("xdotool"),
                    reason="xvfb-run or xdotool not available")
def test_whats_done_takes_you_back_to_where_it_was_asked():
    import re
    # The sidebars from the start. A panel at the right, a sixth of the screen as in the Claude
    # app, grouped by day. Clicking an item (a real click: the keyboard's layer over everything used to take every
    # click) scrolls back to the message that asked for it; one from yesterday puts yesterday's messages back first.
    # Typing stays in the message box.
    import json
    import subprocess
    import sys
    from pathlib import Path
    script = Path(__file__).with_name("screen_done_check.py")
    r = subprocess.run(["xvfb-run", "-a", "-s", "-screen 0 1600x900x24", sys.executable, str(script)],
                       capture_output=True, text=True, timeout=120)
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]     # (the display's warnings can come after it)
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "timeout" not in res, res
    start, today, earlier = res["start"], res["today"], res["earlier"]
    assert not res["first"]["full"] and res["first"]["panel"] and res["first"]["display"] == "flex"     # from the start
    assert not start["full"] and start["panel"] and start["display"] == "flex" and start["title"] == "Actions"
    # Down the right edge of the screen from top to bottom, 17.5 rem wide, as the Claude app's sidebar is.
    rem = start["rem"]
    assert 12 <= rem < 40 and abs(start["left"] + start["width"] - start["view"]) < 1
    assert abs(start["width"] - 17.5 * rem) < 2 and start["radius"] == "0px"
    assert start["column_right"] <= start["left"] and start["box"][1] <= start["left"]
    assert [(i["cls"], i["text"]) for i in start["items"]] == [
        ("side-day", "Yesterday"), ("done-item", "Opened youtube.com"), ("side-day", "Today"),
        ("done-item", "Searched the web for “trains on Sunday”"), ("done-item revertable", "Saved notes.txt"),
        ("done-item", "Set the volume to 40%")]
    # Each thing done says what, and its time at the right of it, on its first line; the date and the time now
    # are above them. No line down the list's side.
    done = [i for i in start["items"] if i["cls"] == "done-item"]
    assert all(re.fullmatch(r"[1-9]\d?:\d{2}( [AP]M)?", i["when"]) and i["beside"] for i in done)
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
    # Revert: only on the one that can be reverted, under the pointer, left of the pin.
    hover, reverting, reverted = res["hover"], res["reverting"], res["reverted"]
    assert hover["with_revert"] == ["d3"] and hover["revert"]["visible"] == "visible"
    assert hover["revert"]["right"] <= hover["pin"]["left"] + 1 and hover["struck"] == "none"
    # Clicked: the computer is asked, and the item says so where its time was (no Revert) until it answers...
    assert res["page_events"] == [{"type": "revert", "id": "d3"}]
    assert "reverting" in reverting["cls"] and reverting["revert"] is None and reverting["when"] == "Reverting\u2026"
    # ...then struck through (still white: no text is grey), its time back, and the revert listed last, which can be
    # reverted in turn.
    assert "reverted" in reverted["cls"] and "reverting" not in reverted["cls"] and reverted["struck"] == "line-through"
    assert reverted["colors"] == ["rgb(240, 240, 240)", "1"] and re.fullmatch(r"\d{1,2}:\d\d(\s?[AP]M)?", reverted["when"])
    assert reverted["revert"] is None and reverted["with_revert"] == ["d5"]
    assert reverted["last"][0] == "d5" and "revertable" in reverted["last"][1]


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
    # grows, Enter sends it all, and typing after a click elsewhere still goes into it. The middle fills the screen to
    # start with; Escape leaves the sidebars and what's typed be (it's for what's in the middle). Each letter's
    # time to reach the screen is reported (logged once a minute, see GtkScreen._watch_speed).
    import json
    import subprocess
    import sys
    from pathlib import Path
    script = Path(__file__).with_name("screen_typing_check.py")
    r = subprocess.run(["xvfb-run", "-a", "-s", "-screen 0 1280x800x24", sys.executable, str(script)],
                       capture_output=True, text=True, timeout=120)
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]     # (the display's warnings can come after it)
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "timeout" not in res, res
    empty, typed, two, sent = res["empty"], res["typed"], res["two_lines"], res["sent"]
    # The cursor in the box from the start; the round send button grey until there's something to send, then black.
    assert empty["focused"] and empty["value"] == "" and empty["send"] == "rgb(150, 149, 141)"
    # Nothing under the box (or above it while awake), and in it just the dimmed hint ("Esc" in it only where Escape
    # has sidebars to bring in: a screen with room for them).
    assert empty["words"] == "" and empty["hint"] == "Type anything" and not empty["full"]
    # (Under it, the Claude app's room: 2.4375 rem, 34 px on this screen.)
    assert empty["wake"] == "none" and 25 < empty["below_box"] < 45 and empty["above_box"] < 14
    assert typed["value"] == "hi there" and typed["drafting"] and typed["send"] == "rgb(249, 249, 247)"
    assert two["value"] == "hi there\nsecond line" and two["box_h"] > typed["box_h"] + 15        # a line taller
    assert res["sent_text"] == ["hi there\nsecond line"]
    assert sent["value"] == "" and sent["box_h"] == typed["box_h"] and not sent["drafting"]
    assert res["after_blur"]["value"] == "abc" and res["after_blur"]["focused"]
    assert res["escaped"]["value"] == "abc" and not res["escaped"]["full"] and res["escaped"]["focused"]
    assert res["escaped_again"]["value"] == "abc" and not res["escaped_again"]["full"]
    assert len(res["typed_ms"]) >= 15 and all(0 <= ms < 2000 for ms in res["typed_ms"])



@pytest.mark.skipif(not __import__("shutil").which("xvfb-run") or not __import__("shutil").which("xdotool"),
                    reason="xvfb-run or xdotool not available")
def test_what_is_open_fills_the_middle_and_escape_closes_it():
    # The sidebars from the start (where there's room for them): Messages at the left and Actions at the right, each
    # against its edge of the screen from top to bottom, 17.5 rem wide, as the Claude app's sidebar. Something opened
    # (a website) takes the conversation's place in the middle: a rounded frame between them, 0.75 rem from them and
    # from the top, down to the message box, with its name and a close button (x) at the top. There's no other part. The website has the keys (typing goes into its box); Page Down in the message box scrolls it.
    # The x closes it; opened again, Escape closes it too (from the page), the sidebars staying: only their switches
    # put them away. A message can be pinned (it goes to the top) and opened, with real clicks.
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
    first, start, pinned, content = res["first"], res["start"], res["pinned"], res["content"]
    assert not first["full"] and first["sides"] == [True, True] and first["toggles"]["left"]["shown"]
    assert first["toggles"]["right"]["shown"] and first["hint"] == "Type anything"
    assert not start["full"] and start["sides"] == [True, True]
    w, h = start["view"]
    assert start["messages"][:2] == [0, 0] and start["messages"][3] == h and start["done"][1:] == [0, w, h]
    assert start["messages"][2] == w - start["done"][0] and 260 <= start["messages"][2] <= 300 and start["radius"] == "0px"
    gap = 12                                                    # the frame's space from the sidebars and the top
    # Newest first; pinned ones above, without being cut off; each with its kind's icon.
    assert [i["who"] for i in start["list"] if "msg" in i["cls"]] == ["Mum", "Jane from the bank", "Alex"]
    assert [i["who"] for i in start["pinned"]] == ["sam.k"]
    assert [i["who"] for i in pinned["pinned"]] == ["Mum", "sam.k"] and [i["icon"] for i in pinned["pinned"]] == [
        "#i-chat", "#i-social"]
    assert not any(i["clipped"] or i["title"] for i in pinned["pinned"] + pinned["list"])
    assert res["events"] == [{"type": "pin", "list": "messages", "id": "m2", "on": True},
                             {"type": "open_message", "id": "m1"}]
    # Only the sidebars' two switches, at the top of each, on its edge nearest the middle (Messages' at its right,
    # Actions' at its left), filled in while their sidebar shows.
    assert not start["bottom_toggle"] and set(start["toggles"]) == {"left", "right"}
    tl, tr = start["toggles"]["left"], start["toggles"]["right"]
    assert tl["shown"] and tr["shown"] and start["messages"][2] - 60 < tl["x"] < start["messages"][2]
    assert start["done"][0] < tr["x"] < start["done"][0] + 60
    # Open: the conversation is put away, and the page is in the frame, named after its title, which is between the
    # sidebars (0.75 rem from them) from the top down to the message box, the same space above it.
    assert content["mode"] == "content" and content["content"] and not content["column_shown"]
    assert content["frame_title"] == "A website" and content["keys_in_page"]
    f, v, box = content["frame"], content["content_view"], content["box_rect"]
    assert abs(f[0] - (start["messages"][2] + gap)) <= 1 and abs(f[2] - (start["done"][0] - gap)) <= 1
    assert abs(f[1] - gap) <= 1 and abs(box[1] - f[3] - gap) <= 2
    assert f[0] < v[0] and f[1] + 30 < v[1] and v[2] < f[2] and v[3] < f[3]          # under its bar, inside
    assert content["close"][0] > v[2] - 60 and f[1] < content["close"][1] < v[1]       # the x: top right, in the bar
    assert content["hint"] == "Type anything"
    # "/": its list shows in front. The window lays the page over this one, so the page ends above the list; it
    # comes back down when the search ends (Escape, which ends the search rather than filling the screen).
    sl = res["slash"]
    assert sl["searching"] and sl["results"][3] - sl["results"][1] > 2 * sl["line"] and sl["hint"] is None
    assert sl["content_view"][3] <= sl["results"][1] and sl["content_view"][1] == v[1]
    un = res["unslashed"]
    assert not un["searching"] and un["content_view"] == v and not un["full"]
    # Both sidebars off (each leaving a narrow margin with its switch): what's open stays where it was, as the
    # person asked of the middle; on again.
    off = res["sides_off"]
    assert off["sides"] == [False, False] and off["toggles"]["left"]["pressed"] == "false"
    assert off["content_view"] == v
    assert [(e["left"], e["right"]) for e in res["panel_events"]] == [(False, True), (False, False)]
    assert res["sides_on"]["sides"] == [True, True] and res["sides_on"]["content_view"] == v
    # The website has the keys: clicked, its box gets what's typed, not the message box.
    t = res["typed_in_page"]
    assert t["keys_in_page"] and t["page"]["typed"] == "abc" and t["box"] == ""
    # Page Down in the empty message box scrolls the page.
    assert res["paged"]["page"]["y"] > 300 and not res["paged"]["keys_in_page"]
    # The x: closed, back to the conversation.
    closed = res["closed"]
    assert closed["mode"] == "chat" and closed["content_view"] == [] and not closed["content"] and res["closed_called"]
    assert closed["column_shown"] and not closed["full"]
    # Opened again: Escape from the page closes it too, back to the conversation, the sidebars staying (only their
    # switches put them away); Escape in the conversation leaves them be.
    assert res["reopened"]["content"] and res["reopened"]["content_view"] == v
    esc = res["page_escaped"]
    assert esc["mode"] == "chat" and not esc["content"] and esc["column_shown"] and not esc["full"] and esc["sides"] == [True, True]
    assert not res["chat_escaped"]["full"] and res["chat_escaped"]["sides"] == [True, True] and res["chat_escaped"]["column_shown"]


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
    assert [r["text"] for r in s["results"]] == ["report.pdf~/Documents", "Ask Chat Computer to find \u201crep\u201d"]
    assert s["bold"] == ["rep"] and "sel" in s["results"][0]["cls"] and s["results_box"][3] <= s["box"][1]
    assert "sel" in res["search_down"]["results"][1]["cls"] and "sel" in res["search_up"]["results"][0]["cls"]
    assert not res["opened"]["searching"] and res["opened"]["value"] == ""
    assert res["events"][:2] == [{"type": "search", "q": "rep"}, {"type": "open_file", "path": "/home/sam/Documents/report.pdf"}]
    nothing = res["nothing"]
    assert [r["text"] for r in nothing["results"]] == ["Ask Chat Computer to find \u201czzq\u201d"]
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
    # A sidebar item swiped left shows Pin (the sidebars show from the start); holding a message selects it;
    # a double tap selects a word.
    assert res["swipe"] == {**res["swipe"], "swiped": True, "label": '"Pin"', "visible": "visible"}
    assert res["hold"]["selected"].strip() == "Hi Sam. The weather is lovely today." and res["hold"]["tapPrevented"]
    assert res["double_tap"] == "weather"
    # Messages pulled down from the top where the sidebars aren't showing (the middle filling the screen, or a narrow
    # screen), and gone with a touch elsewhere.
    pulled = {"shown": "flex", "hiddenAfter": True}
    assert res["messages"] == {"full": pulled, "narrow": pulled}

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
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]     # (the display's warnings can come after it)
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
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
    assert "Sam Lee" in d["text"] and "Curriculum Vitae" in d["text"] and "Designer" in d["text"]
    # What's open: where it is on the screen, and printed as it is.
    x, y, w, h = res["rect_open"]
    sw, sh = res["screen"]
    assert 0 <= x and 0 <= y and 200 < w <= sw and 200 < h <= sh
    assert res["open"]["ok"] and "Packing list" in res["open_pdf"]["text"] and "Passport" in res["open_pdf"]["text"]

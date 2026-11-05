"""The touchscreen's system gestures, as an iPad's (chatcomputer/ui/touch.py): which touches the page under the finger
gets, which are held back for a moment (an edge swipe?) and which the screen takes; and what each gesture means."""
import asyncio

from chatcomputer.config import Paths, Settings
from chatcomputer.controller import Controller
from chatcomputer.platform.fake import FakePlatform
from chatcomputer.screen import RecordingScreen
from chatcomputer.ui.touch import CUT_WITHIN_S, HOLD, HOLD_S, KEY, PASS, RELEASE, TAKE, TouchGestures

W, H = 1920.0, 1080.0


def _tg():
    done = []
    tg = TouchGestures(done.append)
    tg.size = (W, H)
    return tg, done


def _swipe(tg, seq, start, end, steps=6, t0=0.0):
    """One finger from start to end; returns the verdicts of its moves and its end."""
    (x0, y0), (x1, y1) = start, end
    verdicts = [tg.begin(seq, x0, y0, t0)[0]]
    for i in range(1, steps + 1):
        f = i / steps
        verdicts.append(tg.move(seq, x0 + (x1 - x0) * f, y0 + (y1 - y0) * f, t0 + 0.02 * i))
    verdicts.append(tg.end(seq, x1, y1, t0 + 0.02 * (steps + 1)))
    return verdicts


def test_touches_in_the_middle_are_the_page_s():
    tg, done = _tg()
    v = _swipe(tg, 1, (900, 500), (900, 200))                   # scrolling a page
    assert v[:-1] == [PASS] * 7 and v[-1] == (PASS, "") and done == []
    assert tg.begin(2, 900, 500, 1.0) == (PASS, []) and tg.end(2, 900, 500, 1.1) == (PASS, "")      # a tap


def test_swipe_up_from_the_bottom_edge_is_home():
    tg, done = _tg()
    v = _swipe(tg, 1, (900, H - 5), (900, H - 300))
    assert v[0] == HOLD and set(v[1:-1]) == {HOLD} and v[-1] == (TAKE, "home") and done == ["home"]


def test_along_the_bottom_edge_and_in_from_the_sides_is_back_or_forward():
    tg, done = _tg()
    assert _swipe(tg, 1, (900, H - 8), (1300, H - 20))[-1] == (TAKE, "back")       # to the right: the one before
    assert _swipe(tg, 2, (900, H - 8), (500, H - 20))[-1] == (TAKE, "forward")
    assert _swipe(tg, 3, (5, 500), (300, 520))[-1] == (TAKE, "back")               # in from the left, as in Safari
    assert _swipe(tg, 4, (W - 4, 500), (W - 300, 480))[-1] == (TAKE, "forward")
    assert done == ["back", "forward", "back", "forward"]


def test_down_from_the_top_centre_is_messages_but_not_from_a_corner():
    tg, done = _tg()
    assert _swipe(tg, 1, (960, 4), (960, 300))[-1] == (TAKE, "messages")
    assert _swipe(tg, 2, (100, 4), (100, 300))[0] == PASS                          # the top left corner: a page's
    assert done == ["messages"]


def test_an_edge_touch_that_isn_t_a_swipe_goes_to_the_page():
    tg, done = _tg()
    # A tap at the very edge: held, then the page gets it all.
    assert tg.begin(1, 10, 500, 0.0)[0] == HOLD and tg.end(1, 10, 500, 0.1) == (RELEASE, "")
    # Scrolling down along the left edge: released once it's clearly not a swipe in.
    v = _swipe(tg, 2, (8, 300), (14, 600), steps=10)
    assert RELEASE in v and v[v.index(RELEASE) + 1:-1] == [PASS] * len(v[v.index(RELEASE) + 1:-1])
    assert v[-1] == (PASS, "")
    # Pressed at the edge and held still: the page's (a long press).
    assert tg.begin(3, W - 6, 400, 0.0)[0] == HOLD
    assert tg.move(3, W - 7, 401, HOLD_S + 0.1) == RELEASE and tg.end(3, W - 7, 401, HOLD_S + 0.2) == (PASS, "")
    assert done == []


def test_a_second_finger_releases_a_held_one():
    tg, done = _tg()
    assert tg.begin(1, 6, 500, 0.0)[0] == HOLD
    assert tg.begin(2, 600, 500, 0.05) == (RELEASE, [])         # what was held goes to the page first
    assert tg.move(1, 200, 500, 0.1) == PASS and tg.end(1, 200, 500, 0.2) == (PASS, "")
    assert done == []


def _fingers(tg, starts, ends, t0=0.0, seq0=10):
    """Several fingers down together, moved, and lifted; returns what begin said for each and the gesture."""
    begun = [tg.begin(seq0 + i, x, y, t0 + 0.01 * i) for i, (x, y) in enumerate(starts)]
    for i, (x, y) in enumerate(ends):
        tg.move(seq0 + i, x, y, t0 + 0.2)
    results = [tg.end(seq0 + i, x, y, t0 + 0.3 + 0.01 * i) for i, (x, y) in enumerate(ends)]
    return begun, results[-1]


def test_three_fingers_copy_cut_paste_undo_and_redo():
    tg, done = _tg()
    wide = [(700, 400), (900, 400), (800, 600)]
    near = [(780, 460), (820, 460), (800, 500)]
    begun, last = _fingers(tg, wide, near)
    # The first two were the page's until the third came: they're cancelled there.
    assert [b[0] for b in begun] == [PASS, PASS, TAKE] and begun[2][1] == [10, 11]
    assert last == (TAKE, "copy")
    assert _fingers(tg, wide, near, t0=CUT_WITHIN_S / 2)[1] == (TAKE, "cut")      # pinched closed twice: cut
    assert _fingers(tg, near, wide, t0=5)[1] == (TAKE, "paste")
    left = [(x - 200, y) for x, y in wide]
    assert _fingers(tg, wide, left, t0=8)[1] == (TAKE, "undo")
    assert _fingers(tg, left, wide, t0=9)[1] == (TAKE, "redo")
    assert _fingers(tg, wide, wide, t0=12)[1] == (TAKE, "")                          # still: nothing
    assert done == ["copy", "cut", "paste", "undo", "redo"]


def test_four_or_five_fingers_home_and_switching():
    tg, done = _tg()
    hand = [(600, 400), (750, 350), (900, 360), (1050, 420), (650, 600)]
    closed = [(820, 450), (840, 440), (860, 440), (880, 450), (830, 480)]
    assert _fingers(tg, hand, closed)[1] == (TAKE, "home")
    assert _fingers(tg, hand[:4], [(x + 300, y) for x, y in hand[:4]], t0=2)[1] == (TAKE, "back")
    assert _fingers(tg, hand[:4], [(x - 300, y) for x, y in hand[:4]], t0=3)[1] == (TAKE, "forward")
    assert _fingers(tg, hand[:4], [(x, y - 300) for x, y in hand[:4]], t0=4)[1] == (TAKE, "home")
    assert done == ["home", "back", "forward", "home"]


def test_the_onscreen_keyboard_s_touches_are_its_own():
    tg, done = _tg()
    tg.keyboard = (0.0, H - 450, W, 450.0)
    assert tg.begin(1, 300, H - 200, 0.0) == (KEY, []) and tg.move(1, 310, H - 200, 0.05) == KEY
    assert tg.end(1, 310, H - 200, 0.1) == (KEY, "")
    # Its bottom row is along the screen's bottom edge: still keys, not Home.
    assert tg.begin(2, 900, H - 5, 1.0)[0] == KEY and tg.end(2, 900, H - 300, 1.2) == (KEY, "")
    assert tg.begin(3, 900, 300, 2.0)[0] == PASS                    # above it: the page's
    assert tg.cancel(3) == PASS and tg.fingers == {} and done == []


def _controller(tmp_path, monkeypatch, **settings):
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "ChatComputer"))
    screen = RecordingScreen()
    c = Controller(Paths().ensure(), Settings(**settings), FakePlatform(), screen, voice=False, index=False)
    return c, screen


def test_pinching_the_conversation_changes_and_keeps_its_text_size(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch, text_scale=1.2)
    c.page_event({"type": "text_scale", "factor": 1.5})
    assert c.settings.text_scale == 1.8 and Settings.load(c.paths).text_scale == 1.8
    assert any(e[0] == "settings" for e in screen.events)
    c.page_event({"type": "text_scale", "factor": 9})               # not past 3 times
    assert c.settings.text_scale == 3.0
    c.page_event({"type": "text_scale", "factor": "no"})
    assert c.settings.text_scale == 3.0


def test_the_onscreen_keyboard_types(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch)

    async def run():
        c.loop = asyncio.get_running_loop()
        said = await c.set_keyboard(True)
        await c._keyboard_action(("text", "hi"))
        await c._keyboard_action(("key", "Return"))
        await c._keyboard_action(("replace", ". "))
        gone = await c.set_keyboard(False)
        return said, gone
    said, gone = asyncio.run(run())
    assert said == "The keyboard is up: tap its keys on the screen." and gone == "The keyboard is away."
    calls = [x for x in c.platform.calls if x[0] in ("type_text", "key")]
    assert calls == [("type_text", ("hi",)), ("key", ("Return", 1)), ("key", ("BackSpace", 1)), ("type_text", (". ",))]
    assert not c.keyboard_up
    c._keyboard_moved((0, 600, 1920, 480, 12.35, 4))           # a finger brought it up (the window says)
    assert c.keyboard_up

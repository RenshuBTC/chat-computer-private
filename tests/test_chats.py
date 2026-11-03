"""Chats: each task's conversation, in the right sidebar's lower half, newest first, kept across starts."""
import json

from chatcomputer.chats import Chats, MOST
from chatcomputer.config import Paths


def test_a_chat_is_named_by_its_first_words_and_comes_back_after_a_restart(tmp_path):
    paths = Paths(tmp_path / "ChatComputer").ensure()
    chats = Chats(paths)
    first = chats.current
    assert chats.shown() == [{"id": first, "title": "", "ts": chats.on_screen()["ts"]}]
    assert chats.said("Plan us a weekend in Bali, nothing too busy, and somewhere to swim every day please")
    assert not chats.said("And book the hotel")                    # (named once, by its first words)
    title = chats.on_screen()["title"]
    assert title.startswith("Plan us a weekend in Bali") and title.endswith("…") and len(title) <= 48
    chats.session("sess-1")
    # + makes a new one, on the screen; pressed again before anything's said, still the same one.
    second = chats.new()["id"]
    assert second != first and chats.new()["id"] == second
    chats.said("Find my tax papers")
    assert [c["title"] for c in chats.shown()] == ["Find my tax papers", title]
    # Opening the first again: it's on the screen, with its session; the list keeps its order by when it was used.
    assert chats.open(first)["session"] == "sess-1" and chats.current == first
    assert chats.open("nope") is None and chats.current == first
    again = Chats(paths)
    assert again.current == first and [c["title"] for c in again.shown()] == ["Find my tax papers", title]


def test_a_damaged_list_starts_afresh_and_the_list_is_kept_short(tmp_path):
    paths = Paths(tmp_path / "ChatComputer").ensure()
    (paths.root / "chats.json").write_text("{not json")
    chats = Chats(paths)
    assert len(chats.shown()) == 1 and chats.current
    for i in range(MOST + 20):
        chats.new()
        chats.said(f"task {i}")
    saved = json.loads((paths.root / "chats.json").read_text())
    assert len(saved["list"]) == MOST and saved["list"][0]["title"] == f"task {MOST + 19}"

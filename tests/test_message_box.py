"""The message box's two buttons under the words, as the Claude app has them: + adds files to the message (their
paths go with it, for the model to read), and the microphone starts or stops listening."""
import asyncio

from chatcomputer.config import Paths, Settings
from chatcomputer.controller import Controller
from chatcomputer.platform.fake import FakePlatform
from chatcomputer.screen import RecordingScreen
from chatcomputer.ui.gtk_app import PAGE_EVENTS, with_files


def test_files_added_with_plus_go_with_the_words():
    assert with_files("read these", []) == "read these"
    assert with_files("what's in it?", ["/home/sam/Documents/a.pdf"]) == \
        "what's in it?\n\n[Added: /home/sam/Documents/a.pdf]"
    two = with_files("", ["/home/sam/a.txt", "/home/sam/b.txt"])
    assert two == "Here are the files I added.\n\n[Added: /home/sam/a.txt]\n[Added: /home/sam/b.txt]"
    assert with_files("", ["/home/sam/a.txt"]).startswith("Here's the file I added.")


def test_the_microphone_button_stops_and_starts_listening(tmp_path, monkeypatch):
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "ChatComputer"))
    screen = RecordingScreen()
    c = Controller(Paths().ensure(), Settings(user_name="Sam"), FakePlatform(), screen, voice=False, index=False)
    assert "mic_toggle" in PAGE_EVENTS

    async def run():
        c.loop = asyncio.get_running_loop()
        c.page_event({"type": "mic_toggle"})           # listening: it stops (asleep: only the wake word wakes it)
        await asyncio.sleep(0.05)
        asleep = c.asleep
        c.page_event({"type": "mic_toggle"})           # and listens again
        return asleep, c.asleep
    assert asyncio.run(run()) == (True, False)
    states = [e[1] for e in screen.events if e[0] == "state"]
    assert states[-2:] == ["sleeping", "listening"]

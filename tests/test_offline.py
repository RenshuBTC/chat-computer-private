"""Without Claude (signed out, or no internet) Chat Computer still does a few things by voice: Wi-Fi, volume,
power. This checks the small offline command parser, in particular that "connect to <network>" joins a
network even when the words wifi/internet/network aren't in the sentence (the offline reply tells people to
say exactly that)."""
import asyncio

from chatcomputer.config import Paths, Settings
from chatcomputer.controller import Controller
from chatcomputer.platform.fake import FakePlatform
from chatcomputer.screen import RecordingScreen


def _controller(tmp_path, monkeypatch):
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "ChatComputer"))
    return Controller(Paths().ensure(), Settings(user_name="Sam"), FakePlatform(), RecordingScreen(),
                      voice=False, index=False)


def _ai_text(screen) -> str:
    return " ".join(v[1] for (k, v) in screen.events if k == "ai")


def test_connect_to_a_network_joins_it_even_without_a_wifi_word(tmp_path, monkeypatch):
    c = _controller(tmp_path, monkeypatch)

    async def run():
        c.loop = asyncio.get_running_loop()
        await c._offline_command("connect to CoffeeShop", signed_out=True)
    asyncio.run(run())

    assert ("wifi_connect", ("CoffeeShop", False)) in c.platform.calls     # the exact SSID, case kept


def test_connect_to_keeps_the_exact_name_and_reports_it(tmp_path, monkeypatch):
    c = _controller(tmp_path, monkeypatch)

    async def run():
        c.loop = asyncio.get_running_loop()
        await c._offline_command("Connect to Home-WiFi 5G.", signed_out=False)
    asyncio.run(run())

    assert ("wifi_connect", ("Home-WiFi 5G", False)) in c.platform.calls
    assert "Connected to Home-WiFi 5G" in _ai_text(c.screen)


def test_plain_wifi_question_still_lists_networks(tmp_path, monkeypatch):
    c = _controller(tmp_path, monkeypatch)

    async def run():
        c.loop = asyncio.get_running_loop()
        await c._offline_command("what's the wifi", signed_out=True)
    asyncio.run(run())

    names = [e[0] for e in c.platform.calls]
    assert "wifi_scan" in names and "wifi_connect" not in names

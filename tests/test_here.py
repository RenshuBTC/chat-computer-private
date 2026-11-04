"""Where the person is and its weather, for the greeting under the word logo (chatcomputer/here.py, chat.js's
CO.here): the place they said they live in, else their time zone's city; its position looked up once, the weather
again after half an hour; offline, the place alone. The controller tells the screen."""
import asyncio
import json

from chatcomputer import tools as T
from chatcomputer.config import Paths, Settings
from chatcomputer.controller import Controller
from chatcomputer.here import Here, zone_city
from chatcomputer.platform.fake import FakePlatform
from chatcomputer.screen import RecordingScreen


def test_a_time_zone_s_city():
    assert zone_city("Asia/Singapore") == "Singapore"
    assert zone_city("America/Argentina/Buenos_Aires") == "Buenos Aires"
    assert zone_city("America/New_York") == "New York"
    assert zone_city("UTC") == zone_city("Etc/GMT+8") == zone_city("") == ""
    assert Here.place_for("Penang", "Asia/Singapore") == "Penang"
    assert Here.place_for("  ", "Asia/Singapore") == "Singapore"


class Meteo:
    """Open-Meteo, standing in: what it was asked, and rain on a warm night."""
    def __init__(self, fail=False):
        self.asked, self.fail = [], fail

    def __call__(self, url):
        self.asked.append(url.split("?")[0] + ("?name=" + url.split("name=")[1] if "name=" in url else ""))
        if self.fail:
            raise OSError("offline")
        if "geocoding" in url:
            return {"results": [{"name": "Singapore", "latitude": 1.29, "longitude": 103.85}]}
        assert "latitude=1.29&longitude=103.85" in url and "current=temperature_2m,weather_code,is_day" in url
        return {"current": {"temperature_2m": 27.4, "weather_code": 61, "is_day": 0}}


def test_found_once_the_weather_kept_half_an_hour(tmp_path):
    now = [1000.0]
    meteo = Meteo()
    here = Here(tmp_path / "here.json", meteo, clock=lambda: now[0])
    assert here.known("Singapore") == {"place": "Singapore"} and here.stale("Singapore")
    info = here.look_up("Singapore")
    assert info == {"place": "Singapore", "code": 61, "temp": 27.4, "day": False}
    assert meteo.asked == ["https://geocoding-api.open-meteo.com/v1/search?name=Singapore", "https://api.open-meteo.com/v1/forecast"]
    assert not here.stale("Singapore") and here.known("Singapore") == info
    # Kept for the next start; after half an hour, the weather again (the place's position is still known).
    again = Here(tmp_path / "here.json", meteo, clock=lambda: now[0] + 60)
    assert again.known("Singapore") == info
    now[0] += 31 * 60
    assert here.stale("Singapore") and here.known("Singapore") == {"place": "Singapore"}
    here.look_up("Singapore")
    assert meteo.asked[2:] == ["https://api.open-meteo.com/v1/forecast"]
    # Another place: none of the old one's weather.
    assert here.known("Penang") == {"place": "Penang"}
    assert json.loads((tmp_path / "here.json").read_text())["place"] == "Singapore"


def test_offline_the_place_alone(tmp_path):
    here = Here(tmp_path / "here.json", Meteo(fail=True))
    assert here.look_up("Singapore") is None and here.known("Singapore") == {"place": "Singapore"}
    assert Here(tmp_path / "none.json").known("") == {}


def test_the_screen_is_told_and_claude_can_set_where_they_live(tmp_path, monkeypatch):
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "cc"))
    screen = RecordingScreen()
    c = Controller(Paths().ensure(), Settings(user_name="Sam Lee", timezone="Asia/Singapore"), FakePlatform(), screen,
                   voice=False, index=False)
    assert c.weather is False                     # (only the real app looks the weather up)
    c._say_here()
    assert ("here", {"place": "Singapore"}) in screen.events
    c.settings.home_place = "Penang"
    c._settings_changed(["home_place"])
    assert screen.events[-1] == ("here", {"place": "Penang"})
    # Looked up where it's allowed to be: the screen hears the weather once it's here.
    c.weather, c.here.get_json = True, Meteo()
    c.settings.home_place = ""

    async def run():
        c.loop = asyncio.get_running_loop()
        c._settings_changed(["home_place"])
        for _ in range(50):
            await asyncio.sleep(0.02)
            if screen.events[-1][1].get("code") is not None:
                break
    asyncio.run(run())
    # First the place, saying its weather is coming (a greeting about to be chosen waits a moment), then the weather.
    assert screen.events[-2:] == [("here", {"place": "Singapore", "pending": True}),
                                  ("here", {"place": "Singapore", "code": 61, "temp": 27.4, "day": False})]
    assert "home_place" in T.SETTABLE

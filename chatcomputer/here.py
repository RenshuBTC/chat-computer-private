"""Where the person is, and the weather there now: for the greeting under the word logo (chat.js: CO.here), which,
as the Claude app's greetings are for the time of day, can be for the place and its weather ("Stay dry in rainy
Singapore, Sam").

The place is where they said they live (settings.home_place), else their time zone's city ("Asia/Singapore":
Singapore). Its position comes from Open-Meteo's geocoding (only the place's name goes there, once: places don't
move), and the weather from Open-Meteo's forecast (only the position goes), looked up again when it's half an hour
old. Both are kept in here.json, so a start shows at once what was last known. Nothing else is sent."""
from __future__ import annotations

import json
import os
import time
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Callable

GEOCODE = "https://geocoding-api.open-meteo.com/v1/search?count=1&language=en&format=json&name="
FORECAST = "https://api.open-meteo.com/v1/forecast?current=temperature_2m,weather_code,is_day&timezone=auto"
FRESH_S = 30 * 60


def zone_city(zone: str) -> str:
    """A time zone's city: "Asia/Singapore" is Singapore, "America/Argentina/Buenos_Aires" Buenos Aires; none for
    UTC and the like."""
    zone = (zone or "").strip()
    if "/" not in zone or zone.split("/", 1)[0] in ("Etc", "SystemV", "posix", "right"):
        return ""
    return zone.rsplit("/", 1)[1].replace("_", " ")


def system_zone() -> str:
    """This computer's own time zone, as Ubuntu keeps it ("" if it can't be told)."""
    try:
        z = Path("/etc/timezone").read_text().strip()
        if z:
            return z
    except OSError:
        pass
    try:
        target = os.readlink("/etc/localtime")
    except OSError:
        return ""
    return target.split("zoneinfo/", 1)[1] if "zoneinfo/" in target else ""


def _get_json(url: str) -> Any:
    req = urllib.request.Request(url, headers={"User-Agent": "Chat Computer"})
    with urllib.request.urlopen(req, timeout=8) as r:
        return json.loads(r.read().decode("utf-8"))


class Here:
    def __init__(self, file: Path, get_json: Callable[[str], Any] = _get_json,
                 clock: Callable[[], float] = time.time) -> None:
        self.file = Path(file)
        self.get_json = get_json
        self.clock = clock
        try:
            self.kept: dict[str, Any] = json.loads(self.file.read_text())
        except (OSError, ValueError):
            self.kept = {}
        if not isinstance(self.kept, dict):
            self.kept = {}

    @staticmethod
    def place_for(home_place: str, timezone: str) -> str:
        return (home_place or "").strip()[:60] or zone_city(timezone or system_zone())

    def known(self, place: str) -> dict[str, Any]:
        """What the screen is told now: the place, with its weather when that's under half an hour old."""
        if not place:
            return {}
        k = self.kept if self.kept.get("place") == place else {}
        if k and self.clock() - float(k.get("ts") or 0) < FRESH_S and isinstance(k.get("code"), int):
            return {"place": place, "code": k["code"], "temp": k.get("temp"), "day": k.get("day")}
        return {"place": place}

    def stale(self, place: str) -> bool:
        return bool(place) and "code" not in self.known(place)

    def look_up(self, place: str) -> dict[str, Any] | None:
        """The weather there now (its position first, if it isn't known): None when it can't be had (offline)."""
        if not place:
            return None
        k = dict(self.kept) if self.kept.get("place") == place else {"place": place}
        try:
            if not isinstance(k.get("lat"), (int, float)) or not isinstance(k.get("lon"), (int, float)):
                found = (self.get_json(GEOCODE + urllib.parse.quote(place)) or {}).get("results") or []
                if not found:
                    return None
                k["lat"], k["lon"] = float(found[0]["latitude"]), float(found[0]["longitude"])
            now = (self.get_json(f"{FORECAST}&latitude={k['lat']}&longitude={k['lon']}") or {}).get("current") or {}
            code = now.get("weather_code")
            if not isinstance(code, (int, float)):
                return None
            temp = now.get("temperature_2m")
            k.update(code=int(code), temp=float(temp) if isinstance(temp, (int, float)) else None,
                     day=now.get("is_day") == 1, ts=self.clock())
        except Exception:
            return None
        self.kept = k
        try:
            self.file.write_text(json.dumps(k))
        except OSError:
            pass
        return self.known(place)

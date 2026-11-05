"""The onscreen keyboard: the iPad's, key for key, and what pressing a key types.

Every key's place and size is the iPad's own (iPadOS on 11-inch iPads), measured from the three pictures of
the onscreen keyboard in Apple's iPad User Guide (support.apple.com, "Type with the onscreen keyboard on
iPad"; the same pictures on "Use predictive text" and "Dictate text"). There the keys span 587.1 by 180.1
picture pixels; each key's left and right edge and each row's top and bottom were found where the brightness
crosses halfway between key and gap, and the three pictures agree to within 1.4 pixels. The tables below are
those edges, from the keys' top left corner, and the keyboard is scaled to the screen as a whole, so every key
keeps the iPad's proportions. Corners are rounded the way the iPad's are (about a fifth of a key's height).

The letter keys carry the iPad's second characters, small above the letter (on the iPad a downward flick
types them; here they show what the .?123 page has in that place). Behaviour from the same guide: Shift makes
the next letter a capital, a double tap on Shift is Caps Lock, a double tap on Space ends the sentence with a
full stop and a space. The .?123 and #+= pages aren't in Apple's pictures: .?123 has the letter keys' second
characters in their places, and #+= the further symbols an iPad has.

The screen draws it (ui/keyboard_layer.py) and a finger on the touchscreen presses it; this file is the
arithmetic.
"""
from __future__ import annotations

from dataclasses import dataclass

REF_W, REF_H = 587.10, 180.12         # the keys' extent in Apple's pictures (pixels)
ROW_EDGES = ((0.00, 40.48), (46.98, 86.77), (93.41, 133.67), (140.19, 180.12))
KEY_EDGES = (
    ((0.00, 52.26), (59.39, 99.67), (107.09, 147.31), (154.51, 194.71), (202.23, 242.34), (249.53, 289.97),
     (297.17, 337.41), (344.53, 384.84), (392.24, 432.35), (439.74, 480.03), (487.29, 527.44), (534.83, 587.10)),
    ((0.00, 66.39), (73.66, 114.03), (121.39, 161.45), (168.76, 209.07), (216.37, 256.46), (264.02, 304.16),
     (311.43, 351.56), (358.92, 399.26), (406.38, 446.64), (453.99, 494.33), (501.43, 587.10)),
    ((0.00, 88.06), (95.37, 135.48), (142.84, 183.27), (190.31, 230.61), (237.86, 278.20), (285.43, 325.42),
     (332.46, 372.91), (380.14, 420.37), (427.63, 467.92), (475.29, 515.35), (522.60, 587.10)),
    ((0.00, 43.43), (50.50, 93.43), (100.83, 144.04), (151.31, 443.62), (451.14, 515.05), (522.35, 587.10)),
)
PITCH = 47.545                        # one letter key to the next (pixels in the pictures)
UNITS = REF_W / PITCH                 # letter keys across the whole keyboard: 12.35
ROWS = 4
# Round the keys a little, as the iPad's; the gaps and the margin around the keys (the iPad's sides and bottom).
CORNER = 0.2                          # of a key's height
MARGIN = (5.6, 7.0, 5.6, 18.0)        # left, top, right, bottom (picture pixels)
DOUBLE_TAP_S = 0.5


def _letters(chars: str, subs: str) -> list[tuple]:
    return [(c, ("text", c), s, "", "") for c, s in zip(chars, subs)]


# Each page: per row, (label, action, second character, symbol drawn instead of the label, where it sits). The
# symbols are drawn (ui/keyboard_layer.py); "bl"/"br" put them at the key's bottom left or right, as the iPad does.
_TAB = ("tab", ("key", "Tab"), "", "tab", "bl")
_DEL = ("delete", ("key", "BackSpace"), "", "delete", "br")
_CAPS = ("caps lock", ("caps",), "", "caps", "bl")
_RET = ("return", ("key", "Return"), "", "return", "br")
_SHIFT_L = ("shift", ("shift",), "", "shift", "bl")
_SHIFT_R = ("shift", ("shift",), "", "shift", "br")
_EMOJI = ("emoji", ("page", "emoji"), "", "emoji", "bl")
_MIC = ("dictate", ("dictate",), "", "mic", "bl")
_SPACE = ("space", ("text", " "), "", "", "")
_HIDE = ("hide keyboard", ("hide",), "", "hide", "br")


def _bottom(label: str, page: str, first: tuple = _EMOJI) -> list[tuple]:
    return [first, (label, ("page", page), "", "", "bl"), _MIC, _SPACE, (label, ("page", page), "", "", "br"), _HIDE]


PAGES: dict[str, list[list[tuple]]] = {
    "letters": [
        [_TAB] + _letters("qwertyuiop", "1234567890") + [_DEL],
        [_CAPS] + _letters("asdfghjkl", "@#$&*()'\"") + [_RET],
        [_SHIFT_L] + _letters("zxcvbnm", "%-+=/;:") + [(",", ("text", ","), "!", "", ""),
                                                       (".", ("text", "."), "?", "", ""), _SHIFT_R],
        _bottom(".?123", "numbers"),
    ],
    "numbers": [
        [_TAB] + _letters("1234567890", " " * 10) + [_DEL],
        [_CAPS] + _letters("@#$&*()'\"", " " * 9) + [_RET],
        [("#+=", ("page", "symbols"), "", "", "bl")] + _letters("%-+=/;:!?", " " * 9) +
        [("#+=", ("page", "symbols"), "", "", "br")],
        _bottom("ABC", "letters"),
    ],
    "symbols": [
        [_TAB] + _letters("[]{}#%^*+=", " " * 10) + [_DEL],
        [_CAPS] + _letters("_\\|~<>€£¥", " " * 9) + [_RET],
        [(".?123", ("page", "numbers"), "", "", "bl")] + _letters("•…°§¶¡¿,.", " " * 9) +
        [(".?123", ("page", "numbers"), "", "", "br")],
        _bottom("ABC", "letters"),
    ],
    "emoji": [
        [_TAB] + [(e, ("text", e), "", "", "") for e in ("😀", "😂", "😊", "😍", "🥰", "😘", "😎", "🤔", "😢", "😭")] + [_DEL],
        [_CAPS] + [(e, ("text", e), "", "", "") for e in ("👍", "👎", "👏", "🙏", "💪", "👋", "❤️", "🔥", "✨")] + [_RET],
        [("ABC", ("page", "letters"), "", "", "bl")] +
        [(e, ("text", e), "", "", "") for e in ("🎉", "✅", "❌", "⭐", "😴", "🤗", "😮", "👀", "🙂")] +
        [("ABC", ("page", "letters"), "", "", "br")],
        _bottom(".?123", "numbers", ("ABC", ("page", "letters"), "", "", "bl")),
    ],
}


@dataclass
class Key:
    label: str
    action: tuple            # ("text", "a"), ("key", "Tab"), ("shift",), ("caps",), ("page", "numbers"),
                             # ("dictate",), ("hide",)
    x: float                 # the key's face, in the keyboard's own pixels
    y: float
    w: float
    h: float
    cell: tuple[float, float, float, float]     # the face and half the gap around it: what a press lands on
    sub: str = ""            # the second character, small above the letter
    symbol: str = ""         # drawn instead of the label: "tab", "delete", "caps", "return", "shift", "emoji",
                             # "mic", "hide"
    corner: str = ""         # where the label or symbol sits: "bl", "br", or "" (the middle)

    @property
    def special(self) -> bool:
        return self.action[0] != "text" or self.action == ("text", " ")


class Keyboard:
    def __init__(self) -> None:
        self.page = "letters"
        self.shift = "off"               # "off", "once" or "lock"
        self._shift_t = -1e9
        self._space_t = -1e9
        self._before_space = ""          # what was typed just before the last space (through this keyboard)
        self.width = self.height = 0.0
        self.scale = 1.0
        self.origin = (0.0, 0.0)         # the keys' top left, in the keyboard's pixels
        self.keys: list[Key] = []

    # ---------------------------------------------------------------- where the keys are
    @staticmethod
    def size_for(width: float, height: float) -> tuple[float, float]:
        """The keyboard's own size (with its margins) to fit width x height, the iPad's proportions kept."""
        mw, mh = MARGIN[0] + REF_W + MARGIN[2], MARGIN[1] + REF_H + MARGIN[3]
        s = min(width / mw, height / mh)
        return mw * s, mh * s

    def layout(self, width: float, height: float) -> list[Key]:
        """The keys of the current page for a keyboard this many pixels across and high: the iPad's keyboard as
        big as fits, in the middle."""
        self.width, self.height = float(width), float(height)
        mw, mh = MARGIN[0] + REF_W + MARGIN[2], MARGIN[1] + REF_H + MARGIN[3]
        s = self.scale = min(width / mw, height / mh)
        ox = (width - mw * s) / 2 + MARGIN[0] * s
        oy = (height - mh * s) / 2 + MARGIN[1] * s
        self.origin = (ox, oy)
        keys = []
        for r, (row, edges) in enumerate(zip(PAGES[self.page], KEY_EDGES)):
            top, bottom = ROW_EDGES[r]
            # A press in a gap goes to the nearer key: each key's cell reaches halfway into its gaps.
            up = ROW_EDGES[r - 1][1] if r else top - (ROW_EDGES[1][0] - ROW_EDGES[0][1])
            down = ROW_EDGES[r + 1][0] if r + 1 < ROWS else bottom + (ROW_EDGES[1][0] - ROW_EDGES[0][1])
            cy0, cy1 = (top + up) / 2, (bottom + down) / 2
            for i, ((left, right), (label, action, sub, symbol, corner)) in enumerate(zip(edges, row)):
                prev = edges[i - 1][1] if i else left - (edges[1][0] - edges[0][1])
                nxt = edges[i + 1][0] if i + 1 < len(edges) else right + (edges[1][0] - edges[0][1])
                cx0, cx1 = (left + prev) / 2, (right + nxt) / 2
                shown = label.upper() if (len(label) == 1 and label.isalpha() and self.shift != "off") else label
                keys.append(Key(shown, action, ox + left * s, oy + top * s, (right - left) * s, (bottom - top) * s,
                                (ox + cx0 * s, oy + cy0 * s, (cx1 - cx0) * s, (cy1 - cy0) * s),
                                sub.strip(), symbol, corner))
        self.keys = keys
        return keys

    def area(self) -> tuple[float, float, float, float]:
        """Where the keys are (x, y, width, height in the keyboard's pixels)."""
        return (self.origin[0], self.origin[1], REF_W * self.scale, REF_H * self.scale)

    def hit(self, x: float, y: float) -> Key | None:
        """The key a press at x, y lands on: the one whose cell it's in, or in that row the nearest (the ends of
        a row); none outside the keys' rows."""
        if not self.keys:
            return None
        rows: dict[float, list[Key]] = {}
        for k in self.keys:
            rows.setdefault(k.cell[1], []).append(k)
        for cy, row in rows.items():
            if cy <= y < cy + row[0].cell[3]:
                for k in row:
                    if k.cell[0] <= x < k.cell[0] + k.cell[2]:
                        return k
                ax, _ay, aw, _ah = self.area()
                if ax - row[0].h <= x < ax + aw + row[0].h:
                    return min(row, key=lambda k: abs(k.x + k.w / 2 - x))
                return None
        return None

    # ---------------------------------------------------------------- pressing
    def press(self, key: Key, t: float) -> tuple | None:
        """Press a key: what to type (("text", "a"), ("key", "Return"), or ("replace", ". "): delete the last
        character, then type that), ("dictate",), ("hide",), or None when it only changes the keyboard (Shift,
        Caps Lock, another page). The caller lays the keyboard out again after a change."""
        kind = key.action[0]
        if kind == "text":
            ch = key.action[1]
            if ch == " ":
                if t - self._space_t < DOUBLE_TAP_S and self._before_space[-1:].isalnum():
                    self._space_t, self._before_space = -1e9, ""
                    return ("replace", ". ")         # a double tap on Space: a full stop and a space
                self._space_t = t
                return ("text", " ")
            if self.shift != "off":
                if len(ch) == 1 and ch.isalpha():
                    ch = ch.upper()
                if self.shift == "once":
                    self.shift = "off"
            self._before_space, self._space_t = ch, -1e9
            return ("text", ch)
        if kind == "shift":
            if self.shift == "once" and t - self._shift_t < DOUBLE_TAP_S:
                self.shift = "lock"                  # a double tap on Shift: Caps Lock
            elif self.shift == "off":
                self.shift, self._shift_t = "once", t
            else:
                self.shift = "off"
            return None
        if kind == "caps":
            self.shift = "off" if self.shift == "lock" else "lock"
            return None
        if kind == "page":
            self.page, self.shift = key.action[1], "off"
            return None
        self._space_t = -1e9
        return key.action                 # ("key", name), ("dictate",) or ("hide",)

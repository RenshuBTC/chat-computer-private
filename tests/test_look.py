"""Chat Computer's look: black text on a warm off-white only, in Atkinson Hyperlegible, which Chat Computer brings
itself."""
import hashlib
import json
import re
from pathlib import Path
from urllib.parse import unquote, urlparse

from chatcomputer.config import Paths, Settings
from chatcomputer.ui import render

ASSETS = render.ASSETS
FONT_FILES = ["AtkinsonHyperlegibleNext.ttf", "AtkinsonHyperlegibleNext-Italic.ttf",
              "AtkinsonHyperlegibleMono.ttf", "AtkinsonHyperlegibleMono-Italic.ttf"]
COLOR = re.compile(r"#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(|\b(?:grey|gray|silver)\b|\b(?:white|black)\b(?!-)", re.I)


def _asset(name: str) -> str:
    return (ASSETS / name).read_text(encoding="utf-8")


def _code(css: str) -> str:
    """CSS without its comments."""
    return re.sub(r"/\*.*?\*/", "", css, flags=re.S)


def test_only_black_text_on_a_warm_off_white():
    theme, chat = _code(_asset("theme.css")), _code(_asset("chat.css"))
    root = re.search(r":root\s*\{([^}]*)\}", theme).group(1)
    # White made 8% dimmer, faintly warm (black on it 19.3:1; see theme.css for why), and black.
    assert re.search(r"--ground:\s*#F8F5F0;", root) and re.search(r"--ink:\s*#000000;", root)
    # The light grey of your messages' bubbles (black on it is still 16.5:1), as the Claude app shows them; a little
    # darker grey for lines; and one dimmed grey, only for the empty box's hint and the send button with nothing to
    # send (as ChatGPT's).
    assert re.search(r"--bubble:\s*#E7E4DD;", root) and re.search(r"--line:\s*#DDD9D1;", root)
    assert re.search(r"--dim:\s*#918F8A;", root)
    # The message box: white, set off from the background (with a soft shadow), as the Claude app's is.
    assert re.search(r"--paper:\s*#FFFFFF;", root)
    # Those are the only colours: everything else uses them.
    assert COLOR.findall(theme.replace(root, "")) == []
    assert COLOR.findall(chat) == []
    used = set(re.findall(r"var\((--[a-z-]+)", theme + chat + render.DOC_CSS))
    sizes = {"--co-size", "--co-column", "--panel", "--left", "--rail", "--composer-h", "--kb", "--side-gap"}
    assert used - sizes == {"--ground", "--ink", "--bubble", "--line", "--dim", "--paper", "--sans", "--mono"}
    dims = re.findall(r"([^{}]+)\{[^}]*var\(--dim\)", chat)
    assert [d.strip() for d in dims] == ["#input::placeholder", "#send"]
    # No dark mode.
    for text in (theme, chat, _asset("chat.html"), _asset("chat.js"), render.DOC_CSS):
        assert "data-theme" not in text and "dark" not in text.lower()
    # Pages Chat Computer makes are black on the same off-white; only a playing video sits on black, as in any player.
    assert COLOR.findall(render.DOC_CSS) == ["#000"]
    assert re.search(r"\.media \{[^}]*background: #000;", render.DOC_CSS)


def test_your_words_are_in_a_bubble_not_grey_text():
    # Like the Claude app: your messages on the right in a light grey bubble, upright and black; no text is grey.
    chat = _code(_asset("chat.css"))
    assert re.search(r"\.you \{[^}]*align-self: flex-end;[^}]*background: var\(--bubble\);", chat)
    assert "italic" not in chat
    # Nothing is see-through grey: what fades in goes all the way to black, and a row under a message is either
    # there or not (opacity only ever 0 or 1, but for the thinking dot's pulse).
    rest = chat.replace("@keyframes pulse { 0%, 100% { opacity: 0.35; } 50% { opacity: 1; } }", "")
    assert set(re.findall(r"opacity:\s*([\d.]+)", rest)) <= {"0", "1"}
    # The empty box's hint is its placeholder: the one grey text (--dim), only there.
    rules = [r.strip() for r in re.findall(r"([^{}]*::placeholder[^{}]*\{[^}]*\})", chat)]
    assert rules == ["#input::placeholder { color: var(--dim); opacity: 1; }"]


def test_fonts_come_with_chatcomputer():
    fonts = ASSETS / "fonts"
    for name in FONT_FILES:
        assert (fonts / name).read_bytes()[:4] == b"\x00\x01\x00\x00", name   # TrueType
    for licence in ("OFL-AtkinsonHyperlegibleNext.txt", "OFL-AtkinsonHyperlegibleMono.txt"):
        assert "SIL OPEN FONT LICENSE Version 1.1" in (fonts / licence).read_text()
    # The README says where they came from; its checksums are the files'.
    readme = (fonts / "README.md").read_text()
    for name in FONT_FILES:
        assert f"{hashlib.sha256((fonts / name).read_bytes()).hexdigest()}  {name}" in readme
    # theme.css loads each of them, upright and italic, at every weight they have.
    theme = _asset("theme.css")
    faces = re.findall(r"@font-face\s*\{([^}]*)\}", theme)
    assert len(faces) == 4 and all("font-weight: 200 800;" in f for f in faces)
    urls = [u for f in faces for u in re.findall(r'url\("([^"]+)"\)', f)]
    assert sorted(urls) == sorted(f"fonts/{n}" for n in FONT_FILES)
    assert '--sans: "Atkinson Hyperlegible Next", sans-serif;' in theme
    assert '--mono: "Atkinson Hyperlegible Mono", monospace;' in theme


def test_pages_shown_from_elsewhere_still_find_the_fonts(tmp_path):
    html = render.document_html("Notes", "Hello **there**", Settings())
    urls = re.findall(r'url\("([^"]+)"\)', html)
    assert len(urls) == 4
    for u in urls:
        p = urlparse(u)
        assert p.scheme == "file" and Path(unquote(p.path)).is_file(), u
    song = tmp_path / "song.mp3"
    song.write_bytes(b"ID3")
    kind, page = render.file_view(str(song), Settings(), Paths(tmp_path / "c"))
    assert kind == "html" and '<div class="sound">' in page and 'class="media"' not in page


def test_a_document_mentioning_script_tags_stays_whole():
    # The text goes into the page inside <script>: a "</script>" in it (a web page's source, a README about HTML)
    # used to end the script there and show the rest as raw HTML, running any script in it.
    md = 'Example:\n\n```\n<script>alert("x")</script>\n```\n<!-- note -->'
    html = render.document_html("Notes", md, Settings())
    assert html.count("</script>") == html.count("<script>") == 2           # md.js, and the one that renders
    arg = re.search(r"renderMarkdown\((.*)\);</script>", html, re.S).group(1)
    assert "<" not in arg and json.loads(arg) == md


def test_there_is_no_theme_setting(tmp_path):
    assert not hasattr(Settings(), "theme")
    paths = Paths(tmp_path / "ChatComputer").ensure()
    paths.settings_file.write_text(json.dumps({"theme": "dark", "speech_speed": 1.2}))
    s = Settings.load(paths)
    assert s.speech_speed == 1.2 and "theme" not in s.extra
    s.save(paths)
    assert "theme" not in json.loads(paths.settings_file.read_text())

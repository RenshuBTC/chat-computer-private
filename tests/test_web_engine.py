"""Chat Computer's own model engine in the website's chat (docs/demo/engine: os/model as WebAssembly, through
os/webengine, in a Web Worker; engine.js is the page's side). The files there are what scripts/build_web_engine.py
makes: the module from this source, and the model Chat Computer pins (models.json's Qwen3.5 0.8B). Tried in Chromium
with Playwright (tests/web_engine_check.js), with no network (Hugging Face stood in for by local files): it gets a model
ready (downloaded in pieces, checked, kept by the browser), answers exactly as the engine's program on Linux does with
the same arithmetic (os/model's kernel_check example, as os/check.py --model checks the kernel's model), streams the
answer as the program streams it, thinks when asked, stops when asked, loads the browser's copy on a later visit with
nothing downloaded (and the first message's reading with it), makes the same model's file from Qwen's published files
as the program does (8 and 4 bits a weight), and forgets the model when asked.

With CHATCOMPUTER_WEB_ENGINE_BENCH=1, it also times a model of Qwen3.5 0.8B's shape with random weights (0.8 GB of
memory) in the browser, and prints how fast it reads and writes."""
import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
OS = ROOT / "os"
TINY = ROOT / "tests" / "data" / "tiny-model.ccm"
sys.path.insert(0, str(ROOT / "scripts"))

import build_web_engine  # noqa: E402


def test_the_website_s_engine_gets_the_model_chatcomputer_pins():
    assert (ROOT / "docs" / "demo" / "engine" / "model.json").read_bytes() == build_web_engine.manifest(), \
        "run: python3 scripts/build_web_engine.py"
    m = json.loads(build_web_engine.manifest())
    assert m["repo"] == "Qwen/Qwen3.5-0.8B" and m["base"] == "https://huggingface.co" and m["bits"] == 8
    assert {f["name"] for f in m["files"]} >= {"config.json", "tokenizer.json", "chat_template.jinja"}
    # Only what the page needs is there: its own files (and the model's refusal direction, once it's been found), and
    # no third-party code.
    names = {p.name for p in (ROOT / "docs" / "demo" / "engine").iterdir()} - {f"{m['name']}.refusal"}
    assert sorted(names) == ["engine.js", "engine.wasm", "model.json", "worker.js"]
    for name in ("engine.js", "worker.js"):
        js = (ROOT / "docs" / "demo" / "engine" / name).read_text()
        assert "import(" not in js and "importScripts" not in js and "eval(" not in js, name


def _pinned_rust() -> bool:
    """Whether the compiler os/rust-toolchain.toml pins is here (rustup would otherwise download it, which is the OS
    test's job: os.yml), with WebAssembly or able to add it."""
    if not shutil.which("cargo") or not shutil.which("rustup"):
        return False
    channel = next(line.split('"')[1] for line in (OS / "rust-toolchain.toml").read_text().splitlines() if line.startswith("channel"))
    toolchains = subprocess.run(["rustup", "toolchain", "list"], capture_output=True, text=True).stdout
    return any(line.startswith(channel + "-") for line in toolchains.splitlines())


@pytest.mark.skipif(not _pinned_rust(), reason="the pinned Rust compiler isn't here")
def test_the_module_is_what_this_source_builds():
    r = subprocess.run([sys.executable, str(ROOT / "scripts" / "build_web_engine.py"), "--check"], capture_output=True, text=True)
    assert r.returncode == 0, r.stdout[-2000:] + r.stderr[-2000:]


def _node_with_playwright():
    node = shutil.which("node")
    if not node:
        return None, None
    root = subprocess.run(["npm", "root", "-g"], capture_output=True, text=True).stdout.strip() if shutil.which("npm") else ""
    if not root or not (Path(root) / "playwright").exists():
        return None, None
    return node, root


SYSTEM = "You are Chat Computer, here for a test. Answer briefly."
TOOLS = [
    {"type": "function", "function": {"name": "show_map", "description": "Show a map of a place.", "parameters": {
        "type": "object", "properties": {"place": {"type": "string", "description": "The place's name"}}, "required": ["place"]}}},
    {"type": "function", "function": {"name": "play_piano", "description": "Play some calm piano.",
                                      "parameters": {"type": "object", "properties": {}}}},
]
LONG = ("Tell me about the weather, the news, and what's on: " + "a long day of rain and sun, of wind and calm, " * 14).strip()
# The questions (as the page asks: OpenAI's chat completions requests, the likeliest token each time), each with the
# conversation as kernel_check takes it. "play piano" is one the tiny model answers with a tool call (of its random
# words: no name); the "tool_round" asks after a tool's result.
CALLED = {"role": "assistant", "content": "", "tool_calls": [{"id": "call_1_0", "type": "function", "function": {
    "name": "show_map", "arguments": json.dumps({"place": "Paris"})}}]}
QUESTIONS = {
    "hello": ([{"role": "user", "content": "hello"}], 24),
    "calls": ([{"role": "user", "content": "play piano"}], 160),
    "tool_round": ([{"role": "user", "content": "show me Paris"}, CALLED,
                    {"role": "tool", "tool_call_id": "call_1_0", "content": "The map of Paris is on the screen."}], 24),
    "long": ([{"role": "user", "content": LONG}], 200),
}


def _request(messages, max_tokens):
    return {"messages": [{"role": "system", "content": SYSTEM}] + messages, "tools": TOOLS, "think": False,
            "temperature": 0, "max_tokens": max_tokens}


def _kernel_form(m):
    """A message as kernel_check takes it (an assistant's tool calls as "calls", their arguments as written)."""
    if m["role"] == "assistant":
        calls = [{"name": c["function"]["name"], "args": json.loads(c["function"]["arguments"])} for c in m.get("tool_calls", [])]
        return {"role": "assistant", "content": m["content"] or "", "calls": calls}
    return {"role": m["role"], "content": m["content"]}


def _engine_says(model: Path, request: dict, tmp: Path) -> dict:
    """What the engine's program on Linux answers (kernel_check: no thinking, the likeliest token each time, on one
    core, with the plain arithmetic, which the page's is exactly)."""
    (tmp / "asked.json").write_text(json.dumps([_kernel_form(m) for m in request["messages"]]))
    (tmp / "tools.json").write_text(json.dumps(request["tools"]))
    r = subprocess.run(["cargo", "run", "-q", "--release", "-p", "model", "--features", "std", "--example", "kernel_check", "--",
                        str(model), str(tmp / "asked.json"), "--tools", str(tmp / "tools.json"),
                        "--max", str(request["max_tokens"]), "--level", "plain"], cwd=OS, capture_output=True, text=True)
    assert r.returncode == 0, r.stderr[-2000:]
    return json.loads(r.stdout)


def _refusal_file(model: bytes) -> bytes:
    """A refusal direction for the model, as find-refusal keeps one (os/model/src/refusal.rs: "CCREFUS1", the model
    file's mark, eleven numbers, the direction): one of unit length along the first of the model's 64 numbers, taken
    out all the way. (The tiny model's random words refuse nothing, so find-refusal finds nothing to take out of it.)"""
    table = int.from_bytes(model[8:12], "little")
    mark = 0xCBF29CE484222325
    for b in model[:12 + table] + len(model).to_bytes(8, "little"):
        mark = ((mark ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    one, kl = struct.unpack("<I", struct.pack("<f", 1.0))[0], struct.unpack("<I", struct.pack("<f", 0.03))[0]
    numbers = [1, 64, 2, one, 12, 9, 0, kl, 12, 4, 0]           # method, width, layer, strength, checked, before...
    return b"CCREFUS1" + mark.to_bytes(8, "little") + struct.pack("<11I", *numbers) + struct.pack("<64f", 1.0, *[0.0] * 63)


def _page_said(answer: dict) -> dict:
    """The page's answer in kernel_check's form: its words, and its tool calls (names, and arguments as written)."""
    calls = [{"name": c["function"]["name"], "args": json.loads(c["function"]["arguments"])}
             for c in answer["message"].get("tool_calls") or []]
    return {"text": answer["message"]["content"] or "", "calls": calls}


def _streamed_as_the_program_streams(answer: dict, max_tokens: int):
    """Its pieces: first the role, then words, thinking or tool calls, then an empty piece with why it ended (as the
    program says it: out of tokens, else tool calls if there were any, else stop) and the tokens it took; the words and
    calls as the whole message has them."""
    deltas, message, end = answer["deltas"], answer["message"], answer["end"]
    assert deltas[0] == {"role": "assistant"} and deltas[-1] == {"content": ""}
    tokens = end["usage"]["completion_tokens"]
    assert 0 < tokens <= max_tokens
    assert end["finish_reason"] == ("length" if tokens == max_tokens else "tool_calls" if message.get("tool_calls") else "stop")
    # (The whole message is the words trimmed, with any unfinished character the turn ended on: never streamed, as
    # the program never streams it.)
    words, whole = "".join(d.get("content", "") for d in deltas[1:-1]).rstrip(), message["content"] or ""
    assert whole.startswith(words) and len(whole) - len(words) <= 4, (words, whole)
    calls = [c for d in deltas for c in d.get("tool_calls", [])]
    assert calls == (message.get("tool_calls") or [])
    for k, c in enumerate(calls):
        assert c["index"] == k and c["type"] == "function" and c["id"].startswith("call_")


@pytest.mark.skipif(_node_with_playwright()[0] is None or not _pinned_rust(),
                    reason="node with Playwright, or the pinned Rust compiler, not available")
def test_the_engine_answers_in_the_browser_as_it_does_on_a_computer(tmp_path):
    node, modules = _node_with_playwright()
    # Qwen's published files for a small model of random weights, and the engine's program's own file made from them.
    published = tmp_path / "published"
    subprocess.run(["cargo", "run", "-q", "-p", "webengine", "--example", "tiny_published", "--", str(published)],
                   cwd=OS, check=True)
    made = {}
    for bits in (8, 4):
        out = tmp_path / f"published-{bits}.ccm"
        subprocess.run(["cargo", "run", "-q", "--release", "-p", "model", "--features", "std", "--bin", "chat-computer-model",
                        "--", "convert", str(published), str(out), "--bits", str(bits)], cwd=OS, check=True,
                       capture_output=True)
        made[bits] = out
    questions = {q: _request(m, n) for q, (m, n) in QUESTIONS.items()}
    want = {q: _engine_says(TINY, r, tmp_path) for q, r in questions.items()}
    # (A first message long enough to take seconds to read: a question asked meanwhile takes over the reading.)
    long_first = _request([{"role": "user", "content": "hello"}], 24)
    long_first["messages"][0]["content"] = SYSTEM + " Keep this in mind." * 200
    want_long_first = _engine_says(TINY, long_first, tmp_path)
    want_published = {bits: _engine_says(made[bits], questions["hello"], tmp_path) for bits in (8, 4)}
    assert want["calls"]["calls"], "the tiny model no longer calls a tool for \"play piano\": choose another question"
    (tmp_path / "tiny.refusal").write_bytes(_refusal_file(TINY.read_bytes()))
    setup = {"tiny": str(TINY), "published": str(published), "questions": questions, "long_first": long_first,
             "first": {"messages": [{"role": "system", "content": SYSTEM}], "tools": TOOLS},
             "refusal": str(tmp_path / "tiny.refusal"), "bench": None}
    if os.environ.get("CHATCOMPUTER_WEB_ENGINE_BENCH"):
        setup["bench"] = str(tmp_path / "bench.wasm")
        subprocess.run([sys.executable, str(ROOT / "scripts" / "build_web_engine.py"), "--bench", setup["bench"]], check=True)
    (tmp_path / "setup.json").write_text(json.dumps(setup))
    r = subprocess.run([node, str(ROOT / "tests" / "web_engine_check.js"), str(tmp_path / "setup.json")], capture_output=True,
                       text=True, timeout=600, env={**os.environ, "NODE_PATH": modules})
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "failed" not in res, res["failed"]
    assert res["errors"] == []

    # The first visit: the model downloaded (in pieces), checked and kept, then loaded; its first message read ahead.
    first = res["first"]
    assert first["supported"] == {"ok": True, "why": ""}
    tiny = TINY.read_bytes()
    assert first["before"]["kept"] is False and first["before"]["download"] == len(tiny)
    assert first["progress"][0] == "downloading" and "keeping" in first["progress"]
    assert first["after"]["kept"] is True and first["after"]["kept_bytes"] == len(tiny) and first["after"]["loaded"] is True
    assert first["info"]["layers"] == 4 and first["info"]["bytes"] == len(tiny) and first["info"]["label"] == "tiny-test"
    assert first["ahead"]["tokens"] > 100

    # Each answer is the engine's own, streamed as its program streams it.
    for q, (_, max_tokens) in QUESTIONS.items():
        answer = res["answers"][q]
        assert _page_said(answer) == want[q], q
        _streamed_as_the_program_streams(answer, max_tokens)
    long = res["answers"]["long"]
    assert long["reading"][0] == long["reading"][1] == long["end"]["usage"]["prompt_tokens"]
    # (Each question after the first reads only what follows the first message, which was read ahead.)
    for q in QUESTIONS:
        usage = res["answers"][q]["end"]["usage"]
        assert usage["prompt_tokens_details"]["cached_tokens"] >= first["ahead"]["tokens"], (q, usage)

    # Thinking, when asked: streamed as reasoning_content, kept in the message as it is.
    thinking = res["thinking"]
    thought = "".join(d.get("reasoning_content", "") for d in thinking["deltas"])
    assert thought and thought.startswith(thinking["message"].get("reasoning_content", "\0"))

    # Stopped where it was: what it said so far is the answer; the next is as ever.
    stopped = res["stopped"]
    assert stopped["end"]["finish_reason"] in ("stop", "tool_calls")
    assert 3 <= stopped["end"]["usage"]["completion_tokens"] < 4000
    assert _page_said(res["after_stop"]) == want["hello"]

    # A later visit: the browser's copy, with nothing downloaded, and the first message's reading kept with it.
    later = res["later"]
    assert later["downloads"] == 0 and "downloading" not in later["progress"] and later["progress"][-1] == "loading"
    assert _page_said(later["answer"]) == want["hello"]
    assert later["answer"]["end"]["usage"]["prompt_tokens_details"]["cached_tokens"] >= first["ahead"]["tokens"]

    # Asked while a long first message was being read ahead: the reading stopped for the question, which went on from
    # where it had got (what was read ahead counts as read), and answered as ever.
    taken = res["taken_over"]
    assert taken["ahead"]["read"] is False and taken["ahead"]["tokens"] > 4000
    assert {"text": taken["message"]["content"] or "", "calls": []} == want_long_first
    usage = taken["end"]["usage"]
    assert 0 < usage["prompt_tokens_details"]["cached_tokens"] < taken["ahead"]["tokens"], usage

    # Qwen's published files made into the engine's own file in the browser: the program's file, byte for byte; and
    # its answers the program's.
    for bits in (8, 4):
        got, file = res[f"published{bits}"], made[bits].read_bytes()
        assert got["progress"][0] == "downloading" and "keeping" in got["progress"]
        assert got["kept_bytes"] == len(file) and got["kept_sha256"] == hashlib.sha256(file).hexdigest(), bits
        assert _page_said(got["answer"]) == want_published[bits], bits

    # A refusal direction beside the engine, checked against its pin and taken out as the model runs (its answers are
    # another model's then); one that isn't the file pinned is refused before anything is downloaded.
    assert res["uncensored"]["info"]["uncensored"] == {"taken_out": True, "checked": 12, "refused_before": 9, "refused_after": 0}
    assert _page_said(res["uncensored"]["answer"]) != want["hello"]
    assert "isn't the one pinned" in res["misPinned"]["error"]

    # Forgotten: its files gone from the browser, the engine stopped.
    forgotten = res["forgotten"]
    assert forgotten["status"]["kept"] is False
    assert not [n for n in forgotten["files"] if n.startswith("tiny-published-") and "-4bit." in n], forgotten["files"]
    assert forgotten["chat_after"] != "answered"

    speed = res["speed"]
    assert "error" not in speed, speed
    print(f"\nThe tiny model in Chromium: reads {speed['read_tok_s']:.0f} tokens a second ({speed['prompt_tokens']} read), "
          f"writes {speed['write_tok_s']:.0f} ({speed['written']} written)")
    if setup["bench"]:
        bench = res["bench"]
        assert "error" not in bench, bench
        print(f"A model of Qwen3.5 0.8B's shape (8 bits, {bench['memory_gb']:.2f} GB of memory): reads "
              f"{', '.join(f'{x:.1f}' for x in bench['read_tok_s'])} tokens a second (64 at a time), writes "
              f"{', '.join(f'{x:.1f}' for x in bench['write_tok_s'])}")

"""Chat Computer's own model: which one a computer gets (models.pick), getting it ready (models.Store: downloaded from
the pinned revision, each file checked against its SHA-256, carried on after a break, converted, the downloads
removed, its refusals found and taken out), the engine's program that comes with Chat Computer, and the model's
server (server.py) run by the real engine with a tiny model of random weights (tests/data/tiny-model.ccm, written by
os/model's example tiny_model)."""
import asyncio
import hashlib
import http.server
import json
import os
import platform
import shutil
import socketserver
import threading
from pathlib import Path
from types import SimpleNamespace

import pytest

from chatcomputer.brain import models
from chatcomputer.brain import server as S

TINY = Path(__file__).resolve().parent / "data" / "tiny-model.ccm"
GB = 2**30


# ------------------------------------------------------------------ which model
def test_each_model_is_pinned_to_one_revision_with_every_file_checked():
    ms = models.catalogue()
    assert [m.name for m in ms] == ["qwen3.8-27b", "qwen3.6-35b-a3b", "qwen3.5-9b", "qwen3.5-4b", "qwen3.5-2b", "qwen3.5-0.8b"]
    assert [m.memory_gb for m in ms] == sorted((m.memory_gb for m in ms), reverse=True)    # biggest first
    for m in ms:
        assert m.repo.startswith("Qwen/") and len(m.revision) == 40 and int(m.revision, 16) >= 0
        assert m.bits in (4, 8) and m.download_bytes > 0
        names = {f["name"] for f in m.files}
        assert {"config.json", "tokenizer.json"} <= names and any(n.endswith(".safetensors") for n in names)
        for f in m.files:
            assert len(f["sha256"]) == 64 and int(f["sha256"], 16) >= 0 and int(f["size"]) > 0
            # A plain name: each is downloaded into the model's own folder and nowhere else.
            assert "/" not in f["name"] and "\\" not in f["name"] and not f["name"].startswith(".")


@pytest.mark.parametrize("gb, name", [(128, "qwen3.8-27b"), (48, "qwen3.8-27b"), (47, "qwen3.8-27b"),
                                      (46, "qwen3.6-35b-a3b"), (32, "qwen3.6-35b-a3b"), (31, "qwen3.6-35b-a3b"),
                                      (24, "qwen3.5-9b"), (16, "qwen3.5-9b"), (15, "qwen3.5-9b"), (12, "qwen3.5-4b"),
                                      (8, "qwen3.5-4b"), (6, "qwen3.5-2b"), (4, "qwen3.5-2b"), (3, "qwen3.5-2b"),
                                      (2, "qwen3.5-0.8b")])
def test_the_biggest_model_the_memory_runs_well(gb, name):
    # (A computer said to have 32 GB shows a little less: some is the graphics' or the firmware's.)
    assert models.pick(int(gb * GB)).name == name


def test_a_model_the_disk_has_no_room_to_get_ready_gives_way_to_one_it_has(tmp_path, monkeypatch):
    store = models.Store(tmp_path / "models")
    monkeypatch.setattr(store, "free_bytes", lambda: 40 * 10**9)
    # 48 GB of memory runs the 27B model, but getting it ready takes about 72 GB of disk: the 9B fits in 40.
    assert models.pick(48 * GB).name == "qwen3.8-27b"
    assert models.pick(48 * GB, store).name == "qwen3.5-9b"
    # One that's here already is kept, however full the disk is.
    f = store.path(models.by_name("qwen3.6-35b-a3b"))
    f.parent.mkdir(parents=True)
    f.write_bytes(b"x")
    monkeypatch.setattr(store, "free_bytes", lambda: 10**9)
    assert models.pick(64 * GB, store).name == "qwen3.6-35b-a3b"
    assert models.pick(8 * GB, store).name == "qwen3.5-0.8b"      # (the 4B doesn't fit on the disk either)


def test_while_the_chosen_model_is_got_ready_one_already_here_answers(tmp_path, monkeypatch):
    monkeypatch.setattr(models, "memory_bytes", lambda: 64 * GB)
    svc = S.ModelService(SimpleNamespace(models=tmp_path / "models", logs=tmp_path),
                         SimpleNamespace(model="qwen3.8-27b"))
    assert not svc.ready and svc.serving() is None and svc.label == "Qwen3.8 27B"
    small = svc.store.path(models.by_name("qwen3.5-4b"))
    small.parent.mkdir(parents=True)
    small.write_bytes(b"x")
    assert svc.ready and svc.serving().name == "qwen3.5-4b" and svc.label == "Qwen3.5 4B"
    assert svc.file() == str(small)
    svc.store.path(svc.model).write_bytes(b"y")                    # the chosen one is ready: it answers from now
    assert svc.serving() == svc.model and svc.file() == str(svc.store.path(svc.model))


def test_the_setting_names_a_model_or_leaves_it_to_the_memory(monkeypatch):
    monkeypatch.setattr(models, "memory_bytes", lambda: 16 * GB)
    assert models.chosen("auto").name == "qwen3.5-9b" and models.chosen(None).name == "qwen3.5-9b"
    assert models.chosen("qwen3.5-0.8b").name == "qwen3.5-0.8b"
    assert models.chosen("something-else").name == "qwen3.5-9b"
    assert models.label("auto") == "Qwen3.5 9B" and models.label("qwen3.8-27b") == "Qwen3.8 27B"


# ------------------------------------------------------------------ the engine's program
def test_the_engine_comes_with_chat_computer_for_each_processor():
    for arch in ("x86_64", "aarch64"):
        f = models.BIN / f"chat-computer-model-{arch}"
        assert f.is_file() and f.read_bytes()[:4] == b"\x7fELF"
        want = (models.BIN / f"chat-computer-model-{arch}.sha256").read_text().split()[0]
        assert hashlib.sha256(f.read_bytes()).hexdigest() == want


def test_an_engine_that_cant_run_where_it_is_runs_from_a_copy(tmp_path, monkeypatch):
    # An update leaves the files it brings read-only and not runnable (scripts aside): the program is copied to
    # the models' folder, made runnable, and kept the same as the one that came.
    monkeypatch.delenv("CHATCOMPUTER_MODEL_BIN", raising=False)
    monkeypatch.setattr("shutil.which", lambda name: None)
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    shipped = bin_dir / f"chat-computer-model-{platform.machine()}"
    shipped.write_bytes(b"\x7fELF one")
    shipped.chmod(0o644)
    monkeypatch.setattr(models, "BIN", bin_dir)
    monkeypatch.setattr(models, "BUILT", tmp_path / "not-built")
    mdir = tmp_path / "models"
    got = models.engine_program(mdir)
    copy = mdir / "llm" / "chat-computer-model"
    assert got == str(copy) and copy.read_bytes() == b"\x7fELF one" and os.access(copy, os.X_OK)
    shipped.write_bytes(b"\x7fELF two")                 # a newer one came with an update
    assert models.engine_program(mdir) == str(copy) and copy.read_bytes() == b"\x7fELF two"
    copy.write_bytes(b"something else")                  # the copy changed: put back
    assert models.engine_program(mdir) == str(copy) and copy.read_bytes() == b"\x7fELF two"
    monkeypatch.setenv("CHATCOMPUTER_MODEL_BIN", str(copy))
    assert models.engine_program(mdir) == str(copy)


# ------------------------------------------------------------------ getting it ready
class Hub:
    """A stand-in for Hugging Face: serves a repository's files at /<repo>/resolve/<revision>/<name>, with Range
    requests; it can break off a download partway (once) and count what was asked."""

    def __init__(self, files):
        self.files = files
        self.asked = []
        self.break_after = None
        hub = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                name = self.path.rsplit("/", 1)[-1]
                data = hub.files.get(name)
                hub.asked.append((self.path, self.headers.get("Range")))
                if data is None:
                    self.send_error(404)
                    return
                start = 0
                rng = self.headers.get("Range")
                if rng and rng.startswith("bytes="):
                    start = int(rng[6:].split("-")[0])
                body = data[start:]
                self.send_response(206 if start else 200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                if hub.break_after is not None and len(body) > hub.break_after:
                    cut, hub.break_after = hub.break_after, None
                    self.wfile.write(body[:cut])
                    self.wfile.flush()
                    self.connection.shutdown(2)
                    return
                self.wfile.write(body)

        self.httpd = socketserver.ThreadingTCPServer(("127.0.0.1", 0), Handler)
        self.httpd.daemon_threads = True
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.httpd.server_address[1]}"

    def stop(self):
        self.httpd.shutdown()
        self.httpd.server_close()


def _model(files, bits=8, sees=False):
    return models.Model("tiny", "Tiny", "Qwen/Tiny", "0" * 40, 0, bits,
                        tuple({"name": n, "size": len(d), "sha256": hashlib.sha256(d).hexdigest()}
                              for n, d in files.items()), sees)


def _model_file(path, settings: str) -> None:
    """A model file (os/model/src/file.rs) with these settings and a little else, before them."""
    entries = [("tokenizer.tokens", b"x"), ("config", settings.encode())]
    table, data = "", b""
    for name, blob in entries:
        data += bytes(-len(data) % 64)
        table += f"{name} blob - {len(data)} {len(blob)}\n"
        data += blob
    head = b"CCMODEL1" + len(table).to_bytes(4, "little") + table.encode()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(head + bytes(-len(head) % 64) + data)


def _converter(tmp_path):
    """A stand-in for the engine's convert (writes what it was given: the folder's files' names and the bits) and
    find-refusal (says how far it's got, and writes a refusal file's head: nothing to take out)."""
    prog = tmp_path / "convert.py"
    prog.write_text("#!/usr/bin/env python3\nimport os, sys\n"
                    "if sys.argv[1] == 'find-refusal':\n"
                    "    _, cmd, model, flag, out = sys.argv\n"
                    "    assert flag == '--out' and os.path.isfile(model) and model.endswith('.ccm.part')\n"
                    "    for i in range(1, 5):\n"
                    "        print(f'{i}/4 trying layer {i}', file=sys.stderr, flush=True)\n"
                    "    open(out, 'wb').write(b'CCREFUS1' + bytes(8) + (1).to_bytes(4, 'little') + bytes(40))\n"
                    "    print('{}')\n"
                    "    sys.exit(0)\n"
                    "_, cmd, folder, out, flag, bits = sys.argv\n"
                    "assert cmd == 'convert' and flag == '--bits'\n"
                    "open(out, 'w').write(' '.join(sorted(os.listdir(folder))) + ' bits=' + bits)\n")
    prog.chmod(0o755)
    return str(prog)


@pytest.fixture
def hub():
    files = {"config.json": b'{"x": 1}', "tokenizer.json": b"{}" * 10, "model.safetensors": os.urandom(300_000)}
    h = Hub(files)
    yield h
    h.stop()


def test_a_model_is_downloaded_checked_and_converted(tmp_path, monkeypatch, hub):
    monkeypatch.setattr(models, "HUB", hub.url)
    m = _model(hub.files)
    store = models.Store(tmp_path / "models")
    seen = []
    p = models.Progress()
    out = store.prepare(m, p, on_change=lambda pr: seen.append(pr.stage), program=_converter(tmp_path))
    assert out == tmp_path / "models" / "llm" / "tiny.ccm" and store.ready(m) == out
    assert out.read_text() == "config.json model.safetensors tokenizer.json bits=8"
    assert p.stage == "ready" and p.done_bytes == m.download_bytes
    assert "downloading" in seen and "converting" in seen and seen[-1] == "ready"
    # Its refusals were looked for before it was ready (on the converted file, before it took its name).
    assert seen.index("converting") < seen.index("uncensoring") < seen.index("ready")
    assert store.uncensored(m) and store.refusal_path(m) == tmp_path / "models" / "llm" / "tiny.refusal"
    assert not store.downloads(m).exists()                                 # the downloads went once it was made
    assert all(path.startswith("/Qwen/Tiny/resolve/" + "0" * 40 + "/") for path, _ in hub.asked)
    # Ready: nothing is downloaded again.
    hub.asked.clear()
    assert store.prepare(m, models.Progress(), program=_converter(tmp_path)) == out and hub.asked == []


def test_a_download_that_breaks_off_carries_on_where_it_stopped(tmp_path, monkeypatch, hub):
    monkeypatch.setattr(models, "HUB", hub.url)
    monkeypatch.setattr(models.time, "sleep", lambda s: None)
    m = _model(hub.files)
    hub.break_after = 100_000
    store = models.Store(tmp_path / "models")
    store.prepare(m, models.Progress(), program=_converter(tmp_path))
    weights = [r for path, r in hub.asked if path.endswith("model.safetensors")]
    assert weights[0] == "bytes=0-" and weights[1] == "bytes=100000-"      # the second asked only for the rest


def test_a_file_that_comes_down_different_is_refused(tmp_path, monkeypatch, hub):
    monkeypatch.setattr(models, "HUB", hub.url)
    m = _model(hub.files)
    hub.files["model.safetensors"] = b"x" * len(hub.files["model.safetensors"])   # not what was pinned
    store = models.Store(tmp_path / "models")
    p = models.Progress()
    with pytest.raises(RuntimeError, match="different from what Chat Computer pinned"):
        store.prepare(m, p, program=_converter(tmp_path))
    assert p.stage == "failed" and store.ready(m) is None
    assert not (store.downloads(m) / "model.safetensors").exists()
    assert not (store.downloads(m) / "model.safetensors.part").exists()


def test_a_file_made_before_the_part_that_sees_was_kept_is_made_again(tmp_path, monkeypatch, hub):
    """A model that sees, its file made before the engine kept that part (none of it, and no converter said it made
    it): made again, its refusal direction with it; the old one's is replaced. Made again, it isn't again."""
    monkeypatch.setattr(models, "HUB", hub.url)
    m = _model(hub.files, sees=True)
    store = models.Store(tmp_path / "models")
    old = store.path(m)
    _model_file(old, "arch=qwen3_5\nhidden=8\n")
    assert models.settings_of(old) == "arch=qwen3_5\nhidden=8\n"
    assert store.ready(m) == old and store.outdated(m)
    store.refusal_path(m).write_bytes(b"the old file's")
    seen = []
    assert store.prepare(m, models.Progress(), on_change=lambda pr: seen.append(pr.stage), program=_converter(tmp_path)) == old
    assert "downloading" in seen and "converting" in seen and seen[-1] == "ready"
    assert old.read_text() == "config.json model.safetensors tokenizer.json bits=8"     # (the stand-in converter's)
    assert store.refusal_path(m).read_bytes().startswith(b"CCREFUS1") and store.uncensored(m)
    assert not store.refusal_path(m).with_name("tiny.refusal.new").exists() and not store.downloads(m).exists()
    assert not store.outdated(m)
    hub.asked.clear()
    assert store.prepare(m, models.Progress(), program=_converter(tmp_path)) == old and hub.asked == []
    # With the part that sees, or made by a converter that says so, or of a model that doesn't see: not outdated.
    for settings, sees in (("arch=qwen3_5\nvision.depth=2\n", True), ("converter=2\narch=qwen3_5\n", True),
                           ("arch=qwen3_5\n", False)):
        _model_file(old, settings)
        assert not store.outdated(_model(hub.files, sees=sees)), settings
    assert models.settings_of(tmp_path / "nothing") is None


def test_while_a_model_is_made_again_it_answers_then_the_new_one_does(tmp_path, monkeypatch):
    """The model service: an outdated file answers while the new one is made in the background; once it's in
    place (the same name), the server starts again on it."""
    monkeypatch.setattr(models, "memory_bytes", lambda: 64 * GB)
    svc = S.ModelService(SimpleNamespace(models=tmp_path / "models", logs=tmp_path), SimpleNamespace(model="qwen3.5-4b"))
    assert svc.model.sees
    f = svc.store.path(svc.model)
    _model_file(f, "arch=qwen3_5\n")
    started = []
    monkeypatch.setattr(svc, "prepare_in_background", lambda: started.append(True))
    monkeypatch.setattr(models, "engine_program", lambda d=None: "/bin/true")

    class Server:
        def __init__(self, program, model, port, threads, log_file=None, cache=None, uncensored=False):
            self.model, self.uncensored, self.port, self.stopped = model, uncensored, port, False

        async def start(self):
            pass

        async def stop(self):
            self.stopped = True
    monkeypatch.setattr(S, "ModelServer", Server)
    monkeypatch.setattr(svc, "uncensor_in_background", lambda m: None)

    async def run():
        await svc.ensure()
        first = svc.server
        assert started and svc.serving() == svc.model and first.model == str(f)      # it answers meanwhile
        await svc.ensure()
        assert svc.server is first                                                  # (the same file: kept)
        new = f.with_name(f.name + ".made")
        _model_file(new, "converter=2\narch=qwen3_5\nvision.depth=2\n")
        os.replace(new, f)                                                          # made again, in its place
        await svc.ensure()
        assert svc.server is not first and first.stopped and not svc.store.outdated(svc.model)
    asyncio.run(run())


def test_without_room_for_it_nothing_is_downloaded(tmp_path, monkeypatch, hub):
    monkeypatch.setattr(models, "HUB", hub.url)
    m = _model(hub.files)
    store = models.Store(tmp_path / "models")
    monkeypatch.setattr(store, "free_bytes", lambda: 1000)
    p = models.Progress()
    with pytest.raises(RuntimeError, match="free space"):
        store.prepare(m, p, program=_converter(tmp_path))
    assert hub.asked == [] and "free space" in p.words()


def test_progress_reads_as_words():
    p = models.Progress("downloading", 2_500_000_000, 10_000_000_000)
    assert p.words() == "downloading it: 2.5 of 10.0 GB (25%)"
    assert "a few minutes" in models.Progress("converting").words()
    u = models.Progress("uncensoring", step=110, steps=440)
    assert u.words() == "taking its refusals out, so it answers whatever you ask (25%)"


def test_refusals_looked_for_another_way_are_looked_for_again(tmp_path):
    store = models.Store(tmp_path / "models")
    m = _model({"config.json": b"{}"})
    f = store.refusal_path(m)
    f.parent.mkdir(parents=True)
    assert not store.uncensored(m)
    for head, current in [(b"CCREFUS1" + bytes(8) + (1).to_bytes(4, "little"), True),
                          (b"CCREFUS1" + bytes(8) + (2).to_bytes(4, "little"), False),
                          (b"CCMODEL1" + bytes(8) + (1).to_bytes(4, "little"), False), (b"CCREFUS1", False)]:
        f.write_bytes(head + bytes(40))
        assert store.uncensored(m) is current, head


# ------------------------------------------------------------------ its server, with the real engine
def _engine():
    prog = models.BIN / f"chat-computer-model-{platform.machine()}"
    if not prog.is_file():
        pytest.skip(f"no engine for {platform.machine()}")
    if not os.access(prog, os.X_OK):
        pytest.skip("the engine's file isn't runnable here")
    return str(prog)


def _free_port():
    import socket
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


async def _raw(port, head):
    reader, writer = await asyncio.open_connection("127.0.0.1", port)
    writer.write(head.encode())
    await writer.drain()
    data = await reader.read()
    writer.close()
    return data


def test_the_engine_serves_the_model_on_this_computer_only(tmp_path):
    prog, port = _engine(), _free_port()

    async def run():
        srv = S.ModelServer(prog, str(TINY), port, threads=1, log_file=str(tmp_path / "model.log"),
                            cache=str(tmp_path / "cache"))
        await srv.start()
        try:
            assert srv.running and await srv.healthy()
            await srv.start()                                   # running already: nothing to do
            body = {"messages": [{"role": "system", "content": "Be brief."}, {"role": "user", "content": "Hello"}],
                    "max_tokens": 6, "temperature": 0, "think": False}
            pieces = [p async for p in S.stream(port, body)]
            again = [p async for p in S.stream(port, body)]
            # Asked from another site's page, or by another name for this computer (a DNS rebinding): refused.
            other_site = await _raw(port, f"GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n"
                                          "Origin: https://example.com\r\nConnection: close\r\n\r\n")
            other_name = await _raw(port, f"GET /health HTTP/1.1\r\nHost: example.com:{port}\r\n"
                                          "Connection: close\r\n\r\n")
            ours = await _raw(port, f"GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n"
                                    "Origin: https://chatcomputer.ai\r\nConnection: close\r\n\r\n")
            return pieces, again, other_site, other_name, ours
        finally:
            await srv.stop()
            assert not srv.running

    pieces, again, other_site, other_name, ours = asyncio.run(run())
    words = "".join(c["delta"].get("content") or "" for p in pieces for c in p.get("choices", []))
    finish = [c["finish_reason"] for p in pieces for c in p.get("choices", []) if c.get("finish_reason")]
    usage = next(p["usage"] for p in pieces if p.get("usage"))
    assert words and finish == ["length"] and usage["completion_tokens"] == 6
    usage2 = next(p["usage"] for p in again if p.get("usage"))
    assert usage2["prompt_tokens_details"]["cached_tokens"] > 0                # read once, kept for the next
    assert "".join(c["delta"].get("content") or "" for p in again for c in p.get("choices", [])) == words
    assert other_site.startswith(b"HTTP/1.1 403") and other_name.startswith(b"HTTP/1.1 403")
    assert ours.startswith(b"HTTP/1.1 200") and b"access-control-allow-origin: https://chatcomputer.ai" in ours.lower()
    assert list((tmp_path / "cache").glob("*.ccstate"))                          # the system prompt's reading kept


def test_a_request_the_engine_refuses_says_why(tmp_path):
    prog, port = _engine(), _free_port()

    async def run():
        srv = S.ModelServer(prog, str(TINY), port, threads=1)
        await srv.start()
        try:
            with pytest.raises(S.ServerError, match="refused"):
                async for _ in S.stream(port, {"messages": "not a list"}):
                    pass
        finally:
            await srv.stop()
    asyncio.run(run())


def test_the_model_service_starts_its_server_once_the_model_is_ready(tmp_path, monkeypatch):
    prog, port = _engine(), _free_port()
    monkeypatch.setenv("CHATCOMPUTER_MODEL_BIN", prog)
    paths = SimpleNamespace(models=tmp_path / "models", logs=tmp_path)
    tiny = models.Model("tiny", "Tiny", "Qwen/Tiny", "0" * 40, 0, 8, ())
    monkeypatch.setattr(models, "chosen", lambda setting, store=None: tiny)
    svc = S.ModelService(paths, SimpleNamespace(model="auto"), port=port, threads=1)
    prepared = []
    monkeypatch.setattr(svc, "prepare_in_background", lambda: prepared.append(True))

    async def run():
        with pytest.raises(S.NotReady):
            await svc.ensure()                     # not there yet: it's being got ready
        assert prepared and not svc.ready
        f = svc.store.path(tiny)
        f.parent.mkdir(parents=True)
        f.write_bytes(TINY.read_bytes())
        try:
            assert svc.ready and await svc.ensure() == port
            status, body = await S.request(port, "GET", "/health")
            return status, json.loads(body)
        finally:
            await svc.stop()
    status, health = asyncio.run(run())
    assert status == 200 and health["ok"] is True
    assert (tmp_path / "model.log").read_text().startswith("serving tiny")


def test_the_engine_finds_a_models_refusals_and_serves_it_with_them_taken_out(tmp_path):
    """The real engine's find-refusal on the tiny model (its random words refuse nothing: there's nothing to take
    out, and the file says so), then serving it: the file beside the model's is read."""
    prog, port = _engine(), _free_port()
    store = models.Store(tmp_path / "models")
    tiny = models.Model("tiny", "Tiny", "Qwen/Tiny", "0" * 40, 0, 8, ())
    f = store.path(tiny)
    f.parent.mkdir(parents=True)
    f.write_bytes(TINY.read_bytes())
    p, seen = models.Progress(), []
    assert store.uncensor(tiny, f, p, on_change=lambda pr: seen.append((pr.step, pr.steps)), program=prog, threads=1)
    assert store.uncensored(tiny) and p.stage == "uncensoring" and p.step == p.steps > 0
    head = store.refusal_path(tiny).read_bytes()
    assert head[:8] == b"CCREFUS1" and len(head) == 60                    # its head only: no direction
    number = [int.from_bytes(head[i:i + 4], "little") for i in range(16, 60, 4)]
    assert number[:5] == [1, 0, 0, 0, 12]       # this way of finding them, no direction, strength 0, 12 measured

    async def run():
        srv = S.ModelServer(prog, str(f), port, threads=1, log_file=str(tmp_path / "model.log"), uncensored=True)
        try:
            await srv.start()
            return json.loads((await S.request(port, "GET", "/health"))[1])
        finally:
            await srv.stop()
    health = asyncio.run(run())
    assert health["uncensored"] == {"taken_out": False, "checked": 12, "refused_before": 0, "refused_after": 0}
    assert "nothing to take out" in (tmp_path / "model.log").read_text()
    # One found for another model file isn't used.
    other = tmp_path / "models" / "llm" / "other.ccm"
    other.write_bytes(TINY.read_bytes() + bytes(64))
    other.with_suffix(".refusal").write_bytes(head)
    log = tmp_path / "other.log"

    async def run_other():
        srv = S.ModelServer(prog, str(other), port, threads=1, log_file=str(log))
        try:
            await srv.start()
            return json.loads((await S.request(port, "GET", "/health"))[1])
        finally:
            await srv.stop()
    assert asyncio.run(run_other())["uncensored"] is None
    assert "isn't used: found for another model file" in log.read_text()


def test_a_model_got_ready_before_has_its_refusals_found_while_it_answers(tmp_path, monkeypatch):
    prog, port = _engine(), _free_port()
    monkeypatch.setenv("CHATCOMPUTER_MODEL_BIN", prog)
    paths = SimpleNamespace(models=tmp_path / "models", logs=tmp_path)
    tiny = models.Model("tiny", "Tiny", "Qwen/Tiny", "0" * 40, 0, 8, ())
    monkeypatch.setattr(models, "chosen", lambda setting, store=None: tiny)
    svc = S.ModelService(paths, SimpleNamespace(model="auto"), port=port, threads=1)
    f = svc.store.path(tiny)
    f.parent.mkdir(parents=True)
    f.write_bytes(TINY.read_bytes())

    async def run():
        try:
            assert await svc.ensure() == port and svc.server.uncensored is False    # it answers meanwhile
            first = svc.server
            svc._uncensoring.join(60)
            assert svc.store.uncensored(tiny)
            assert await svc.ensure() == port and svc.server is not first and svc.server.uncensored is True
            health = json.loads((await S.request(port, "GET", "/health"))[1])
            again = svc.server
            assert await svc.ensure() == port and svc.server is again                # (once is enough)
            return health
        finally:
            await svc.stop()
    assert asyncio.run(run())["uncensored"]["checked"] == 12


# ------------------------------------------------------------------ the engine gets a model ready itself
def _https_hub(tmp_path, files):
    """A stand-in for Hugging Face over HTTPS (a test authority's certificate for localhost): a file asked for at
    /<repo>/resolve/<revision>/<name> is sent from /storage/<name>, as Hugging Face sends its files from its storage;
    Range requests; it can break off a download partway (once)."""
    import ssl
    import subprocess
    if not shutil.which("openssl"):
        pytest.skip("no openssl to make the test's certificates")
    d = tmp_path / "pki"
    d.mkdir()
    run = lambda *a: subprocess.run(a, cwd=d, check=True, capture_output=True)  # noqa: E731
    run("openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes", "-keyout", "ca.key",
        "-out", "ca.pem", "-days", "2", "-subj", "/CN=Chat Computer test authority",
        "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign")
    run("openssl", "req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes", "-keyout", "key.pem",
        "-out", "req.csr", "-subj", "/CN=localhost")
    (d / "ext.cnf").write_text("subjectAltName=DNS:localhost\nbasicConstraints=CA:FALSE\n")
    run("openssl", "x509", "-req", "-in", "req.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial",
        "-out", "cert.pem", "-days", "2", "-extfile", "ext.cnf")
    hub = SimpleNamespace(files=files, asked=[], break_after=None, ca=str(d / "ca.pem"))

    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *a):
            pass

        def do_GET(self):
            hub.asked.append((self.path, self.headers.get("Range")))
            name = self.path.rsplit("/", 1)[-1]
            if "/resolve/" in self.path:
                self.send_response(302)
                self.send_header("Location", f"https://localhost:{hub.port}/storage/{name}?sig=1")
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            data = hub.files.get(name.split("?")[0])
            if data is None:
                self.send_error(404)
                return
            start = 0
            rng = self.headers.get("Range")
            if rng and rng.startswith("bytes="):
                start = int(rng[6:].split("-")[0])
            body = data[start:]
            self.send_response(206 if start else 200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            if hub.break_after is not None and len(body) > hub.break_after:
                cut, hub.break_after = hub.break_after, None
                self.wfile.write(body[:cut])
                self.wfile.flush()
                self.connection.shutdown(2)
                return
            self.wfile.write(body)

    httpd = socketserver.ThreadingTCPServer(("127.0.0.1", 0), Handler)
    httpd.daemon_threads = True
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.minimum_version = ssl.TLSVersion.TLSv1_3
    ctx.load_cert_chain(d / "cert.pem", d / "key.pem")
    httpd.socket = ctx.wrap_socket(httpd.socket, server_side=True)
    hub.port = httpd.server_address[1]
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    hub.stop = lambda: (httpd.shutdown(), httpd.server_close())
    return hub


def _prepare(prog, hub, tmp_path, files, *extra):
    import subprocess
    cat = tmp_path / "models.json"
    cat.write_text(json.dumps({"models": [{
        "name": "tiny", "label": "Tiny", "repo": "Qwen/Tiny", "revision": "0" * 40, "memory_gb": 0, "bits": 8,
        "files": [{"name": n, "size": len(b), "sha256": hashlib.sha256(b).hexdigest()} for n, b in files.items()]}]}))
    env = {k: v for k, v in os.environ.items() if k.lower() not in ("https_proxy", "http_proxy", "all_proxy")}
    env.update(CHATCOMPUTER_HUB=f"https://localhost:{hub.port}", CHATCOMPUTER_EXTRA_CA=hub.ca)
    return subprocess.run([prog, "prepare", str(tmp_path / "models"), "--catalogue", str(cat), "--model", "tiny", *extra],
                          capture_output=True, text=True, timeout=120, env=env)


def test_the_engine_downloads_a_model_over_its_own_https(tmp_path):
    """The engine's own prepare: each file over Chat Computer's own TLS (the test's authority trusted as a company's
    proxy's would be), following the hub's redirect to its storage, carried on where it broke off, checked against
    its SHA-256; then converting (these files aren't a model: it says so, and the downloads are kept)."""
    prog = _engine()
    files = {"config.json": b'{"x": 1}', "tokenizer.json": b"{}" * 10, "model.safetensors": os.urandom(400_000)}
    hub = _https_hub(tmp_path, files)
    try:
        hub.break_after = 150_000
        r = _prepare(prog, hub, tmp_path, files)
    finally:
        hub.stop()
    lines = r.stderr.strip().splitlines()
    assert r.returncode == 1 and lines[-2] == "converting" and "the engine doesn't run" in lines[-1], r.stderr[-800:]
    d = tmp_path / "models" / "download-tiny"
    assert {p.name: p.read_bytes() for p in d.iterdir()} == files
    weights = [rng for path, rng in hub.asked if path.startswith("/storage/model.safetensors")]
    assert weights[0] is None and weights[1] == "bytes=150000-"                 # carried on where it broke off
    assert all(path.startswith("/Qwen/Tiny/resolve/" + "0" * 40 + "/") or path.startswith("/storage/") for path, _ in hub.asked)
    assert "downloading" in r.stderr and "converting" in r.stderr and not (tmp_path / "models" / "tiny.ccm").exists()


def test_a_file_that_comes_down_different_is_not_used(tmp_path):
    prog = _engine()
    files = {"config.json": b'{"x": 1}', "model.safetensors": os.urandom(50_000)}
    served = dict(files, **{"model.safetensors": b"x" * 50_000})                # not what was pinned
    hub = _https_hub(tmp_path, served)
    try:
        r = _prepare(prog, hub, tmp_path, files)
    finally:
        hub.stop()
    assert r.returncode == 1 and "came down different from what Chat Computer pinned" in r.stderr
    d = tmp_path / "models" / "download-tiny"
    assert not (d / "model.safetensors").exists() and not (d / "model.safetensors.part").exists()


def test_without_the_hubs_authority_nothing_comes(tmp_path):
    """A server whose certificate no trusted authority signed: refused (Mozilla's authorities don't include the
    test's)."""
    prog = _engine()
    files = {"config.json": b'{"x": 1}'}
    hub = _https_hub(tmp_path, files)
    hub.ca = str(tmp_path / "none.pem")
    try:
        r = _prepare(prog, hub, tmp_path, files)
    finally:
        hub.stop()
    assert r.returncode != 0 and "certificate" in r.stderr.lower() and hub.asked == []

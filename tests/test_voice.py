"""Voice pipeline tests with the real models: Kokoro speaks test sentences, and the listener
(Silero VAD + faster-whisper) has to hear them back correctly."""
import asyncio
import os
import re
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np
import pytest

from chatcomputer.voice.tts import SentenceSplitter, TTS, speakable, _resample

MODELS = Path(os.environ.get("CHATCOMPUTER_TEST_MODELS", str(Path.home() / ".cache" / "chatcomputer-test" / "models")))
KOKORO = MODELS / "kokoro-v1.0.int8.onnx"
VOICES = MODELS / "voices-v1.0.bin"
WHISPER = MODELS / "whisper" / f"faster-whisper-{os.environ.get('CHATCOMPUTER_TEST_STT', 'small.en')}"
have_models = KOKORO.exists() and (WHISPER / "model.bin").exists()


def test_speakable_cleans_markdown_for_the_ear():
    s = speakable("## Results\n**Bitcoin** is at `$67,000`. See [the chart](https://x.com/a) or https://www.coingecko.com/en/coins/bitcoin\n"
                  "- one\n- two\n```python\nprint(1)\n```\n| a | b |\n|---|---|\nDone 🎉")
    assert "#" not in s and "*" not in s and "`" not in s and "print" not in s and "|" not in s and "🎉" not in s
    assert "the chart" in s and "coingecko.com" in s and "Bitcoin is at $67,000." in s


def test_chatcomputer_streams_tell_pipewire_who_they_are():
    # WirePlumber 0.4 switches AirPods to their microphone only for a Communication stream or a listed app
    # (linux/system/etc/wireplumber/policy.lua.d lists "Chat Computer"); PipeWire's ALSA plugin takes both from
    # PIPEWIRE_ALSA, unless someone set it already.
    import json
    from chatcomputer.voice import audio
    assert "PIPEWIRE_ALSA" in os.environ                  # set as soon as the audio code loads
    env: dict[str, str] = {}
    audio.name_streams_for_pipewire(env)
    assert json.loads(env["PIPEWIRE_ALSA"]) == {"application.name": "Chat Computer", "media.role": "Communication"}
    policy = (Path(__file__).resolve().parents[1] / "linux/system/etc/wireplumber/policy.lua.d/51-chatcomputer-policy.lua").read_text()
    assert '"Chat Computer"' in policy
    env = {"PIPEWIRE_ALSA": '{ "node.latency": "512/16000" }'}
    audio.name_streams_for_pipewire(env)
    assert env["PIPEWIRE_ALSA"] == '{ "node.latency": "512/16000" }'


def test_a_voice_or_speed_kokoro_lacks_keeps_the_natural_voice():
    # Kokoro raises for an unknown voice or a speed outside 0.5-2.0, and every sentence fell over to espeak.
    t = TTS(None, None)
    calls = []

    class Kokoro:
        def get_voices(self):
            return ["af_heart", "bf_emma"]

        def create(self, text, voice, speed, lang):
            if voice not in self.get_voices() or not 0.5 <= speed <= 2.0:
                raise ValueError("bad voice or speed")
            calls.append((voice, speed))
            return np.zeros(2400, dtype=np.float32), 24000

    t.kokoro, t.espeak = Kokoro(), None
    t.voice, t.speed = "robot", 5
    assert t.synth("Hello.").size > 0 and calls == [("af_heart", 2.0)]
    t.voice, t.speed = "bf_emma", "fast"
    assert t.synth("Hello.").size > 0 and calls[-1] == ("bf_emma", 1.0)


class _LevelVAD:
    """Speech where the frame is loud, silence where it's quiet."""

    def __call__(self, frame):
        return 0.9 if float(np.abs(frame).mean()) > 0.05 else 0.0

    def reset(self):
        pass


class _RecordingSTT:
    def __init__(self):
        self.calls = []

    def transcribe(self, audio, fast=False):
        self.calls.append((audio.size // 512, fast))
        return f"{audio.size // 512} frames", {}


def _drive(listener_frames, stt, partial_every=None):
    """Pushes (kind, n) frame runs through a Listener with a level VAD, letting the loop run between frames.
    Returns (utterances, partials, calls made by the time the first utterance's end was reached)."""
    from chatcomputer.voice.listener import Listener, ListenerEvents
    got = {"utt": [], "partials": [], "calls_at": []}

    async def go():
        ev = ListenerEvents(on_partial=got["partials"].append, on_utterance=lambda t, s: got["utt"].append(t))
        lis = Listener(_LevelVAD(), stt, ThreadPoolExecutor(1), ev)
        if partial_every is not None:
            lis.PARTIAL_EVERY = partial_every
        lis.attach(asyncio.get_running_loop())
        speech = np.full(512, 0.3, dtype=np.float32)
        quiet = np.zeros(512, dtype=np.float32)
        for kind, n in listener_frames:
            for _ in range(n):
                lis.push(speech if kind == "talk" else quiet)
                got["calls_at"].append(len(stt.calls))
                await asyncio.sleep(0.002)
        for _ in range(100):
            await asyncio.sleep(0.01)
    asyncio.run(go())
    return got


def test_the_answer_is_transcribed_during_the_pause_and_only_once():
    # 0.7 s of silence ends an utterance, but its transcription starts after 0.3 s: by the end it's (nearly) done.
    stt = _RecordingSTT()
    got = _drive([("quiet", 5), ("talk", 30), ("quiet", 30)], stt)
    assert got["utt"] == [stt.calls[0] and f"{stt.calls[0][0]} frames"]
    finals = [c for c in stt.calls if not c[1]]
    assert finals == [(5 + 30 + 3, False)]              # preroll, speech, 3 frames of its silence
    first_final = next(i for i, n in enumerate(got["calls_at"]) if n and not stt.calls[n - 1][1])
    assert first_final < 5 + 30 + 21 - 5                # asked for well before the 21 frames of silence were up


def test_talking_on_after_a_pause_transcribes_everything_said():
    stt = _RecordingSTT()
    got = _drive([("talk", 20), ("quiet", 12), ("talk", 20), ("quiet", 30)], stt)
    # What was transcribed in each pause shows as the live words; the answer covers both parts.
    assert [c for c in stt.calls if not c[1]] == [(20 + 3, False), (20 + 12 + 20 + 3, False)]
    assert got["partials"] == ["23 frames", "55 frames"] and got["utt"] == ["55 frames"]


def test_live_words_are_not_transcribed_from_silence():
    # Partials as often as possible: still none while nobody is talking.
    stt = _RecordingSTT()
    got = _drive([("talk", 25), ("quiet", 30)], stt, partial_every=0.0)
    partials = [n for n, fast in stt.calls if fast]
    # A partial asked for in the silence would hold more frames than were spoken.
    assert partials and max(partials) <= 25 and got["utt"] == ["28 frames"]


def test_partials_wait_longer_when_transcribing_is_slow():
    from chatcomputer.voice.listener import Listener, ListenerEvents
    lis = Listener(_LevelVAD(), _RecordingSTT(), ThreadPoolExecutor(1), ListenerEvents())
    assert lis.partial_every == Listener.PARTIAL_EVERY
    lis.stt_seconds = 2.0
    assert lis.partial_every == 3.0


def test_every_speech_model_downloads_from_a_repository_that_exists():
    # The settings allow any of WHISPER_MODELS; a wrong repository name for one left the voice waiting forever for
    # a download that could never work ("distil" models are Systran/faster-distil-whisper-..., not faster-distil-...).
    from chatcomputer.voice import models as M
    official = pytest.importorskip("faster_whisper.utils")._MODELS
    for name in M.WHISPER_MODELS:
        assert M.stt_repo(name) == official.get(name, M.stt_repo(name)), name
    assert M.stt_repo("distil-large-v3") == "Systran/faster-distil-whisper-large-v3"


def test_speech_model_repositories_without_faster_whispers_own_list(monkeypatch):
    from chatcomputer.voice import models as M
    utils = pytest.importorskip("faster_whisper.utils")
    monkeypatch.setattr(utils, "_MODELS", {})
    assert M.stt_repo("distil-small.en") == "Systran/faster-distil-whisper-small.en"
    assert M.stt_repo("small.en") == "Systran/faster-whisper-small.en"
    assert M.stt_repo("large-v3") == "Systran/faster-whisper-large-v3"


def test_sentence_splitter_streams_sentences():
    sp = SentenceSplitter()
    out = []
    for piece in ["Sure. I opened ", "YouTube for you. Dr. Lee said", " 3.5 is fine! What ", "next?"]:
        out += sp.feed(piece)
    out += sp.flush()
    assert out == ["Sure.", "I opened YouTube for you.", "Dr. Lee said 3.5 is fine!", "What next?"]


def test_sentence_splitter_skips_code_blocks():
    sp = SentenceSplitter()
    out = sp.feed("Here's the code.\n```python\nx = 1. y = 2.\n```\nIt sets x. ") + sp.flush()
    assert out == ["Here's the code.", "It sets x."]


def test_first_long_sentence_is_cut_at_a_comma_to_start_talking_sooner():
    sp = SentenceSplitter(first_chunk_chars=40)
    out = sp.feed("I found three flights from Singapore to Tokyo next Friday, the cheapest leaves at")
    assert out and out[0].endswith("Friday,")


@pytest.fixture(scope="module")
def tts():
    if not KOKORO.exists():
        pytest.skip("Kokoro model not downloaded")
    return TTS(str(KOKORO), str(VOICES), "af_heart", 1.0)


@pytest.fixture(scope="module")
def stt():
    if not (WHISPER / "model.bin").exists():
        pytest.skip("Whisper model not downloaded")
    from chatcomputer.voice.stt import STT
    return STT(str(WHISPER), "en", hint="Claude, Sam Lee, YouTube")


def speech16k(tts, text):
    a = tts.synth(text)
    return _resample(a, 24000, 16000)


def words(s):
    return set(re.findall(r"[a-z0-9']+", s.lower()))


def test_kokoro_speaks(tts):
    t0 = time.time()
    a = tts.synth("Hello Sam Lee. YouTube is open on the screen.")
    took = time.time() - t0
    secs = a.size / 24000
    assert 2.0 < secs < 6.0
    assert np.abs(a).max() > 0.05
    print(f"\nTTS: {secs:.2f}s of speech in {took:.2f}s")


def run_listener(stt, audio, speaking=lambda: False, barge=lambda: True):
    from chatcomputer.voice.listener import Listener, ListenerEvents
    from chatcomputer.voice.models import vad_model
    from chatcomputer.voice.vad import StreamingVAD
    got = {"utt": [], "partials": [], "barge": 0, "discards": 0}

    async def go():
        loop = asyncio.get_running_loop()
        ev = ListenerEvents(on_partial=lambda t: got["partials"].append(t),
                            on_utterance=lambda t, s: got["utt"].append(t),
                            on_barge_in=lambda: got.__setitem__("barge", got["barge"] + 1),
                            on_discard=lambda: got.__setitem__("discards", got["discards"] + 1))
        ex = ThreadPoolExecutor(1)
        lis = Listener(StreamingVAD(str(vad_model())), stt, ex, ev, speaking, barge)
        lis.attach(loop)
        for i in range(0, audio.size, 512):
            lis.push(audio[i:i + 512])
            if i % (512 * 8) == 0:
                await asyncio.sleep(0.01)
        for _ in range(200):  # let transcriptions finish
            await asyncio.sleep(0.05)
            if not any(t for t in asyncio.all_tasks() if t is not asyncio.current_task()):
                break
        # A slow computer (GitHub's Intel Macs) may still be transcribing: wait for the worker without blocking
        # the loop, then let the loop deliver what it heard.
        await loop.run_in_executor(None, ex.shutdown, True)
        for _ in range(40):
            await asyncio.sleep(0.05)
            if not any(t for t in asyncio.all_tasks() if t is not asyncio.current_task()):
                break
    asyncio.run(go())
    return got


def test_listener_hears_two_requests(tts, stt):
    sil = lambda s: np.zeros(int(16000 * s), dtype=np.float32)
    a1 = speech16k(tts, "Open YouTube and play some calm piano music.")
    a2 = speech16k(tts, "What's the weather like in Singapore today?")
    audio = np.concatenate([sil(0.6), a1, sil(1.2), a2, sil(1.2)])
    t0 = time.time()
    got = run_listener(stt, audio)
    took = time.time() - t0
    print(f"\nheard: {got['utt']} partials={len(got['partials'])} in {took:.1f}s for {audio.size/16000:.1f}s of audio")
    assert len(got["utt"]) == 2
    assert {"youtube", "piano", "music"} <= words(got["utt"][0])
    assert {"weather", "singapore"} <= words(got["utt"][1])


def test_noise_is_ignored(stt):
    rng = np.random.default_rng(1)
    noise = (rng.standard_normal(16000 * 4) * 0.01).astype(np.float32)
    got = run_listener(stt, noise)
    assert got["utt"] == []


def test_talking_over_claude_interrupts_with_headphones(tts, stt):
    sil = lambda s: np.zeros(int(16000 * s), dtype=np.float32)
    a = speech16k(tts, "Stop, that's enough, thank you.")
    got = run_listener(stt, np.concatenate([sil(0.3), a, sil(1.0)]), speaking=lambda: True, barge=lambda: True)
    assert got["barge"] == 1 and got["utt"]


def test_loudspeakers_dont_hear_themselves(tts, stt):
    sil = lambda s: np.zeros(int(16000 * s), dtype=np.float32)
    a = speech16k(tts, "This is Chat Computer talking through the speakers.")
    got = run_listener(stt, np.concatenate([sil(0.3), a, sil(1.0)]), speaking=lambda: True, barge=lambda: False)
    assert got["barge"] == 0 and got["utt"] == []

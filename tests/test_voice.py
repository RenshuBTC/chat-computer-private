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

from claudeos.voice.tts import SentenceSplitter, TTS, speakable, _resample

MODELS = Path(os.environ.get("CLAUDEOS_TEST_MODELS", "/tmp/claude-0/-home-claude/3e6478db-57d3-52bd-b4d8-cfceab0784d7/scratchpad/models"))
KOKORO = MODELS / "kokoro-v1.0.int8.onnx"
VOICES = MODELS / "voices-v1.0.bin"
WHISPER = MODELS / "whisper" / f"faster-whisper-{os.environ.get('CLAUDEOS_TEST_STT', 'small.en')}"
have_models = KOKORO.exists() and (WHISPER / "model.bin").exists()


def test_speakable_cleans_markdown_for_the_ear():
    s = speakable("## Results\n**Bitcoin** is at `$67,000`. See [the chart](https://x.com/a) or https://www.coingecko.com/en/coins/bitcoin\n"
                  "- one\n- two\n```python\nprint(1)\n```\n| a | b |\n|---|---|\nDone 🎉")
    assert "#" not in s and "*" not in s and "`" not in s and "print" not in s and "|" not in s and "🎉" not in s
    assert "the chart" in s and "coingecko.com" in s and "Bitcoin is at $67,000." in s


def test_claudeos_streams_tell_pipewire_who_they_are():
    # WirePlumber 0.4 switches AirPods to their microphone only for a Communication stream or a listed app
    # (linux/system/etc/wireplumber/policy.lua.d lists "ClaudeOS"); PipeWire's ALSA plugin takes both from
    # PIPEWIRE_ALSA, unless someone set it already.
    import json
    from claudeos.voice import audio
    assert "PIPEWIRE_ALSA" in os.environ                  # set as soon as the audio code loads
    env: dict[str, str] = {}
    audio.name_streams_for_pipewire(env)
    assert json.loads(env["PIPEWIRE_ALSA"]) == {"application.name": "ClaudeOS", "media.role": "Communication"}
    policy = (Path(__file__).resolve().parents[1] / "linux/system/etc/wireplumber/policy.lua.d/51-claudeos-policy.lua").read_text()
    assert '"ClaudeOS"' in policy
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
    from claudeos.voice.stt import STT
    return STT(str(WHISPER), "en", hint="Claude, Ren Shu, YouTube")


def speech16k(tts, text):
    a = tts.synth(text)
    return _resample(a, 24000, 16000)


def words(s):
    return set(re.findall(r"[a-z0-9']+", s.lower()))


def test_kokoro_speaks(tts):
    t0 = time.time()
    a = tts.synth("Hello Ren Shu. YouTube is open on the screen.")
    took = time.time() - t0
    secs = a.size / 24000
    assert 2.0 < secs < 6.0
    assert np.abs(a).max() > 0.05
    print(f"\nTTS: {secs:.2f}s of speech in {took:.2f}s")


def run_listener(stt, audio, speaking=lambda: False, barge=lambda: True):
    from claudeos.voice.listener import Listener, ListenerEvents
    from claudeos.voice.models import vad_model
    from claudeos.voice.vad import StreamingVAD
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
    a = speech16k(tts, "This is ClaudeOS talking through the speakers.")
    got = run_listener(stt, np.concatenate([sil(0.3), a, sil(1.0)]), speaking=lambda: True, barge=lambda: False)
    assert got["barge"] == 0 and got["utt"] == []

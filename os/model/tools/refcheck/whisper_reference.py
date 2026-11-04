"""The reference for the engine's speech recognition (whisper.rs): Whisper as Chat Computer's app runs it
(faster-whisper's features: the log-mel spectrogram of the sound with a hop of silence after it, padded to 30
seconds with zeros), the network written out here in numpy (float32, from the published safetensors weights:
transformers' Whisper without PyTorch), and greedy decoding with faster-whisper's rules (its prompt, the tokens it
suppresses, a blank or the end suppressed first). Writes, beside OUT_PREFIX:

  .mel.f32      the features (80 or 128 mels by 3000 frames, mel by mel), as the encoder reads them
  .enc.f32      the encoder's output (1500 positions, each as wide as the model)
  .logits.f32   the decoder's scores at each step (after the prompt, one row per token chosen, vocab wide)
  .json         the prompt, the tokens chosen, the text, the chance it heard no speech, the mean log-likelihood

    python whisper_reference.py MODEL_FOLDER SOUND.wav OUT_PREFIX [--faster-whisper CT2_FOLDER] [--language CODE]

--faster-whisper: also transcribed by faster-whisper itself (CTranslate2, float32), whose tokens must be the same.
--language: a model of many languages told which (else the likeliest language token after the start, as
faster-whisper finds it)."""
import json
import math
import sys
import wave

import numpy as np
from safetensors import safe_open

folder, sound, out = sys.argv[1], sys.argv[2], sys.argv[3]
args = sys.argv[4:]
ct2 = args[args.index("--faster-whisper") + 1] if "--faster-whisper" in args else None
language = args[args.index("--language") + 1] if "--language" in args else None

cfg = json.load(open(folder + "/config.json"))
gen = json.load(open(folder + "/generation_config.json"))
weights = safe_open(folder + "/model.safetensors", "np")


def w(name):
    return weights.get_tensor(name).astype(np.float32)


with wave.open(sound) as f:
    assert f.getframerate() == 16000 and f.getnchannels() == 1 and f.getsampwidth() == 2, "16 kHz mono 16-bit"
    audio = np.frombuffer(f.readframes(f.getnframes()), dtype=np.int16).astype(np.float32) / 32768.0

# The features, as faster-whisper makes them.
from faster_whisper.audio import pad_or_trim
from faster_whisper.feature_extractor import FeatureExtractor

n_mels = cfg["num_mel_bins"]
fe = FeatureExtractor(feature_size=n_mels)
mel = pad_or_trim(fe(audio)).astype(np.float32)
assert mel.shape == (n_mels, 3000), mel.shape
mel.tofile(out + ".mel.f32")

erf = np.vectorize(math.erf, otypes=[np.float64])


def gelu(x):
    return (0.5 * x.astype(np.float64) * (1.0 + erf(x.astype(np.float64) / math.sqrt(2.0)))).astype(np.float32)


def layer_norm(x, p):
    m = x.mean(-1, keepdims=True)
    v = ((x - m) ** 2).mean(-1, keepdims=True)
    return (x - m) / np.sqrt(v + 1e-5) * w(p + ".weight") + w(p + ".bias")


def linear(x, p, bias=True):
    y = x @ w(p + ".weight").T
    return y + w(p + ".bias") if bias else y


def conv1d(x, p, stride):
    # x: channels by frames; kernel 3, padding 1.
    k = w(p + ".weight")  # out, in, 3
    xp = np.pad(x, ((0, 0), (1, 1)))
    frames = (x.shape[1] + 2 - 3) // stride + 1
    cols = np.stack([xp[:, i * stride:i * stride + 3].reshape(-1) for i in range(frames)], axis=1)  # in*3, frames
    return k.reshape(k.shape[0], -1) @ cols + w(p + ".bias")[:, None]


d = cfg["d_model"]
heads_enc = cfg["encoder_attention_heads"]
heads_dec = cfg["decoder_attention_heads"]


def attend(q, k, v, heads, causal=False):
    t, s = q.shape[0], k.shape[0]
    hd = d // heads
    q = q.reshape(t, heads, hd).transpose(1, 0, 2) * hd ** -0.5
    k = k.reshape(s, heads, hd).transpose(1, 0, 2)
    v = v.reshape(s, heads, hd).transpose(1, 0, 2)
    scores = q @ k.transpose(0, 2, 1)
    if causal:
        # (Query i, at position s - t + i, sees keys up to there.)
        mask = np.arange(s)[None, :] > (np.arange(t)[:, None] + s - t)
        scores = np.where(mask[None], -np.inf, scores)
    scores = scores - scores.max(-1, keepdims=True)
    p = np.exp(scores)
    p /= p.sum(-1, keepdims=True)
    return (p @ v).transpose(1, 0, 2).reshape(t, d)


# The encoder.
x = gelu(conv1d(mel, "model.encoder.conv1", 1))
x = gelu(conv1d(x, "model.encoder.conv2", 2)).T  # 1500, d
x = x + w("model.encoder.embed_positions.weight")[: x.shape[0]]
for i in range(cfg["encoder_layers"]):
    p = f"model.encoder.layers.{i}."
    h = layer_norm(x, p + "self_attn_layer_norm")
    a = attend(linear(h, p + "self_attn.q_proj"), linear(h, p + "self_attn.k_proj", False), linear(h, p + "self_attn.v_proj"), heads_enc)
    x = x + linear(a, p + "self_attn.out_proj")
    h = layer_norm(x, p + "final_layer_norm")
    x = x + linear(gelu(linear(h, p + "fc1")), p + "fc2")
enc = layer_norm(x, "model.encoder.layer_norm").astype(np.float32)
enc.tofile(out + ".enc.f32")

# The decoder: the encoder's keys and values for each layer once, then a token at a time.
cross = []
for i in range(cfg["decoder_layers"]):
    p = f"model.decoder.layers.{i}.encoder_attn."
    cross.append((linear(enc, p + "k_proj", False), linear(enc, p + "v_proj")))
emb = w("model.decoder.embed_tokens.weight")
pos = w("model.decoder.embed_positions.weight")
cache = [(np.zeros((0, d), np.float32), np.zeros((0, d), np.float32)) for _ in range(cfg["decoder_layers"])]


def decode(tokens, start):
    """The scores after each of `tokens` (at positions start.. on), the cache kept."""
    x = emb[tokens] + pos[start:start + len(tokens)]
    for i in range(cfg["decoder_layers"]):
        p = f"model.decoder.layers.{i}."
        h = layer_norm(x, p + "self_attn_layer_norm")
        k = np.concatenate([cache[i][0], linear(h, p + "self_attn.k_proj", False)])
        v = np.concatenate([cache[i][1], linear(h, p + "self_attn.v_proj")])
        cache[i] = (k, v)
        x = x + linear(attend(linear(h, p + "self_attn.q_proj"), k, v, heads_dec, causal=True), p + "self_attn.out_proj")
        h = layer_norm(x, p + "encoder_attn_layer_norm")
        x = x + linear(attend(linear(h, p + "encoder_attn.q_proj"), cross[i][0], cross[i][1], heads_dec), p + "encoder_attn.out_proj")
        h = layer_norm(x, p + "final_layer_norm")
        x = x + linear(gelu(linear(h, p + "fc1")), p + "fc2")
    return layer_norm(x, "model.decoder.layer_norm") @ emb.T


# faster-whisper's prompt and suppressed tokens (its own code, over this model's tokenizer).
import tokenizers
from faster_whisper.tokenizer import Tokenizer
from faster_whisper.transcribe import get_suppressed_tokens

multilingual = cfg["vocab_size"] >= 51865
hf_tokenizer = tokenizers.Tokenizer.from_file(folder + "/tokenizer.json")
if multilingual and language is None:
    # The likeliest language token after the start of a transcript.
    probe = Tokenizer(hf_tokenizer, True, task="transcribe", language="en")
    first = decode(np.array([probe.sot]), 0)[-1]
    codes = {token: name[2:-2] for name, token in gen["lang_to_id"].items()}
    language = codes[max(codes, key=lambda t: first[t])]
    cache = [(np.zeros((0, d), np.float32), np.zeros((0, d), np.float32)) for _ in range(cfg["decoder_layers"])]
tok = Tokenizer(hf_tokenizer, multilingual, task="transcribe" if multilingual else None,
                language=(language or "en") if multilingual else None)
prompt = list(tok.sot_sequence) + [tok.no_timestamps]
suppress = list(get_suppressed_tokens(tok, [-1]))
eot, blank = tok.eot, tok.encode(" ")[0]
timestamps = tok.timestamp_begin

scores = decode(np.array(prompt), 0)
# The chance it heard no speech: at the start of the transcript, as faster-whisper reads it.
first = scores[prompt.index(tok.sot)].astype(np.float64)
no_speech = float(np.exp(first[tok.no_speech] - first.max()) / np.exp(first - first.max()).sum())
chosen, rows, total = [], [], 0.0
logits = scores[-1]
while True:
    l = logits.astype(np.float64).copy()
    l[suppress] = -np.inf
    l[timestamps:] = -np.inf
    if not chosen:
        l[[blank, eot]] = -np.inf
    rows.append(logits.astype(np.float32))
    t = int(np.argmax(l))
    total += float(l[t] - (l.max() + np.log(np.exp(l - l.max()).sum())))
    if t == eot or len(prompt) + len(chosen) >= 448:
        break
    chosen.append(t)
    logits = decode(np.array([t]), len(prompt) + len(chosen) - 1)[-1]
np.array(rows, np.float32).tofile(out + ".logits.f32")
text = tok.decode(chosen)
result = {"prompt": prompt, "tokens": chosen, "text": text, "language": language or "en", "no_speech": no_speech,
          "avg_logprob": total / (len(chosen) + 1), "suppress": suppress, "eot": eot, "blank": blank,
          "timestamps": timestamps, "sot": tok.sot, "no_speech_token": tok.no_speech}
print(json.dumps({k: result[k] for k in ("tokens", "text", "no_speech", "avg_logprob", "language")}, ensure_ascii=False))

if ct2:
    from faster_whisper import WhisperModel

    m = WhisperModel(ct2, device="cpu", compute_type="float32", cpu_threads=2)
    segs, _ = m.transcribe(audio, language="en", beam_size=1, best_of=1, temperature=0.0,
                           condition_on_previous_text=False, without_timestamps=True, vad_filter=False,
                           suppress_blank=True)
    segs = list(segs)
    theirs = [t for s in segs for t in s.tokens]
    result["faster_whisper"] = {"tokens": theirs, "text": "".join(s.text for s in segs),
                                "no_speech": [s.no_speech_prob for s in segs], "avg_logprob": [s.avg_logprob for s in segs]}
    print("faster-whisper:", json.dumps(result["faster_whisper"]))
    print("the same tokens" if theirs == chosen else "DIFFERENT tokens")
json.dump(result, open(out + ".json", "w"))

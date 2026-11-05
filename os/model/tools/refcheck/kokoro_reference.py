"""The reference for the engine's voice (voice.rs, being written): Kokoro-82M (hexgrad's, Apache 2.0) written out
here in numpy, in float64, from its published checkpoint (kokoro-v1_0.pth, read without PyTorch) and a voice
(voices/NAME.pt), as kokoro's own KModel.forward_with_tokens runs it: PL-BERT (ALBERT), the prosody predictor
(durations, then the pitch and energy curves), the text encoder, and the decoder (iSTFTNet: the harmonic source
from the pitch, upsampling, AdaIN residual blocks with Snake, an inverse STFT), 24,000 samples a second.

Its noise (the source's, which is random in kokoro) is the engine's own deterministic noise instead (splitmix64,
Box-Muller), the same numbers in the same order, so the waveforms can be compared sample for sample. Writes, beside
OUT_PREFIX:

  .json         the phonemes' ids, each one's duration (in frames of 600 samples), the frames
  .f0.f32       the pitch curve (two values a frame), .n.f32 the energy curve
  .audio.f32    the waveform (floats), and .wav (16-bit)

    python kokoro_reference.py KOKORO_FOLDER VOICE.pt PHONEMES OUT_PREFIX [--speed S] [--seed N]

PHONEMES: the phonemes themselves (misaki's, as kokoro reads them), or @FILE for a file of them.

Checked against Kokoro's own published samples (its repository's samples/af_heart_0.wav and af_heart_1.wav, from the
phonemes its SAMPLES.md gives, with voices/af_heart.pt): the same number of samples (118,800 and 258,600: the
durations the same), and waveforms that agree to a correlation of 0.994, the rest being the source's random noise."""
import json
import math
import pickle
import sys
import wave
import zipfile
from collections import OrderedDict

import numpy as np

folder, voice_path, phonemes, out = sys.argv[1:5]
args = sys.argv[5:]
speed = float(args[args.index("--speed") + 1]) if "--speed" in args else 1.0
seed = int(args[args.index("--seed") + 1]) if "--seed" in args else 0
if phonemes.startswith("@"):
    phonemes = open(phonemes[1:], encoding="utf-8").read().strip()


# ---------------------------------------------------------------- PyTorch's checkpoint, without PyTorch
def load_pth(path):
    zf = zipfile.ZipFile(path)
    names = zf.namelist()
    root = names[0].split("/")[0]
    pkl = next(n for n in names if n.endswith("data.pkl"))
    dtypes = {"FloatStorage": np.float32, "HalfStorage": np.float16, "LongStorage": np.int64, "IntStorage": np.int32,
              "DoubleStorage": np.float64}

    class Storage:
        def __init__(self, name):
            self.name = name

    def rebuild(storage, offset, size, stride, *rest):
        dtype, key = storage
        a = np.frombuffer(zf.read(f"{root}/data/{key}"), dtype=dtype)
        if not size:
            return a[offset:offset + 1].reshape(()).copy()
        return np.lib.stride_tricks.as_strided(a[offset:], tuple(size), tuple(s * a.itemsize for s in stride)).copy()

    class Unpickler(pickle.Unpickler):
        def find_class(self, mod, name):
            if (mod, name) == ("torch._utils", "_rebuild_tensor_v2"):
                return rebuild
            if (mod, name) == ("torch._utils", "_rebuild_parameter"):
                return lambda data, *rest: data
            if mod == "torch" and name.endswith("Storage"):
                return Storage(name)
            if (mod, name) == ("collections", "OrderedDict"):
                return OrderedDict
            raise pickle.UnpicklingError(f"{mod}.{name}")

        def persistent_load(self, pid):
            _, storage, key, _, _ = pid
            return dtypes[storage.name], key

    return Unpickler(zf.open(pkl)).load()


cfg = json.load(open(folder + "/config.json", encoding="utf-8"))
ckpt = load_pth(folder + "/kokoro-v1_0.pth")
W = {}
for part, sd in ckpt.items():
    for k, v in sd.items():
        W[part + "." + (k[7:] if k.startswith("module.") else k)] = v.astype(np.float64)
pack = load_pth(voice_path).astype(np.float64)


def w(name):
    return W[name]


def folded(p):
    """A weight-normed convolution's weight: g * v / |v|, the norm over all but the first dimension."""
    if p + ".weight" in W:
        return W[p + ".weight"]
    g, v = W[p + ".weight_g"], W[p + ".weight_v"]
    norm = np.sqrt((v ** 2).reshape(v.shape[0], -1).sum(1)).reshape((-1,) + (1,) * (v.ndim - 1))
    return g * v / norm


# ---------------------------------------------------------------- the pieces
def linear(x, p):
    return x @ w(p + ".weight").T + w(p + ".bias")


def layer_norm(x, g=None, b=None, eps=1e-5):
    m = x.mean(-1, keepdims=True)
    v = ((x - m) ** 2).mean(-1, keepdims=True)
    y = (x - m) / np.sqrt(v + eps)
    return y if g is None else y * g + b


def sigmoid(x):
    return 1.0 / (1.0 + np.exp(-x))


def leaky(x, slope):
    return np.where(x >= 0, x, x * slope)


def lstm(x, p):
    """PyTorch's bidirectional LSTM (gates i, f, g, o) over x (positions by inputs)."""
    T = x.shape[0]
    out = []
    for suffix in ("", "_reverse"):
        wi, wh = w(f"{p}.weight_ih_l0{suffix}"), w(f"{p}.weight_hh_l0{suffix}")
        b = w(f"{p}.bias_ih_l0{suffix}") + w(f"{p}.bias_hh_l0{suffix}")
        H = wh.shape[1]
        xi = x @ wi.T + b
        h, c = np.zeros(H), np.zeros(H)
        hs = np.zeros((T, H))
        order = range(T) if suffix == "" else range(T - 1, -1, -1)
        for t in order:
            g = xi[t] + wh @ h
            i, f, gg, o = sigmoid(g[:H]), sigmoid(g[H:2 * H]), np.tanh(g[2 * H:3 * H]), sigmoid(g[3 * H:])
            c = f * c + i * gg
            h = o * np.tanh(c)
            hs[t] = h
        out.append(hs)
    return np.concatenate(out, 1)


def conv1d(x, weight, bias=None, stride=1, padding=0, dilation=1):
    """x: channels by positions; weight: out by in by kernel."""
    O, I, K = weight.shape
    L = x.shape[1]
    xp = np.pad(x, ((0, 0), (padding, padding)))
    n = (L + 2 * padding - dilation * (K - 1) - 1) // stride + 1
    cols = np.stack([xp[:, k * dilation:k * dilation + stride * (n - 1) + 1:stride] for k in range(K)], 1)  # I, K, n
    y = np.tensordot(weight, cols, axes=([1, 2], [0, 1]))
    return y if bias is None else y + bias[:, None]


def conv_transpose1d(x, weight, bias, stride, padding, output_padding=0, groups=1):
    """x: channels by positions; weight: in by out/groups by kernel (PyTorch's layout)."""
    I, Og, K = weight.shape
    L = x.shape[1]
    n = (L - 1) * stride - 2 * padding + K + output_padding
    full = np.zeros((Og * groups, (L - 1) * stride + K + output_padding))
    per = I // groups
    for gi in range(groups):
        xs = x[gi * per:(gi + 1) * per]
        ws = weight[gi * per:(gi + 1) * per]  # per, Og, K
        contrib = np.einsum("il,iok->okl", xs, ws)  # Og, K, L
        for k in range(K):
            full[gi * Og:(gi + 1) * Og, k:k + stride * (L - 1) + 1:stride] += contrib[:, k, :]
    y = full[:, padding:padding + n]
    return y + bias[:, None]


def instance_norm(x, eps=1e-5):
    m = x.mean(1, keepdims=True)
    v = ((x - m) ** 2).mean(1, keepdims=True)
    return (x - m) / np.sqrt(v + eps)


def adain(x, s, p):
    h = linear(s, p + ".fc")
    C = x.shape[0]
    return (1 + h[:C, None]) * instance_norm(x) + h[C:, None]


def adain_resblk(x, s, p, upsample=False):
    """AdainResBlk1d."""
    res = leaky(adain(x, s, p + ".norm1"), 0.2)
    if upsample:
        res = conv_transpose1d(res, folded(p + ".pool"), w(p + ".pool.bias"), 2, 1, output_padding=1, groups=res.shape[0])
    res = conv1d(res, folded(p + ".conv1"), w(p + ".conv1.bias"), padding=1)
    res = leaky(adain(res, s, p + ".norm2"), 0.2)
    res = conv1d(res, folded(p + ".conv2"), w(p + ".conv2.bias"), padding=1)
    sc = np.repeat(x, 2, 1) if upsample else x
    if p + ".conv1x1.weight_v" in W:
        sc = conv1d(sc, folded(p + ".conv1x1"))
    return (res + sc) * np.float64(np.float32(1 / math.sqrt(2)))


def snake(x, a):
    return x + (1 / a) * np.sin(a * x) ** 2


def adain_res1(x, s, p, k):
    """AdaINResBlock1 (dilations 1, 3, 5)."""
    for j, d in enumerate((1, 3, 5)):
        a1, a2 = w(f"{p}.alpha1.{j}").reshape(-1, 1), w(f"{p}.alpha2.{j}").reshape(-1, 1)
        xt = snake(adain(x, s, f"{p}.adain1.{j}"), a1)
        xt = conv1d(xt, folded(f"{p}.convs1.{j}"), w(f"{p}.convs1.{j}.bias"), padding=(k * d - d) // 2, dilation=d)
        xt = snake(adain(xt, s, f"{p}.adain2.{j}"), a2)
        xt = conv1d(xt, folded(f"{p}.convs2.{j}"), w(f"{p}.convs2.{j}.bias"), padding=(k - 1) // 2)
        x = xt + x
    return x


def splitmix_gauss(n, seed):
    """The engine's noise: splitmix64's numbers (from `seed`), two at a time made two Gaussians by Box-Muller."""
    m = (n + 1) // 2 * 2
    gamma = np.uint64(0x9E3779B97F4A7C15)
    with np.errstate(over="ignore"):
        z = np.uint64(seed) + np.arange(1, m + 1, dtype=np.uint64) * gamma
        z = (z ^ (z >> np.uint64(30))) * np.uint64(0xBF58476D1CE4E5B9)
        z = (z ^ (z >> np.uint64(27))) * np.uint64(0x94D049BB133111EB)
        z = z ^ (z >> np.uint64(31))
    u = ((z >> np.uint64(11)).astype(np.float64) + 1.0) * 2.0 ** -53
    u1, u2 = u[0::2], u[1::2]
    r = np.sqrt(-2.0 * np.log(u1))
    g = np.empty(m)
    g[0::2] = r * np.cos(2 * math.pi * u2)
    g[1::2] = r * np.sin(2 * math.pi * u2)
    return g[:n]


def interp_linear(x, scale, out_len):
    """F.interpolate(mode='linear', align_corners=False, scale_factor=scale) along the first axis."""
    n = x.shape[0]
    src = (np.arange(out_len) + 0.5) / scale - 0.5
    src = np.maximum(src, 0)
    lo = np.minimum(np.floor(src).astype(int), n - 1)
    hi = np.minimum(lo + 1, n - 1)
    f = (src - lo)[:, None]
    return x[lo] * (1 - f) + x[hi] * f


# ---------------------------------------------------------------- the model
vocab = cfg["vocab"]
ids = [0] + [vocab[c] for c in phonemes if c in vocab] + [0]
T = len(ids)
ref_s = pack[len(phonemes) - 1, 0]
s_pred, s_dec = ref_s[128:], ref_s[:128]

# PL-BERT (ALBERT).
e = w("bert.embeddings.word_embeddings.weight")[ids] + w("bert.embeddings.position_embeddings.weight")[:T] \
    + w("bert.embeddings.token_type_embeddings.weight")[0]
e = layer_norm(e, w("bert.embeddings.LayerNorm.weight"), w("bert.embeddings.LayerNorm.bias"), 1e-12)
h = linear(e, "bert.encoder.embedding_hidden_mapping_in")
L = "bert.encoder.albert_layer_groups.0.albert_layers.0"
heads = cfg["plbert"]["num_attention_heads"]
for _ in range(cfg["plbert"]["num_hidden_layers"]):
    q, k, v = (linear(h, f"{L}.attention.{n}").reshape(T, heads, -1).transpose(1, 0, 2) for n in ("query", "key", "value"))
    sc = q @ k.transpose(0, 2, 1) / math.sqrt(q.shape[-1])
    sc = np.exp(sc - sc.max(-1, keepdims=True))
    sc /= sc.sum(-1, keepdims=True)
    ctx = (sc @ v).transpose(1, 0, 2).reshape(T, -1)
    h1 = layer_norm(h + linear(ctx, f"{L}.attention.dense"), w(f"{L}.attention.LayerNorm.weight"), w(f"{L}.attention.LayerNorm.bias"), 1e-12)
    x = linear(h1, f"{L}.ffn")
    x = 0.5 * x * (1 + np.tanh(math.sqrt(2 / math.pi) * (x + 0.044715 * x ** 3)))
    h = layer_norm(linear(x, f"{L}.ffn_output") + h1, w(f"{L}.full_layer_layer_norm.weight"), w(f"{L}.full_layer_layer_norm.bias"), 1e-12)
d_en = linear(h, "bert_encoder")  # T by 512

# The prosody predictor: durations.
x = np.concatenate([d_en, np.tile(s_pred, (T, 1))], 1)
for i in range(cfg["n_layer"]):
    x = lstm(x, f"predictor.text_encoder.lstms.{2 * i}")
    hh = linear(s_pred, f"predictor.text_encoder.lstms.{2 * i + 1}.fc")
    C = x.shape[1]
    x = (1 + hh[:C]) * layer_norm(x) + hh[C:]
    x = np.concatenate([x, np.tile(s_pred, (T, 1))], 1)
d = x  # T by 640
dur = sigmoid(linear(lstm(d, "predictor.lstm"), "predictor.duration_proj.linear_layer")).sum(1) / speed
pred = np.maximum(np.round(dur), 1).astype(int)  # (numpy rounds half to even, as torch.round)
frames = int(pred.sum())
align = np.repeat(np.arange(T), pred)

# The pitch and energy curves.
en = d[align]  # frames by 640
xs = lstm(en, "predictor.shared").T  # 512 by frames
curves = []
for name in ("F0", "N"):
    y = xs
    for b in range(3):
        y = adain_resblk(y, s_pred, f"predictor.{name}.{b}", upsample=(b == 1))
    curves.append(conv1d(y, w(f"predictor.{name}_proj.weight"), w(f"predictor.{name}_proj.bias"))[0])
F0, N = curves  # 2 x frames each

# The text encoder.
t = w("text_encoder.embedding.weight")[ids].T
for i in range(cfg["n_layer"]):
    t = conv1d(t, folded(f"text_encoder.cnn.{i}.0"), w(f"text_encoder.cnn.{i}.0.bias"), padding=2)
    t = layer_norm(t.T, w(f"text_encoder.cnn.{i}.1.gamma"), w(f"text_encoder.cnn.{i}.1.beta")).T
    t = leaky(t, 0.2)
t = lstm(t.T, "text_encoder.lstm").T  # 512 by T
asr = t[:, align]  # 512 by frames

# The decoder.
f0 = conv1d(F0[None], folded("decoder.F0_conv"), w("decoder.F0_conv.bias"), stride=2, padding=1)
nn = conv1d(N[None], folded("decoder.N_conv"), w("decoder.N_conv.bias"), stride=2, padding=1)
x = adain_resblk(np.concatenate([asr, f0, nn]), s_dec, "decoder.encode")
asr_res = conv1d(asr, folded("decoder.asr_res.0"), w("decoder.asr_res.0.bias"))
for b in range(4):
    x = adain_resblk(np.concatenate([x, asr_res, f0, nn]), s_dec, f"decoder.decode.{b}", upsample=(b == 3))

# The generator: the harmonic source from the pitch.
G = "decoder.generator"
scale = 300
f0_up = np.repeat(F0, scale)  # samples
Ls = f0_up.shape[0]
fn = f0_up[:, None] * np.arange(1, 10)[None]
rad = (fn / 24000.0) % 1
rad_down = interp_linear(rad, 1 / scale, Ls // scale)
phase = np.cumsum(rad_down, 0) * 2 * math.pi
phase = interp_linear(phase * scale, scale, Ls)
uv = (f0_up > 10).astype(np.float64)[:, None]
noise = (uv * 0.003 + (1 - uv) * 0.1 / 3) * splitmix_gauss(Ls * 9, seed).reshape(Ls, 9)
sines = np.sin(phase) * 0.1 * uv + noise
har = np.tanh(sines @ w(f"{G}.m_source.l_linear.weight").T + w(f"{G}.m_source.l_linear.bias"))[:, 0]

n_fft, hop = 20, 5
win = 0.5 - 0.5 * np.cos(2 * math.pi * np.arange(n_fft) / n_fft)
hp = np.pad(har, (n_fft // 2, n_fft // 2), mode="reflect")
nf = Ls // hop + 1
fr = np.stack([hp[i * hop:i * hop + n_fft] * win for i in range(nf)], 1)  # n_fft by frames
spec = np.fft.rfft(fr, axis=0)
re, im = spec.real.copy(), spec.imag.copy()
im[0] = 0.0
im[-1] = 0.0
har_in = np.concatenate([np.sqrt(re ** 2 + im ** 2), np.arctan2(im, re)])  # 22 by frames

ups = cfg["istftnet"]
for i, (u, k) in enumerate(zip(ups["upsample_rates"], ups["upsample_kernel_sizes"])):
    x = leaky(x, 0.1)
    if i + 1 < len(ups["upsample_rates"]):
        sf = int(np.prod(ups["upsample_rates"][i + 1:]))
        xsrc = conv1d(har_in, w(f"{G}.noise_convs.{i}.weight"), w(f"{G}.noise_convs.{i}.bias"), stride=sf, padding=(sf + 1) // 2)
        xsrc = adain_res1(xsrc, s_dec, f"{G}.noise_res.{i}", 7)
    else:
        xsrc = conv1d(har_in, w(f"{G}.noise_convs.{i}.weight"), w(f"{G}.noise_convs.{i}.bias"))
        xsrc = adain_res1(xsrc, s_dec, f"{G}.noise_res.{i}", 11)
    x = conv_transpose1d(x, folded(f"{G}.ups.{i}"), w(f"{G}.ups.{i}.bias"), u, (k - u) // 2)
    if i == len(ups["upsample_rates"]) - 1:
        x = np.concatenate([x[:, 1:2], x], 1)
    x = x + xsrc
    acc = 0
    for j, kk in enumerate(ups["resblock_kernel_sizes"]):
        acc = acc + adain_res1(x, s_dec, f"{G}.resblocks.{i * 3 + j}", kk)
    x = acc / 3
x = leaky(x, 0.01)
x = conv1d(x, folded(f"{G}.conv_post"), w(f"{G}.conv_post.bias"), padding=3)
mag = np.exp(x[:n_fft // 2 + 1])
ph = np.sin(x[n_fft // 2 + 1:])
frames_out = np.fft.irfft(mag * np.exp(1j * ph), n=n_fft, axis=0) * win[:, None]
nfo = frames_out.shape[1]
total = n_fft + hop * (nfo - 1)
y = np.zeros(total)
env = np.zeros(total)
for i in range(nfo):
    y[i * hop:i * hop + n_fft] += frames_out[:, i]
    env[i * hop:i * hop + n_fft] += win ** 2
audio = (y / np.where(env > 1e-11, env, 1.0))[n_fft // 2:total - n_fft // 2]

json.dump({"phonemes": phonemes, "ids": ids, "durations": pred.tolist(), "frames": frames, "samples": int(audio.size),
           "speed": speed, "seed": seed}, open(out + ".json", "w", encoding="utf-8"), ensure_ascii=False)
F0.astype(np.float32).tofile(out + ".f0.f32")
N.astype(np.float32).tofile(out + ".n.f32")
audio.astype(np.float32).tofile(out + ".audio.f32")
with wave.open(out + ".wav", "wb") as f:
    f.setnchannels(1)
    f.setsampwidth(2)
    f.setframerate(24000)
    f.writeframes((np.clip(audio, -1, 1) * 32767).astype("<i2").tobytes())
print(f"{len(ids)} ids, {frames} frames, {audio.size} samples ({audio.size / 24000:.2f} s), peak {abs(audio).max():.3f}")

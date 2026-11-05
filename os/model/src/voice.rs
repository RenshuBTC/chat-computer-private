//! Speaking: Kokoro-82M (hexgrad's, published under the Apache 2.0 licence: StyleTTS 2 with an iSTFTNet decoder),
//! run by the engine as kokoro's own KModel runs it, from phonemes (misaki's, as Kokoro reads them) and a voice:
//!
//! - PL-BERT (ALBERT: one layer's weights, twelve times) over the phonemes;
//! - the prosody predictor: each phoneme's duration (in frames of 600 samples), then the pitch and energy curves
//!   (two values a frame), from LSTMs and AdaIN residual blocks styled by the voice;
//! - the text encoder (convolutions and an LSTM), its phonemes stretched to their frames;
//! - the decoder: AdaIN residual blocks, then the generator, which makes a harmonic source from the pitch (a sine
//!   and its eight overtones, with noise), upsamples by 10 and 6 through residual blocks with Snake, and makes the
//!   sound by an inverse STFT (20 points, a hop of 5): 24,000 samples a second.
//!
//! Its noise (random in kokoro) is the engine's own deterministic noise (splitmix64, Box-Muller), so the same
//! phonemes always sound the same, and the sound can be compared sample for sample with the reference
//! (tools/refcheck/kokoro_reference.py, itself checked against Kokoro's published samples).
//!
//! A step at a time for a caller with other things to do (the own system's kernel, its screen and keyboard going on
//! meanwhile): `begin`, then `advance` until it gives the sound, each step one layer (or one convolution of a
//! residual block). The sound is the same however it's stepped.

use crate::file::{Error, File};
use crate::math;
use crate::net::{Matrix, Pool};
use crate::quant::{self, Prepared};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Samples a second it speaks.
pub const RATE: usize = 24_000;
/// A frame's samples (a phoneme's duration is a number of frames).
pub const FRAME: usize = 600;
/// The most phonemes it reads at once (with the start and end: the ALBERT's 512 positions).
pub const MOST_PHONEMES: usize = 510;

/// The architecture's sizes (Kokoro's config.json).
#[derive(Clone, Debug)]
pub struct Settings {
    pub vocab: usize,
    pub hidden: usize,
    pub style: usize,
    pub layers: usize,
    pub max_dur: usize,
    pub text_kernel: usize,
    pub bert_hidden: usize,
    pub bert_embed: usize,
    pub bert_heads: usize,
    pub bert_layers: usize,
    pub bert_ffn: usize,
    pub bert_positions: usize,
    pub upsample_rates: Vec<usize>,
    pub upsample_kernels: Vec<usize>,
    pub upsample_initial: usize,
    pub resblock_kernels: Vec<usize>,
    pub n_fft: usize,
    pub hop: usize,
}

/// A convolution, its weights a matrix: `outputs` rows of `inputs × k` (each row padded with zeros to a multiple
/// of 32).
struct Conv<'a> {
    m: Matrix<'a>,
    bias: Vec<f32>,
    inputs: usize,
    k: usize,
}

/// A transposed convolution (upsampling by `stride`), its weights a matrix for each phase of the output: `stride ×
/// outputs` rows of `inputs × taps` (taps = k / stride).
struct ConvT<'a> {
    m: Matrix<'a>,
    bias: Vec<f32>,
    inputs: usize,
    outputs: usize,
    k: usize,
    stride: usize,
}

/// A style's scale and shift for each channel (AdaIN, AdaLayerNorm): fc(style), 2 × channels.
struct StyleFc {
    w: Vec<f32>,
    b: Vec<f32>,
}

struct Lstm<'a> {
    /// Forward, then backward: the inputs' weights (4H rows), the hidden state's (4H × H, f32), the biases (summed).
    ih: [Matrix<'a>; 2],
    hh: [Vec<f32>; 2],
    b: [Vec<f32>; 2],
    hidden: usize,
}

/// AdainResBlk1d: (AdaIN, LeakyReLU 0.2, [upsampling], conv 3) twice, plus the shortcut, over √2.
struct ResBlk<'a> {
    norm1: StyleFc,
    conv1: Conv<'a>,
    norm2: StyleFc,
    conv2: Conv<'a>,
    shortcut: Option<Conv<'a>>,
    /// The upsampling's depthwise transposed convolution (each channel's 3 weights), and its bias.
    pool: Option<(Vec<f32>, Vec<f32>)>,
}

/// AdaINResBlock1: three times (AdaIN, Snake, a dilated conv, AdaIN, Snake, a conv), each added to what went in.
struct SnakeBlk<'a> {
    k: usize,
    adain1: Vec<StyleFc>,
    convs1: Vec<Conv<'a>>,
    alpha1: Vec<Vec<f32>>,
    adain2: Vec<StyleFc>,
    convs2: Vec<Conv<'a>>,
    alpha2: Vec<Vec<f32>>,
}

struct Norm {
    w: Vec<f32>,
    b: Vec<f32>,
}

/// The speaking model, over its file's bytes.
pub struct Voice<'a> {
    pub settings: Settings,
    /// Each phoneme it reads, and its id.
    symbols: Vec<(char, u32)>,
    /// The voices: each name, and its styles (510 rows of 256: one for each length of phonemes).
    pub voices: Vec<(String, Vec<f32>)>,
    // ALBERT.
    word: Vec<f32>,
    position: Vec<f32>,
    token_type: Vec<f32>,
    emb_norm: Norm,
    map: Matrix<'a>,
    map_bias: Vec<f32>,
    q: Matrix<'a>,
    q_bias: Vec<f32>,
    k: Matrix<'a>,
    k_bias: Vec<f32>,
    v: Matrix<'a>,
    v_bias: Vec<f32>,
    dense: Matrix<'a>,
    dense_bias: Vec<f32>,
    attn_norm: Norm,
    ffn: Matrix<'a>,
    ffn_bias: Vec<f32>,
    ffn_out: Matrix<'a>,
    ffn_out_bias: Vec<f32>,
    out_norm: Norm,
    bert_encoder: Matrix<'a>,
    bert_encoder_bias: Vec<f32>,
    // The prosody predictor.
    dur_lstms: Vec<Lstm<'a>>,
    dur_norms: Vec<StyleFc>,
    lstm: Lstm<'a>,
    dur_proj: Matrix<'a>,
    dur_proj_bias: Vec<f32>,
    shared: Lstm<'a>,
    f0: Vec<ResBlk<'a>>,
    n: Vec<ResBlk<'a>>,
    f0_proj: (Vec<f32>, f32),
    n_proj: (Vec<f32>, f32),
    // The text encoder.
    text_embed: Vec<f32>,
    text_cnn: Vec<(Conv<'a>, Norm)>,
    text_lstm: Lstm<'a>,
    // The decoder.
    f0_conv: ([f32; 3], f32),
    n_conv: ([f32; 3], f32),
    asr_res: Conv<'a>,
    encode: ResBlk<'a>,
    decode: Vec<ResBlk<'a>>,
    source: (Vec<f32>, f32),
    noise_convs: Vec<Conv<'a>>,
    noise_res: Vec<SnakeBlk<'a>>,
    ups: Vec<ConvT<'a>>,
    resblocks: Vec<SnakeBlk<'a>>,
    post: Conv<'a>,
    /// The Hann window (periodic), and the transform's cos and sin for each bin and point.
    window: Vec<f64>,
    cos: Vec<f64>,
    sin: Vec<f64>,
}

/// Where a step of speaking is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    /// The ALBERT's embeddings (0), its layers (1..=12), then its output to the predictor (13).
    Bert(usize),
    /// The duration encoder's layers, then the durations.
    DurEncoder(usize),
    Durations,
    /// The pitch and energy curves: the shared LSTM, then the F0 and N blocks (each curve's three, then its
    /// projection).
    Shared,
    Curve(usize, usize),
    /// The text encoder, its phonemes stretched to their frames.
    Text,
    /// The decoder's blocks: encode (0), decode (1..=4).
    Decoder(usize),
    /// The harmonic source and its STFT.
    Source,
    /// The generator: an upsampling stage's part.
    Generator(usize, Part),
    /// The last convolution and the inverse STFT.
    Post,
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    /// The source's convolution for this stage, then its residual block (which of three, which half).
    NoiseConv,
    NoiseRes(usize, usize),
    /// The upsampling (and the source added).
    Up,
    /// A residual block (which of three), which of its three, which half; then their average.
    Res(usize, usize, usize),
    Average,
}

/// Some phonemes being spoken (`Voice::begin`, then `advance` until it gives the sound).
pub struct Speaking {
    pub ids: Vec<u32>,
    style: Vec<f32>,
    speed: f32,
    seed: u64,
    stage: Stage,
    /// ALBERT's state (positions × 768), then the predictor's (positions × 640).
    h: Vec<f32>,
    d: Vec<f32>,
    /// Each position's frames, and which position each frame is.
    pub durations: Vec<u32>,
    align: Vec<usize>,
    /// The shared LSTM's output (512 × frames), the curve being made (channels × length), the curves.
    xs: Vec<f32>,
    curve: Vec<f32>,
    pub f0: Vec<f32>,
    pub energy: Vec<f32>,
    /// The text encoder's output stretched to the frames (512 × frames), the decoder's own residual (64 × frames),
    /// the pitch and energy at the frames' rate.
    asr: Vec<f32>,
    asr_res: Vec<f32>,
    f0_down: Vec<f32>,
    n_down: Vec<f32>,
    /// What flows through the decoder and the generator: channels × length.
    x: Vec<f32>,
    x_ch: usize,
    x_len: usize,
    /// The source's spectrum (22 × its frames).
    har: Vec<f32>,
    har_len: usize,
    /// A residual block's state: what goes on (r), the half done (t); the source's (its own residual block's).
    r: Vec<f32>,
    t: Vec<f32>,
    xsrc: Vec<f32>,
    acc: Vec<f32>,
    pub audio: Vec<f32>,
}

#[derive(Clone, Copy)]
struct SyncPtr(*mut f32);
unsafe impl Sync for SyncPtr {}
unsafe impl Send for SyncPtr {}
impl SyncPtr {
    fn get(self) -> *mut f32 {
        self.0
    }
}

impl Speaking {
    /// Where it's got (for a check's timings).
    pub fn stage(&self) -> String {
        alloc::format!("{:?}", self.stage)
    }
}

/// A row's numbers padded to a multiple of 32 (the matrices' rows are).
pub fn padded(n: usize) -> usize {
    n.div_ceil(32) * 32
}

impl<'a> Voice<'a> {
    /// The model in a file (Error::Config: the file isn't a speaking model).
    pub fn load(file: &File<'a>) -> Result<Voice<'a>, Error> {
        let c = file.config()?;
        if c.str("arch") != Some("kokoro") {
            return Err(Error::Config);
        }
        let n = |k: &str| c.usize(&alloc::format!("kokoro.{k}"));
        let list = |k: &str| -> Vec<usize> {
            c.str(&alloc::format!("kokoro.{k}")).map_or(Vec::new(), |s| s.split(',').filter_map(|v| v.trim().parse().ok()).collect())
        };
        let s = Settings {
            vocab: n("vocab")?,
            hidden: n("hidden")?,
            style: n("style")?,
            layers: n("layers")?,
            max_dur: n("max_dur")?,
            text_kernel: n("text_kernel")?,
            bert_hidden: n("bert_hidden")?,
            bert_embed: n("bert_embed")?,
            bert_heads: n("bert_heads")?,
            bert_layers: n("bert_layers")?,
            bert_ffn: n("bert_ffn")?,
            bert_positions: n("bert_positions")?,
            upsample_rates: list("upsample_rates"),
            upsample_kernels: list("upsample_kernels"),
            upsample_initial: n("upsample_initial")?,
            resblock_kernels: list("resblock_kernels"),
            n_fft: n("n_fft")?,
            hop: n("hop")?,
        };
        // (The decoder's own sizes are Kokoro's: what's written below for it is its architecture.)
        if s.hidden != 512 || s.style != 128 || s.upsample_rates.len() != 2 || s.upsample_kernels.len() != 2
            || s.resblock_kernels.len() != 3 || s.upsample_initial != 512 || s.bert_heads == 0
            || !s.bert_hidden.is_multiple_of(s.bert_heads) || s.n_fft != 20 || s.hop != 5
            || s.upsample_rates.iter().product::<usize>() * s.hop != FRAME / 2
            || s.upsample_rates.iter().zip(&s.upsample_kernels).any(|(r, k)| *r == 0 || k % r != 0 || (k - r) % 2 != 0)
        {
            return Err(Error::Config);
        }
        let vector = |name: &str, len: usize| -> Result<Vec<f32>, Error> {
            let v = crate::net::vector(file, name)?;
            if v.len() == len { Ok(v) } else { Err(Error::Damaged) }
        };
        let matrix = |name: &str, rows: usize, cols: usize| crate::net::matrix(file, name, rows, padded(cols));
        let norm = |name: &str, len: usize| -> Result<Norm, Error> {
            Ok(Norm { w: vector(&alloc::format!("{name}.weight"), len)?, b: vector(&alloc::format!("{name}.bias"), len)? })
        };
        let style_fc = |name: &str, channels: usize| -> Result<StyleFc, Error> {
            Ok(StyleFc {
                w: vector(&alloc::format!("{name}.fc.weight"), 2 * channels * s.style)?,
                b: vector(&alloc::format!("{name}.fc.bias"), 2 * channels)?,
            })
        };
        let conv = |name: &str, outputs: usize, inputs: usize, k: usize, bias: bool| -> Result<Conv<'a>, Error> {
            Ok(Conv {
                m: matrix(&alloc::format!("{name}.weight"), outputs, inputs * k)?,
                bias: if bias { vector(&alloc::format!("{name}.bias"), outputs)? } else { Vec::new() },
                inputs,
                k,
            })
        };
        let lstm = |name: &str, inputs: usize, hidden: usize| -> Result<Lstm<'a>, Error> {
            let part = |suffix: &str| -> Result<(Matrix<'a>, Vec<f32>, Vec<f32>), Error> {
                Ok((
                    matrix(&alloc::format!("{name}.weight_ih_l0{suffix}"), 4 * hidden, inputs)?,
                    vector(&alloc::format!("{name}.weight_hh_l0{suffix}"), 4 * hidden * hidden)?,
                    vector(&alloc::format!("{name}.bias_l0{suffix}"), 4 * hidden)?,
                ))
            };
            let (f, b) = (part("")?, part("_reverse")?);
            Ok(Lstm { ih: [f.0, b.0], hh: [f.1, b.1], b: [f.2, b.2], hidden })
        };
        let resblk = |name: &str, inputs: usize, outputs: usize, upsample: bool| -> Result<ResBlk<'a>, Error> {
            Ok(ResBlk {
                norm1: style_fc(&alloc::format!("{name}.norm1"), inputs)?,
                conv1: conv(&alloc::format!("{name}.conv1"), outputs, inputs, 3, true)?,
                norm2: style_fc(&alloc::format!("{name}.norm2"), outputs)?,
                conv2: conv(&alloc::format!("{name}.conv2"), outputs, outputs, 3, true)?,
                shortcut: if inputs != outputs { Some(conv(&alloc::format!("{name}.conv1x1"), outputs, inputs, 1, false)?) } else { None },
                pool: if upsample {
                    Some((vector(&alloc::format!("{name}.pool.weight"), inputs * 3)?, vector(&alloc::format!("{name}.pool.bias"), inputs)?))
                } else {
                    None
                },
            })
        };
        let snakeblk = |name: &str, channels: usize, k: usize| -> Result<SnakeBlk<'a>, Error> {
            let mut b = SnakeBlk { k, adain1: Vec::new(), convs1: Vec::new(), alpha1: Vec::new(), adain2: Vec::new(), convs2: Vec::new(), alpha2: Vec::new() };
            for j in 0..3 {
                b.adain1.push(style_fc(&alloc::format!("{name}.adain1.{j}"), channels)?);
                b.convs1.push(conv(&alloc::format!("{name}.convs1.{j}"), channels, channels, k, true)?);
                b.alpha1.push(vector(&alloc::format!("{name}.alpha1.{j}"), channels)?);
                b.adain2.push(style_fc(&alloc::format!("{name}.adain2.{j}"), channels)?);
                b.convs2.push(conv(&alloc::format!("{name}.convs2.{j}"), channels, channels, k, true)?);
                b.alpha2.push(vector(&alloc::format!("{name}.alpha2.{j}"), channels)?);
            }
            Ok(b)
        };
        let (h, bh, be) = (s.hidden, s.bert_hidden, s.bert_embed);
        let p = "bert.encoder.albert_layer_groups.0.albert_layers.0";
        let mut dur_lstms = Vec::new();
        let mut dur_norms = Vec::new();
        for i in 0..s.layers {
            dur_lstms.push(lstm(&alloc::format!("predictor.text_encoder.lstms.{}", 2 * i), h + s.style, h / 2)?);
            dur_norms.push(style_fc(&alloc::format!("predictor.text_encoder.lstms.{}", 2 * i + 1), h)?);
        }
        let curve = |name: &str| -> Result<Vec<ResBlk<'a>>, Error> {
            Ok(vec![
                resblk(&alloc::format!("predictor.{name}.0"), h, h, false)?,
                resblk(&alloc::format!("predictor.{name}.1"), h, h / 2, true)?,
                resblk(&alloc::format!("predictor.{name}.2"), h / 2, h / 2, false)?,
            ])
        };
        let proj = |name: &str| -> Result<(Vec<f32>, f32), Error> {
            Ok((vector(&alloc::format!("predictor.{name}.weight"), h / 2)?, vector(&alloc::format!("predictor.{name}.bias"), 1)?[0]))
        };
        let three = |name: &str| -> Result<([f32; 3], f32), Error> {
            let w = vector(&alloc::format!("{name}.weight"), 3)?;
            Ok(([w[0], w[1], w[2]], vector(&alloc::format!("{name}.bias"), 1)?[0]))
        };
        let mut text_cnn = Vec::new();
        for i in 0..s.layers {
            text_cnn.push((
                conv(&alloc::format!("text_encoder.cnn.{i}.0"), h, h, s.text_kernel, true)?,
                Norm { w: vector(&alloc::format!("text_encoder.cnn.{i}.1.gamma"), h)?, b: vector(&alloc::format!("text_encoder.cnn.{i}.1.beta"), h)? },
            ));
        }
        let wide = 1024;
        let side = 2 + 64;
        let mut decode = Vec::new();
        for b in 0..4 {
            decode.push(resblk(&alloc::format!("decoder.decode.{b}"), wide + side, if b == 3 { h } else { wide }, b == 3)?);
        }
        let g = "decoder.generator";
        let bins = s.n_fft + 2;
        let mut noise_convs = Vec::new();
        let mut noise_res = Vec::new();
        let mut ups = Vec::new();
        let mut resblocks = Vec::new();
        let stages = s.upsample_rates.len();
        for i in 0..stages {
            let (rate, k) = (s.upsample_rates[i], s.upsample_kernels[i]);
            let (cin, cout) = (s.upsample_initial >> i, s.upsample_initial >> (i + 1));
            ups.push(ConvT {
                m: crate::net::matrix(file, &alloc::format!("{g}.ups.{i}.weight"), rate * cout, padded(cin * (k / rate)))?,
                bias: vector(&alloc::format!("{g}.ups.{i}.bias"), cout)?,
                inputs: cin,
                outputs: cout,
                k,
                stride: rate,
            });
            for (j, &rk) in s.resblock_kernels.iter().enumerate() {
                resblocks.push(snakeblk(&alloc::format!("{g}.resblocks.{}", i * 3 + j), cout, rk)?);
            }
            if i + 1 < stages {
                let stride: usize = s.upsample_rates[i + 1..].iter().product();
                noise_convs.push(conv(&alloc::format!("{g}.noise_convs.{i}"), cout, bins, 2 * stride, true)?);
                noise_res.push(snakeblk(&alloc::format!("{g}.noise_res.{i}"), cout, 7)?);
            } else {
                noise_convs.push(conv(&alloc::format!("{g}.noise_convs.{i}"), cout, bins, 1, true)?);
                noise_res.push(snakeblk(&alloc::format!("{g}.noise_res.{i}"), cout, 11)?);
            }
        }
        let last = s.upsample_initial >> stages;
        // The phonemes: "id symbol" lines.
        let symbols_text = file.get("kokoro.symbols").map(|e| file.data(e)).ok_or(Error::Missing)?;
        let symbols_text = core::str::from_utf8(symbols_text).map_err(|_| Error::Damaged)?;
        let mut symbols = Vec::new();
        for line in symbols_text.split('\n').filter(|l| !l.is_empty()) {
            let (id, sym) = line.split_once(' ').ok_or(Error::Damaged)?;
            let id: u32 = id.parse().map_err(|_| Error::Damaged)?;
            let mut chars = sym.chars();
            let (Some(ch), None) = (chars.next(), chars.next()) else { return Err(Error::Damaged) };
            if id as usize >= s.vocab {
                return Err(Error::Damaged);
            }
            symbols.push((ch, id));
        }
        let mut voices = Vec::new();
        for name in c.str("kokoro.voices").unwrap_or("").split(',').map(str::trim).filter(|v| !v.is_empty()) {
            voices.push((String::from(name), vector(&alloc::format!("kokoro.voice.{name}"), MOST_PHONEMES * 2 * s.style)?));
        }
        if voices.is_empty() {
            return Err(Error::Missing);
        }
        let (nfft, half) = (s.n_fft, s.n_fft / 2);
        let window: Vec<f64> = (0..nfft).map(|k| 0.5 - 0.5 * math::sin_cos(2.0 * core::f64::consts::PI * k as f64 / nfft as f64).1).collect();
        let mut cos = vec![0.0f64; (half + 1) * nfft];
        let mut sin = vec![0.0f64; (half + 1) * nfft];
        for b in 0..=half {
            for t in 0..nfft {
                // (Exactly: the angle's multiple of π/2 found first, so the zeros are zeros.)
                let turns = (b * t) % nfft;
                let (sn, cn) = match (4 * turns).checked_rem(nfft).filter(|r| *r == 0).map(|_| 4 * turns / nfft) {
                    Some(0) => (0.0, 1.0),
                    Some(1) => (1.0, 0.0),
                    Some(2) => (0.0, -1.0),
                    Some(3) => (-1.0, 0.0),
                    _ => math::sin_cos(2.0 * core::f64::consts::PI * turns as f64 / nfft as f64),
                };
                cos[b * nfft + t] = cn;
                sin[b * nfft + t] = sn;
            }
        }
        Ok(Voice {
            symbols,
            voices,
            word: vector("bert.embeddings.word_embeddings.weight", s.vocab * be)?,
            position: vector("bert.embeddings.position_embeddings.weight", s.bert_positions * be)?,
            token_type: vector("bert.embeddings.token_type_embeddings.weight", 2 * be)?[..be].to_vec(),
            emb_norm: norm("bert.embeddings.LayerNorm", be)?,
            map: matrix("bert.encoder.embedding_hidden_mapping_in.weight", bh, be)?,
            map_bias: vector("bert.encoder.embedding_hidden_mapping_in.bias", bh)?,
            q: matrix(&alloc::format!("{p}.attention.query.weight"), bh, bh)?,
            q_bias: vector(&alloc::format!("{p}.attention.query.bias"), bh)?,
            k: matrix(&alloc::format!("{p}.attention.key.weight"), bh, bh)?,
            k_bias: vector(&alloc::format!("{p}.attention.key.bias"), bh)?,
            v: matrix(&alloc::format!("{p}.attention.value.weight"), bh, bh)?,
            v_bias: vector(&alloc::format!("{p}.attention.value.bias"), bh)?,
            dense: matrix(&alloc::format!("{p}.attention.dense.weight"), bh, bh)?,
            dense_bias: vector(&alloc::format!("{p}.attention.dense.bias"), bh)?,
            attn_norm: norm(&alloc::format!("{p}.attention.LayerNorm"), bh)?,
            ffn: matrix(&alloc::format!("{p}.ffn.weight"), s.bert_ffn, bh)?,
            ffn_bias: vector(&alloc::format!("{p}.ffn.bias"), s.bert_ffn)?,
            ffn_out: matrix(&alloc::format!("{p}.ffn_output.weight"), bh, s.bert_ffn)?,
            ffn_out_bias: vector(&alloc::format!("{p}.ffn_output.bias"), bh)?,
            out_norm: norm(&alloc::format!("{p}.full_layer_layer_norm"), bh)?,
            bert_encoder: matrix("bert_encoder.weight", h, bh)?,
            bert_encoder_bias: vector("bert_encoder.bias", h)?,
            dur_lstms,
            dur_norms,
            lstm: lstm("predictor.lstm", h + s.style, h / 2)?,
            dur_proj: matrix("predictor.duration_proj.linear_layer.weight", s.max_dur, h)?,
            dur_proj_bias: vector("predictor.duration_proj.linear_layer.bias", s.max_dur)?,
            shared: lstm("predictor.shared", h + s.style, h / 2)?,
            f0: curve("F0")?,
            n: curve("N")?,
            f0_proj: proj("F0_proj")?,
            n_proj: proj("N_proj")?,
            text_embed: vector("text_encoder.embedding.weight", s.vocab * h)?,
            text_cnn,
            text_lstm: lstm("text_encoder.lstm", h, h / 2)?,
            f0_conv: three("decoder.F0_conv")?,
            n_conv: three("decoder.N_conv")?,
            asr_res: conv("decoder.asr_res.0", 64, h, 1, true)?,
            encode: resblk("decoder.encode", h + 2, wide, false)?,
            decode,
            source: (vector(&alloc::format!("{g}.m_source.l_linear.weight"), 9)?, vector(&alloc::format!("{g}.m_source.l_linear.bias"), 1)?[0]),
            noise_convs,
            noise_res,
            ups,
            resblocks,
            post: conv(&alloc::format!("{g}.conv_post"), bins, last, 7, true)?,
            window,
            cos,
            sin,
            settings: s,
        })
    }

    /// The ids of the phonemes it reads (the others left out), between the start and the end (0).
    pub fn ids(&self, phonemes: &str) -> Vec<u32> {
        let mut ids = vec![0];
        for ch in phonemes.chars() {
            if let Some(&(_, id)) = self.symbols.iter().find(|(c, _)| *c == ch) {
                ids.push(id);
            }
        }
        ids.truncate(MOST_PHONEMES + 1);
        ids.push(0);
        ids
    }

    /// Whether it reads this phoneme.
    pub fn reads(&self, ch: char) -> bool {
        self.symbols.iter().any(|(c, _)| *c == ch)
    }

    /// The voice called `name` (its place among the voices), if there is one.
    pub fn voice(&self, name: &str) -> Option<usize> {
        self.voices.iter().position(|(n, _)| n == name)
    }

    /// The whole of it at once: the sound of `phonemes` in voice `voice`, at `speed` (1: as the voice speaks).
    pub fn speak(&self, phonemes: &str, voice: usize, speed: f32, seed: u64, pool: &dyn Pool) -> Vec<f32> {
        let mut s = self.begin(phonemes, voice, speed, seed);
        loop {
            if self.advance(&mut s, pool) {
                return core::mem::take(&mut s.audio);
            }
        }
    }

    /// Phonemes to speak, a step at a time (`advance`). Their style: the voice's for their length.
    pub fn begin(&self, phonemes: &str, voice: usize, speed: f32, seed: u64) -> Speaking {
        let st = 2 * self.settings.style;
        let count = phonemes.chars().count().clamp(1, MOST_PHONEMES);
        let pack = &self.voices[voice.min(self.voices.len() - 1)].1;
        Speaking {
            ids: self.ids(phonemes),
            style: pack[(count - 1) * st..count * st].to_vec(),
            speed: if speed > 0.0 { speed } else { 1.0 },
            seed,
            stage: Stage::Bert(0),
            h: Vec::new(),
            d: Vec::new(),
            durations: Vec::new(),
            align: Vec::new(),
            xs: Vec::new(),
            curve: Vec::new(),
            f0: Vec::new(),
            energy: Vec::new(),
            asr: Vec::new(),
            asr_res: Vec::new(),
            f0_down: Vec::new(),
            n_down: Vec::new(),
            x: Vec::new(),
            x_ch: 0,
            x_len: 0,
            har: Vec::new(),
            har_len: 0,
            r: Vec::new(),
            t: Vec::new(),
            xsrc: Vec::new(),
            acc: Vec::new(),
            audio: Vec::new(),
        }
    }

    /// A step (a layer, or a convolution of a residual block). True when the sound is ready (in `audio`).
    pub fn advance(&self, s: &mut Speaking, pool: &dyn Pool) -> bool {
        let st = &self.settings;
        let (h, style) = (st.hidden, st.style);
        let s_pred = s.style[style..].to_vec();
        let s_dec = s.style[..style].to_vec();
        match s.stage {
            Stage::Bert(0) => {
                s.h = self.bert_embed(&s.ids, pool);
                s.stage = Stage::Bert(1);
            }
            Stage::Bert(l) if l <= st.bert_layers => {
                self.bert_layer(&mut s.h, s.ids.len(), pool);
                s.stage = Stage::Bert(l + 1);
            }
            Stage::Bert(_) => {
                let t = s.ids.len();
                let d_en = linear(&self.bert_encoder, &self.bert_encoder_bias, &s.h, t, pool);
                s.d = with_style(&d_en, t, h, &s_pred);
                s.h = Vec::new();
                s.stage = Stage::DurEncoder(0);
            }
            Stage::DurEncoder(i) => {
                let t = s.ids.len();
                let mut x = self.run_lstm(&self.dur_lstms[i], &s.d, t, pool);
                let (g, b) = style_scale(&self.dur_norms[i], &s_pred, h);
                for row in x.chunks_exact_mut(h) {
                    layer_norm(row, None, 1e-5);
                    for c in 0..h {
                        row[c] = (1.0 + g[c]) * row[c] + b[c];
                    }
                }
                s.d = with_style(&x, t, h, &s_pred);
                s.stage = if i + 1 < st.layers { Stage::DurEncoder(i + 1) } else { Stage::Durations };
            }
            Stage::Durations => {
                let t = s.ids.len();
                let x = self.run_lstm(&self.lstm, &s.d, t, pool);
                let logits = linear(&self.dur_proj, &self.dur_proj_bias, &x, t, pool);
                s.durations = logits
                    .chunks_exact(st.max_dur)
                    .map(|row| {
                        let sum: f32 = row.iter().map(|&v| math::sigmoid(v)).sum();
                        round_even(sum / s.speed).max(1.0) as u32
                    })
                    .collect();
                s.align = Vec::new();
                for (i, &n) in s.durations.iter().enumerate() {
                    s.align.extend(core::iter::repeat_n(i, n as usize));
                }
                s.stage = Stage::Shared;
            }
            Stage::Shared => {
                let frames = s.align.len();
                let w = h + style;
                let mut en = vec![0.0f32; frames * w];
                for (f, &i) in s.align.iter().enumerate() {
                    en[f * w..(f + 1) * w].copy_from_slice(&s.d[i * w..(i + 1) * w]);
                }
                let x = self.run_lstm(&self.shared, &en, frames, pool);
                s.xs = transpose(&x, frames, h);
                s.curve = s.xs.clone();
                s.stage = Stage::Curve(0, 0);
            }
            Stage::Curve(which, b) => {
                let blocks = if which == 0 { &self.f0 } else { &self.n };
                let len = s.curve.len() / blocks[b].conv1.inputs;
                let (y, _) = self.res_blk(&blocks[b], &s.curve, len, &s_pred, pool);
                s.curve = y;
                if b + 1 < blocks.len() {
                    s.stage = Stage::Curve(which, b + 1);
                } else {
                    let (w, bias) = if which == 0 { &self.f0_proj } else { &self.n_proj };
                    let ch = w.len();
                    let len = s.curve.len() / ch;
                    let mut out = vec![*bias; len];
                    for (c, &wc) in w.iter().enumerate() {
                        for (o, &v) in out.iter_mut().zip(&s.curve[c * len..(c + 1) * len]) {
                            *o += wc * v;
                        }
                    }
                    if which == 0 {
                        s.f0 = out;
                        s.curve = s.xs.clone();
                        s.stage = Stage::Curve(1, 0);
                    } else {
                        s.energy = out;
                        s.curve = Vec::new();
                        s.xs = Vec::new();
                        s.stage = Stage::Text;
                    }
                }
            }
            Stage::Text => {
                let t = s.ids.len();
                let mut x = vec![0.0f32; h * t];
                for (p, &id) in s.ids.iter().enumerate() {
                    for c in 0..h {
                        x[c * t + p] = self.text_embed[id as usize * h + c];
                    }
                }
                for (cv, nm) in &self.text_cnn {
                    let (mut y, _) = conv(cv, &x, t, 1, st.text_kernel / 2, 1, pool);
                    // LayerNorm over the channels at each position, then LeakyReLU 0.2.
                    let mut col = vec![0.0f32; h];
                    for p in 0..t {
                        for c in 0..h {
                            col[c] = y[c * t + p];
                        }
                        layer_norm(&mut col, Some((&nm.w, &nm.b)), 1e-5);
                        for c in 0..h {
                            y[c * t + p] = leaky(col[c], 0.2);
                        }
                    }
                    x = y;
                }
                let lstm_out = self.run_lstm(&self.text_lstm, &transpose(&x, h, t), t, pool);
                let frames = s.align.len();
                let mut asr = vec![0.0f32; h * frames];
                for c in 0..h {
                    for (f, &i) in s.align.iter().enumerate() {
                        asr[c * frames + f] = lstm_out[i * h + c];
                    }
                }
                s.asr = asr;
                s.d = Vec::new();
                s.stage = Stage::Decoder(0);
            }
            Stage::Decoder(0) => {
                let frames = s.align.len();
                s.f0_down = small_conv(&self.f0_conv, &s.f0);
                s.n_down = small_conv(&self.n_conv, &s.energy);
                let (res, _) = conv(&self.asr_res, &s.asr, frames, 1, 0, 1, pool);
                s.asr_res = res;
                let mut x = s.asr.clone();
                x.extend_from_slice(&s.f0_down[..frames]);
                x.extend_from_slice(&s.n_down[..frames]);
                let (y, _) = self.res_blk(&self.encode, &x, frames, &s_dec, pool);
                s.x = y;
                s.x_ch = 1024;
                s.x_len = frames;
                s.asr = Vec::new();
                s.stage = Stage::Decoder(1);
            }
            Stage::Decoder(b) => {
                let frames = s.align.len();
                let mut x = core::mem::take(&mut s.x);
                x.extend_from_slice(&s.asr_res);
                x.extend_from_slice(&s.f0_down[..frames]);
                x.extend_from_slice(&s.n_down[..frames]);
                let blk = &self.decode[b - 1];
                let (y, len) = self.res_blk(blk, &x, frames, &s_dec, pool);
                s.x_ch = y.len() / len;
                s.x = y;
                s.x_len = len;
                s.stage = if b < 4 { Stage::Decoder(b + 1) } else { Stage::Source };
            }
            Stage::Source => {
                let (har, len) = self.source(&s.f0, s.seed);
                s.har = har;
                s.har_len = len;
                s.asr_res = Vec::new();
                s.stage = Stage::Generator(0, Part::NoiseConv);
            }
            Stage::Generator(i, part) => self.generator_step(s, i, part, &s_dec, pool),
            Stage::Post => {
                let len = s.x_len;
                for v in s.x.iter_mut() {
                    *v = leaky(*v, 0.01);
                }
                let (y, n) = conv(&self.post, &s.x, len, 1, 3, 1, pool);
                s.audio = self.istft(&y, n);
                s.x = Vec::new();
                s.har = Vec::new();
                s.stage = Stage::Done;
                return true;
            }
            Stage::Done => return true,
        }
        false
    }

    /// The generator's steps.
    fn generator_step(&self, s: &mut Speaking, i: usize, part: Part, s_dec: &[f32], pool: &dyn Pool) {
        let stages = self.ups.len();
        let k_blocks = self.settings.resblock_kernels.len();
        match part {
            Part::NoiseConv => {
                // (LeakyReLU 0.1 on what's upsampled.)
                for v in s.x.iter_mut() {
                    *v = leaky(*v, 0.1);
                }
                let c = &self.noise_convs[i];
                let (stride, pad) = if i + 1 < stages {
                    let sf: usize = self.settings.upsample_rates[i + 1..].iter().product();
                    (sf, sf.div_ceil(2))
                } else {
                    (1, 0)
                };
                let (y, _) = conv(c, &s.har, s.har_len, stride, pad, 1, pool);
                s.xsrc = y;
                s.stage = Stage::Generator(i, Part::NoiseRes(0, 0));
            }
            Part::NoiseRes(j, half) => {
                let blk = &self.noise_res[i];
                let ch = blk.alpha1[0].len();
                let len = s.xsrc.len() / ch;
                let mut r = core::mem::take(&mut s.xsrc);
                let mut t = core::mem::take(&mut s.t);
                self.snake_half(blk, j, half, &mut r, &mut t, len, s_dec, pool);
                s.xsrc = r;
                s.t = t;
                s.stage = Stage::Generator(i, if half == 0 { Part::NoiseRes(j, 1) } else if j < 2 { Part::NoiseRes(j + 1, 0) } else { Part::Up });
            }
            Part::Up => {
                let up = &self.ups[i];
                let (mut y, mut len) = conv_transpose(up, &s.x, s.x_len, pool);
                if i + 1 == stages {
                    // (ReflectionPad1d((1, 0)): the second value before the first.)
                    let ch = up.outputs;
                    let mut padded = vec![0.0f32; ch * (len + 1)];
                    for c in 0..ch {
                        padded[c * (len + 1)] = y[c * len + 1];
                        padded[c * (len + 1) + 1..(c + 1) * (len + 1)].copy_from_slice(&y[c * len..(c + 1) * len]);
                    }
                    y = padded;
                    len += 1;
                }
                for (a, b) in y.iter_mut().zip(&s.xsrc) {
                    *a += b;
                }
                s.xsrc = Vec::new();
                s.x = y;
                s.x_ch = up.outputs;
                s.x_len = len;
                s.acc = vec![0.0f32; s.x.len()];
                s.r = s.x.clone();
                s.stage = Stage::Generator(i, Part::Res(0, 0, 0));
            }
            Part::Res(b, j, half) => {
                let blk = &self.resblocks[i * k_blocks + b];
                let mut r = core::mem::take(&mut s.r);
                let mut t = core::mem::take(&mut s.t);
                self.snake_half(blk, j, half, &mut r, &mut t, s.x_len, s_dec, pool);
                s.t = t;
                s.stage = if half == 0 {
                    s.r = r;
                    Stage::Generator(i, Part::Res(b, j, 1))
                } else if j < 2 {
                    s.r = r;
                    Stage::Generator(i, Part::Res(b, j + 1, 0))
                } else {
                    // This block's done: added up, and the next starts from the same input.
                    for (a, v) in s.acc.iter_mut().zip(&r) {
                        *a += v;
                    }
                    if b + 1 < k_blocks {
                        s.r = s.x.clone();
                        Stage::Generator(i, Part::Res(b + 1, 0, 0))
                    } else {
                        Stage::Generator(i, Part::Average)
                    }
                };
            }
            Part::Average => {
                let n = k_blocks as f32;
                s.x = core::mem::take(&mut s.acc).into_iter().map(|v| v / n).collect();
                s.r = Vec::new();
                s.t = Vec::new();
                s.stage = if i + 1 < stages { Stage::Generator(i + 1, Part::NoiseConv) } else { Stage::Post };
            }
        }
    }

    /// Half of one of a Snake block's three: AdaIN, Snake and its first convolution (into t), or the second's, added to r.
    #[allow(clippy::too_many_arguments)]
    fn snake_half(&self, blk: &SnakeBlk, j: usize, half: usize, r: &mut [f32], t: &mut Vec<f32>, len: usize, s_dec: &[f32], pool: &dyn Pool) {
        let d = [1usize, 3, 5][j];
        if half == 0 {
            let mut x = r.to_vec();
            adain_snake(&mut x, len, &blk.adain1[j], &blk.alpha1[j], s_dec, pool);
            let (y, _) = conv(&blk.convs1[j], &x, len, 1, (blk.k * d - d) / 2, d, pool);
            *t = y;
        } else {
            let x = t;
            adain_snake(x, len, &blk.adain2[j], &blk.alpha2[j], s_dec, pool);
            let (y, _) = conv(&blk.convs2[j], x, len, 1, (blk.k - 1) / 2, 1, pool);
            for (a, b) in r.iter_mut().zip(&y) {
                *a += b;
            }
            x.clear();
        }
    }

    /// AdainResBlk1d over x (inputs × len): its output (outputs × len, or 2 len upsampled), and its length.
    fn res_blk(&self, b: &ResBlk, x: &[f32], len: usize, style: &[f32], pool: &dyn Pool) -> (Vec<f32>, usize) {
        let inputs = b.conv1.inputs;
        let mut res = x.to_vec();
        let (g, bb) = style_scale(&b.norm1, style, inputs);
        instance_norm_styled(&mut res, len, &g, &bb, 0.2, pool);
        let (mut res, len2) = match &b.pool {
            Some((w, bias)) => (depthwise_up(&res, inputs, len, w, bias), 2 * len),
            None => (res, len),
        };
        let (mut y, _) = conv(&b.conv1, &res, len2, 1, 1, 1, pool);
        res.clear();
        let outputs = y.len() / len2;
        let (g, bb) = style_scale(&b.norm2, style, outputs);
        instance_norm_styled(&mut y, len2, &g, &bb, 0.2, pool);
        let (mut y, _) = conv(&b.conv2, &y, len2, 1, 1, 1, pool);
        // The shortcut: upsampled (nearest), then the 1×1 convolution where the channels change.
        let up;
        let sc: &[f32] = if b.pool.is_some() {
            let mut u = vec![0.0f32; inputs * len2];
            for c in 0..inputs {
                for p in 0..len {
                    u[c * len2 + 2 * p] = x[c * len + p];
                    u[c * len2 + 2 * p + 1] = x[c * len + p];
                }
            }
            up = u;
            &up
        } else {
            x
        };
        let shortcut;
        let sc: &[f32] = match &b.shortcut {
            Some(c) => {
                shortcut = conv(c, sc, len2, 1, 0, 1, pool).0;
                &shortcut
            }
            None => sc,
        };
        // (Over √2, as torch.rsqrt(2) in float32 is.)
        let scale = 0.707_106_77f32;
        for (a, b) in y.iter_mut().zip(sc) {
            *a = (*a + b) * scale;
        }
        (y, len2)
    }

    /// The ALBERT's embeddings, layer-normed and mapped to its width: positions × 768.
    fn bert_embed(&self, ids: &[u32], pool: &dyn Pool) -> Vec<f32> {
        let be = self.settings.bert_embed;
        let t = ids.len();
        let mut e = vec![0.0f32; t * be];
        for (p, &id) in ids.iter().enumerate() {
            let row = &mut e[p * be..(p + 1) * be];
            for c in 0..be {
                row[c] = self.word[id as usize * be + c] + self.position[p * be + c] + self.token_type[c];
            }
            layer_norm(row, Some((&self.emb_norm.w, &self.emb_norm.b)), 1e-12);
        }
        linear(&self.map, &self.map_bias, &e, t, pool)
    }

    /// One of the ALBERT's layers (the same weights each time), over h (positions × 768).
    fn bert_layer(&self, h: &mut [f32], t: usize, pool: &dyn Pool) {
        let bh = self.settings.bert_hidden;
        let heads = self.settings.bert_heads;
        let hd = bh / heads;
        let q = linear(&self.q, &self.q_bias, h, t, pool);
        let k = linear(&self.k, &self.k_bias, h, t, pool);
        let v = linear(&self.v, &self.v_bias, h, t, pool);
        let mut ctx = vec![0.0f32; t * bh];
        let scale = 1.0 / math::sqrt(hd as f32);
        let ptr = SyncPtr(ctx.as_mut_ptr());
        pool.run(heads * t, &|job| {
            let (head, i) = (job / t, job % t);
            let qi = &q[i * bh + head * hd..i * bh + (head + 1) * hd];
            let mut scores = vec![0.0f32; t];
            let mut max = f32::NEG_INFINITY;
            for (j, sc) in scores.iter_mut().enumerate() {
                *sc = quant::dot(qi, &k[j * bh + head * hd..j * bh + (head + 1) * hd]) * scale;
                max = max.max(*sc);
            }
            let mut sum = 0.0f32;
            for sc in scores.iter_mut() {
                *sc = math::exp(*sc - max);
                sum += *sc;
            }
            let mut out = vec![0.0f32; hd];
            for (j, &p) in scores.iter().enumerate() {
                let vj = &v[j * bh + head * hd..j * bh + (head + 1) * hd];
                for (o, &x) in out.iter_mut().zip(vj) {
                    *o += p * x;
                }
            }
            for (d, o) in out.iter().enumerate() {
                // (Each job its own head and position.)
                unsafe { *ptr.get().add(i * bh + head * hd + d) = o / sum };
            }
        });
        let attn = linear(&self.dense, &self.dense_bias, &ctx, t, pool);
        for (row, a) in h.chunks_exact_mut(bh).zip(attn.chunks_exact(bh)) {
            for (x, y) in row.iter_mut().zip(a) {
                *x += y;
            }
            layer_norm(row, Some((&self.attn_norm.w, &self.attn_norm.b)), 1e-12);
        }
        let mut f = linear(&self.ffn, &self.ffn_bias, h, t, pool);
        for x in f.iter_mut() {
            *x = gelu_tanh(*x);
        }
        let out = linear(&self.ffn_out, &self.ffn_out_bias, &f, t, pool);
        for (row, o) in h.chunks_exact_mut(bh).zip(out.chunks_exact(bh)) {
            for (x, y) in row.iter_mut().zip(o) {
                *x += y;
            }
            layer_norm(row, Some((&self.out_norm.w, &self.out_norm.b)), 1e-12);
        }
    }

    /// A bidirectional LSTM over x (positions × inputs): positions × 2 hidden (forward, then backward).
    fn run_lstm(&self, l: &Lstm, x: &[f32], t: usize, pool: &dyn Pool) -> Vec<f32> {
        let hn = l.hidden;
        let gates = [linear(&l.ih[0], &l.b[0], x, t, pool), linear(&l.ih[1], &l.b[1], x, t, pool)];
        let mut out = vec![0.0f32; t * 2 * hn];
        let ptr = SyncPtr(out.as_mut_ptr());
        pool.run(2, &|dir| {
            let (g_in, hh) = (&gates[dir], &l.hh[dir]);
            let mut hs = vec![0.0f32; hn];
            let mut cs = vec![0.0f32; hn];
            let mut g = vec![0.0f32; 4 * hn];
            for step in 0..t {
                let p = if dir == 0 { step } else { t - 1 - step };
                for (r, gv) in g.iter_mut().enumerate() {
                    *gv = g_in[p * 4 * hn + r] + quant::dot(&hh[r * hn..(r + 1) * hn], &hs);
                }
                for c in 0..hn {
                    let i = math::sigmoid(g[c]);
                    let f = math::sigmoid(g[hn + c]);
                    let gg = tanh(g[2 * hn + c]);
                    let o = math::sigmoid(g[3 * hn + c]);
                    cs[c] = f * cs[c] + i * gg;
                    hs[c] = o * tanh(cs[c]);
                }
                for c in 0..hn {
                    // (Each direction its own half of each row.)
                    unsafe { *ptr.get().add(p * 2 * hn + dir * hn + c) = hs[c] };
                }
            }
        });
        out
    }

    /// The harmonic source from the pitch curve (two values a frame), as its STFT's magnitudes and phases
    /// (22 × its frames), and their number.
    fn source(&self, f0: &[f32], seed: u64) -> (Vec<f32>, usize) {
        let scale = FRAME / 2;
        let rate = RATE as f64;
        let frames2 = f0.len();
        let samples = frames2 * scale;
        let harmonics = 9;
        // The phase at each of the pitch's values: the sum of each harmonic's turns, a value's turns being taken
        // at its middle sample (what F.interpolate's linear downsampling picks there: the value itself).
        let mut phase = vec![0.0f64; frames2 * harmonics];
        let mut sum = [0.0f64; 9];
        for (i, &f) in f0.iter().enumerate() {
            for (hm, acc) in sum.iter_mut().enumerate() {
                // (Python's % 1: what's above the whole number below, for a negative pitch too.)
                let v = f as f64 * (hm + 1) as f64 / rate;
                let rad = v - floor64(v);
                *acc += rad;
                phase[i * harmonics + hm] = *acc * 2.0 * core::f64::consts::PI * scale as f64;
            }
        }
        // Upsampled linearly (align_corners false), the sines, the noise, merged by the linear layer and tanh.
        let noise = gaussians(samples * harmonics, seed);
        let (w, b) = (&self.source.0, self.source.1);
        let mut har = vec![0.0f64; samples];
        for (t, out) in har.iter_mut().enumerate() {
            let src = ((t as f64 + 0.5) / scale as f64 - 0.5).max(0.0);
            let lo = (src as usize).min(frames2 - 1);
            let hi = (lo + 1).min(frames2 - 1);
            let frac = src - lo as f64;
            let fv = f0[t / scale];
            let voiced = fv > 10.0;
            let amp = if voiced { 0.003 } else { 0.1 / 3.0 };
            let mut acc = b as f64;
            for hm in 0..harmonics {
                let p = phase[lo * harmonics + hm] * (1.0 - frac) + phase[hi * harmonics + hm] * frac;
                let sine = if voiced { math::sin_cos(p).0 * 0.1 } else { 0.0 };
                acc += w[hm] as f64 * (sine + amp * noise[t * harmonics + hm] as f64);
            }
            *out = tanh_f64(acc);
        }
        // Its STFT: frames of 20 every 5 (reflected at the ends), Hann-windowed.
        let (nfft, hop) = (self.settings.n_fft, self.settings.hop);
        let half = nfft / 2;
        let bins = half + 1;
        let n = samples / hop + 1;
        let at = |i: isize| -> f64 {
            let last = samples as isize - 1;
            let j = if i < 0 { -i } else if i > last { 2 * last - i } else { i };
            har[j.clamp(0, last) as usize]
        };
        let mut out = vec![0.0f32; 2 * bins * n];
        let mut frame = vec![0.0f64; nfft];
        for f in 0..n {
            for (k, v) in frame.iter_mut().enumerate() {
                *v = at((f * hop + k) as isize - half as isize) * self.window[k];
            }
            for b in 0..bins {
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (k, &v) in frame.iter().enumerate() {
                    re += v * self.cos[b * nfft + k];
                    im -= v * self.sin[b * nfft + k];
                }
                if b == 0 || b == half {
                    im = 0.0;
                }
                out[b * n + f] = sqrt64(re * re + im * im) as f32;
                out[(bins + b) * n + f] = atan2(im, re) as f32;
            }
        }
        (out, n)
    }

    /// The inverse STFT of the generator's output (22 × n: the log-magnitudes, then the phases' arguments).
    fn istft(&self, y: &[f32], n: usize) -> Vec<f32> {
        let (nfft, hop) = (self.settings.n_fft, self.settings.hop);
        let half = nfft / 2;
        let bins = half + 1;
        let total = nfft + hop * (n - 1);
        let mut out = vec![0.0f64; total];
        let mut env = vec![0.0f64; total];
        for f in 0..n {
            let mut spec = [(0.0f64, 0.0f64); 11];
            for (b, sp) in spec.iter_mut().enumerate().take(bins) {
                let mag = math::exp(y[b * n + f]) as f64;
                let ph = sin_f32(y[(bins + b) * n + f]) as f64;
                let (s, c) = math::sin_cos(ph);
                *sp = (mag * c, mag * s);
            }
            for t in 0..nfft {
                // The inverse real DFT (the first and last bins' imaginary parts left out).
                let mut v = spec[0].0 + spec[half].0 * if t % 2 == 0 { 1.0 } else { -1.0 };
                for (b, &(re, im)) in spec.iter().enumerate().take(half).skip(1) {
                    v += 2.0 * (re * self.cos[b * nfft + t] - im * self.sin[b * nfft + t]);
                }
                let w = self.window[t];
                out[f * hop + t] += v / nfft as f64 * w;
                env[f * hop + t] += w * w;
            }
        }
        (half..total - half).map(|i| (out[i] / if env[i] > 1e-11 { env[i] } else { 1.0 }) as f32).collect()
    }
}

/// x (positions × inputs) times a matrix (outputs × inputs) plus its bias: positions × outputs.
fn linear(m: &Matrix, bias: &[f32], x: &[f32], t: usize, pool: &dyn Pool) -> Vec<f32> {
    let inputs = x.len() / t.max(1);
    let mut row = vec![0.0f32; m.cols];
    let xs: Vec<Prepared> = x
        .chunks_exact(inputs)
        .map(|r| {
            row[..inputs].copy_from_slice(r);
            Prepared::new(&row)
        })
        .collect();
    let mut out = m.times_all(&xs, pool);
    for r in out.chunks_exact_mut(m.rows) {
        for (o, b) in r.iter_mut().zip(bias) {
            *o += b;
        }
    }
    out
}

/// A convolution over x (inputs × len, each channel a row): out (outputs × n, n = (len + 2 pad - dil (k - 1) - 1) /
/// stride + 1), its positions shared among the pool's processors.
fn conv(c: &Conv, x: &[f32], len: usize, stride: usize, pad: usize, dil: usize, pool: &dyn Pool) -> (Vec<f32>, usize) {
    let outputs = c.m.rows;
    let span = dil * (c.k - 1) + 1;
    let n = if len + 2 * pad >= span { (len + 2 * pad - span) / stride + 1 } else { 0 };
    let mut out = vec![0.0f32; outputs * n];
    if n == 0 {
        return (out, 0);
    }
    const PER: usize = 16;
    let jobs = n.div_ceil(PER);
    let ptr = SyncPtr(out.as_mut_ptr());
    let cols = c.m.cols;
    pool.run(jobs, &|j| {
        let (t0, t1) = (j * PER, ((j + 1) * PER).min(n));
        let mut col = vec![0.0f32; cols];
        let mut prepared = Vec::with_capacity(t1 - t0);
        for t in t0..t1 {
            for i in 0..c.inputs {
                let row = &x[i * len..(i + 1) * len];
                for k in 0..c.k {
                    let p = (t * stride + k * dil) as isize - pad as isize;
                    col[i * c.k + k] = if p >= 0 && (p as usize) < len { row[p as usize] } else { 0.0 };
                }
            }
            prepared.push(Prepared::new(&col));
        }
        let refs: Vec<&Prepared> = prepared.iter().collect();
        let mut vals = vec![0.0f32; t1 - t0];
        for o in 0..outputs {
            quant::dot_row_many(c.m.dtype, c.m.data, cols, o, &refs, &mut vals);
            let b = c.bias.get(o).copied().unwrap_or(0.0);
            for (dt, v) in vals.iter().enumerate() {
                // (Each job its own positions.)
                unsafe { *ptr.get().add(o * n + t0 + dt) = v + b };
            }
        }
    });
    (out, n)
}

/// A transposed convolution (upsampling by its stride, padding (k - stride) / 2): each phase of the output a
/// convolution of its own over the input's positions.
fn conv_transpose(c: &ConvT, x: &[f32], len: usize, pool: &dyn Pool) -> (Vec<f32>, usize) {
    let (s, k) = (c.stride, c.k);
    let pad = (k - s) / 2;
    let taps = k / s;
    let n = (len - 1) * s + k - 2 * pad;
    let mut out = vec![0.0f32; c.outputs * n];
    // Output t = m s + φ - pad, for phase φ: the taps φ + j s, reading the input at m - j.
    const PER: usize = 16;
    let per_phase = (n + pad).div_ceil(s) + 1;
    let chunks = per_phase.div_ceil(PER);
    let ptr = SyncPtr(out.as_mut_ptr());
    let cols = c.m.cols;
    pool.run(s * chunks, &|job| {
        let (phase, chunk) = (job / chunks, job % chunks);
        let mut ms = Vec::new();
        for m in chunk * PER..((chunk + 1) * PER).min(per_phase) {
            let t = (m * s + phase) as isize - pad as isize;
            if t >= 0 && (t as usize) < n {
                ms.push(m);
            }
        }
        if ms.is_empty() {
            return;
        }
        let mut col = vec![0.0f32; cols];
        let prepared: Vec<Prepared> = ms
            .iter()
            .map(|&m| {
                for i in 0..c.inputs {
                    for j in 0..taps {
                        let l = m as isize - j as isize;
                        col[i * taps + j] = if l >= 0 && (l as usize) < len { x[i * len + l as usize] } else { 0.0 };
                    }
                }
                Prepared::new(&col)
            })
            .collect();
        let refs: Vec<&Prepared> = prepared.iter().collect();
        let mut vals = vec![0.0f32; ms.len()];
        for o in 0..c.outputs {
            quant::dot_row_many(c.m.dtype, c.m.data, cols, phase * c.outputs + o, &refs, &mut vals);
            for (&m, v) in ms.iter().zip(&vals) {
                let t = m * s + phase - pad;
                // (Each job its own phase's positions.)
                unsafe { *ptr.get().add(o * n + t) = v + c.bias[o] };
            }
        }
    });
    (out, n)
}

/// The upsampling of an AdainResBlk1d: a depthwise transposed convolution (kernel 3, stride 2, padding 1, output
/// padding 1), each channel its own three weights.
fn depthwise_up(x: &[f32], ch: usize, len: usize, w: &[f32], bias: &[f32]) -> Vec<f32> {
    let n = 2 * len;
    let mut out = vec![0.0f32; ch * n];
    for c in 0..ch {
        let o = &mut out[c * n..(c + 1) * n];
        o.fill(bias[c]);
        for l in 0..len {
            let v = x[c * len + l];
            for k in 0..3 {
                let t = 2 * l as isize - 1 + k as isize;
                if t >= 0 && (t as usize) < n {
                    o[t as usize] += v * w[c * 3 + k];
                }
            }
        }
    }
    out
}

/// The decoder's convolution of a curve: kernel 3, stride 2, padding 1, one channel.
fn small_conv((w, b): &([f32; 3], f32), x: &[f32]) -> Vec<f32> {
    let len = x.len();
    let n = (len + 2 - 3) / 2 + 1;
    (0..n)
        .map(|t| {
            let mut v = *b;
            for (k, wk) in w.iter().enumerate() {
                let p = (2 * t + k) as isize - 1;
                if p >= 0 && (p as usize) < len {
                    v += wk * x[p as usize];
                }
            }
            v
        })
        .collect()
}

/// The scale and shift a style gives each channel (fc(style), split in two).
fn style_scale(fc: &StyleFc, style: &[f32], channels: usize) -> (Vec<f32>, Vec<f32>) {
    let n = style.len();
    let h: Vec<f32> = (0..2 * channels).map(|r| quant::dot(&fc.w[r * n..(r + 1) * n], style) + fc.b[r]).collect();
    (h[..channels].to_vec(), h[channels..].to_vec())
}

/// Instance norm of each channel (over its positions), scaled and shifted by the style, then LeakyReLU.
fn instance_norm_styled(x: &mut [f32], len: usize, g: &[f32], b: &[f32], slope: f32, pool: &dyn Pool) {
    let ptr = SyncPtr(x.as_mut_ptr());
    pool.run(g.len(), &|c| {
        // (Each job its own channel.)
        let row = unsafe { core::slice::from_raw_parts_mut(ptr.get().add(c * len), len) };
        let (mean, inv) = moments(row);
        let (scale, shift) = (1.0 + g[c], b[c]);
        for v in row.iter_mut() {
            *v = leaky(scale * ((*v - mean) * inv) + shift, slope);
        }
    });
}

/// AdaIN then Snake (x + sin²(a x) / a) of each channel.
fn adain_snake(x: &mut [f32], len: usize, fc: &StyleFc, alpha: &[f32], style: &[f32], pool: &dyn Pool) {
    let (g, b) = style_scale(fc, style, alpha.len());
    let ptr = SyncPtr(x.as_mut_ptr());
    pool.run(alpha.len(), &|c| {
        // (Each job its own channel.)
        let row = unsafe { core::slice::from_raw_parts_mut(ptr.get().add(c * len), len) };
        let (mean, inv) = moments(row);
        let (scale, shift, a) = (1.0 + g[c], b[c], alpha[c]);
        let inv_a = 1.0 / a;
        for v in row.iter_mut() {
            let y = scale * ((*v - mean) * inv) + shift;
            let sn = sin_f32(a * y);
            *v = y + inv_a * sn * sn;
        }
    });
}

/// A row's mean and 1 / √(variance + 1e-5) (the variance over the row, as instance norm takes it).
fn moments(row: &[f32]) -> (f32, f32) {
    let n = row.len().max(1) as f64;
    let mean = row.iter().map(|&v| v as f64).sum::<f64>() / n;
    let var = row.iter().map(|&v| (v as f64 - mean) * (v as f64 - mean)).sum::<f64>() / n;
    (mean as f32, (1.0 / sqrt64(var + 1e-5)) as f32)
}

fn layer_norm(row: &mut [f32], wb: Option<(&[f32], &[f32])>, eps: f64) {
    let n = row.len() as f64;
    let mean = row.iter().map(|&v| v as f64).sum::<f64>() / n;
    let var = row.iter().map(|&v| (v as f64 - mean) * (v as f64 - mean)).sum::<f64>() / n;
    let inv = 1.0 / sqrt64(var + eps);
    for (i, v) in row.iter_mut().enumerate() {
        let y = ((*v as f64 - mean) * inv) as f32;
        *v = match wb {
            Some((w, b)) => y * w[i] + b[i],
            None => y,
        };
    }
}

/// x (positions × width) with the style after each position's numbers: positions × (width + style).
fn with_style(x: &[f32], t: usize, width: usize, style: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(t * (width + style.len()));
    for row in x.chunks_exact(width).take(t) {
        out.extend_from_slice(row);
        out.extend_from_slice(style);
    }
    out
}

/// rows × cols into cols × rows.
fn transpose(x: &[f32], rows: usize, cols: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; rows * cols];
    for r in 0..rows {
        for c in 0..cols {
            out[c * rows + r] = x[r * cols + c];
        }
    }
    out
}

fn leaky(x: f32, slope: f32) -> f32 {
    if x >= 0.0 { x } else { x * slope }
}

fn tanh(x: f32) -> f32 {
    let a = x.abs();
    if a > 9.0 {
        return x.signum();
    }
    let e = math::exp(-2.0 * a);
    let t = (1.0 - e) / (1.0 + e);
    if x < 0.0 { -t } else { t }
}

fn tanh_f64(x: f64) -> f64 {
    let a = x.abs();
    if a > 20.0 {
        return x.signum();
    }
    let e = math::exp_f64(-2.0 * a);
    let t = (1.0 - e) / (1.0 + e);
    if x < 0.0 { -t } else { t }
}

/// GELU as ALBERT's "gelu_new" (the tanh approximation).
fn gelu_tanh(x: f32) -> f32 {
    0.5 * x * (1.0 + tanh(0.797_884_6 * (x + 0.044715 * x * x * x)))
}

fn sqrt64(x: f64) -> f64 {
    math::sqrt64(x)
}

/// Rounded to the nearest whole number, halves to the even one (as torch.round).
fn round_even(x: f32) -> f32 {
    let fl = floor(x);
    let diff = x - fl;
    if diff > 0.5 || (diff == 0.5 && (fl as i64) % 2 != 0) { fl + 1.0 } else { fl }
}

fn floor64(x: f64) -> f64 {
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

fn floor(x: f32) -> f32 {
    let t = x as i64 as f32;
    if t > x { t - 1.0 } else { t }
}

/// sin x, in single precision (Snake's, and the phases'): x less π times the nearest whole number of π, then its
/// series.
pub fn sin_f32(x: f32) -> f32 {
    if !x.is_finite() {
        return f32::NAN;
    }
    let xd = x as f64;
    let k = (xd * core::f64::consts::FRAC_1_PI + if xd < 0.0 { -0.5 } else { 0.5 }) as i64;
    let r = (xd - k as f64 * core::f64::consts::PI) as f32;
    let r2 = r * r;
    let s = r * (1.0 + r2 * (-1.0 / 6.0 + r2 * (1.0 / 120.0 + r2 * (-1.0 / 5040.0 + r2 * (1.0 / 362_880.0 + r2 * (-1.0 / 39_916_800.0))))));
    if k % 2 == 0 { s } else { -s }
}

/// atan(y / x) in its quadrant (y = 0 with x < 0: π, as the reference has it).
fn atan2(y: f64, x: f64) -> f64 {
    use core::f64::consts::{FRAC_PI_2, PI};
    if x == 0.0 {
        return if y > 0.0 { FRAC_PI_2 } else if y < 0.0 { -FRAC_PI_2 } else { 0.0 };
    }
    let a = atan(y / x);
    if x > 0.0 {
        a
    } else if y < 0.0 {
        a - PI
    } else {
        a + PI
    }
}

fn atan(x: f64) -> f64 {
    use core::f64::consts::{FRAC_PI_2, FRAC_PI_6};
    let (neg, x) = if x < 0.0 { (true, -x) } else { (false, x) };
    let (inverted, x) = if x > 1.0 { (true, 1.0 / x) } else { (false, x) };
    // Within tan(π/12) of 0, by atan(x) = π/6 + atan((x√3 - 1) / (√3 + x)).
    const SQRT3: f64 = 1.732_050_807_568_877_2;
    let (shifted, u) = if x > 0.267_949_192_431_122_7 { (true, (x * SQRT3 - 1.0) / (SQRT3 + x)) } else { (false, x) };
    let u2 = u * u;
    let mut series = 0.0;
    for n in (0..12).rev() {
        series = series * u2 + if n % 2 == 0 { 1.0 } else { -1.0 } / (2 * n + 1) as f64;
    }
    let mut a = u * series;
    if shifted {
        a += FRAC_PI_6;
    }
    if inverted {
        a = FRAC_PI_2 - a;
    }
    if neg { -a } else { a }
}

/// `n` Gaussian numbers from `seed`: splitmix64's numbers, two at a time made two by Box-Muller (as the reference
/// makes them).
pub fn gaussians(n: usize, seed: u64) -> Vec<f32> {
    let mut out = Vec::with_capacity(n + 1);
    let mut counter = 0u64;
    let mut next = || {
        counter = counter.wrapping_add(1);
        let mut z = seed.wrapping_add(counter.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        ((z >> 11) as f64 + 1.0) * (1.0 / (1u64 << 53) as f64)
    };
    while out.len() < n {
        let (u1, u2) = (next(), next());
        let r = sqrt64(-2.0 * math::ln_f64(u1));
        let (s, c) = math::sin_cos(2.0 * core::f64::consts::PI * u2);
        out.push((r * c) as f32);
        out.push((r * s) as f32);
    }
    out.truncate(n);
    out
}

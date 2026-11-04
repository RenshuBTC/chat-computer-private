//! Speech recognition: OpenAI's Whisper (published under the MIT licence), run by the engine, as Chat Computer's app
//! runs it with faster-whisper:
//!
//! - the sound (16,000 samples a second) as a log-mel spectrogram: a hop of silence after it, frames of 400 samples
//!   every 160 (each windowed by a Hann window, its power at 201 frequencies by a Fourier transform), through the mel
//!   filters (Slaney's, as librosa makes them), log10, floored 8 below the loudest, scaled; padded to 3,000 frames
//!   (30 seconds) with zeros;
//! - the encoder: two convolutions (the second halving the frames to 1,500), each with GELU, learned positions, then
//!   blocks of attention over every position and a feed-forward part, each after a layer norm and added to what went
//!   in, and a last layer norm;
//! - the decoder, a token at a time: its tokens' embeddings and learned positions, then blocks of attention over the
//!   tokens so far, attention over the encoder's positions and a feed-forward part; the scores are the embeddings
//!   times what comes out (the weights tied);
//! - greedy decoding with faster-whisper's rules: the prompt (the start of a transcript, the language and the task
//!   for a model of many languages, no timestamps), its list of tokens never chosen (speaker tags, music, the special
//!   tokens), a blank or the end never first, and the chance it heard no speech read at the start.
//!
//! A step at a time for a caller with other things to do (the own system's kernel, its screen and keyboard going on
//! meanwhile): `begin`, then `advance` until it gives the transcript, each step a part of the work for as many
//! positions as the caller says. The transcript is the same however many a step takes.

use crate::file::{Error, File};
use crate::math;
use crate::net::{Matrix, Pool};
use crate::quant::{self, Prepared};
use crate::token::Tokenizer;
use crate::vector;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Samples a second the models hear.
pub const RATE: usize = 16_000;
/// A frame's samples, and how far apart frames start.
pub const N_FFT: usize = 400;
pub const HOP: usize = 160;
/// The frames the encoder reads: 30 seconds.
pub const FRAMES: usize = 3000;
const BINS: usize = N_FFT / 2 + 1;
/// The most tokens a transcript has: half the decoder's positions, as OpenAI's own decoding allows.
const MOST_TOKENS: usize = 224;
const EPS: f32 = 1e-5;

#[derive(Clone, Debug)]
pub struct Settings {
    pub mels: usize,
    pub width: usize,
    pub encoder_layers: usize,
    pub decoder_layers: usize,
    pub encoder_heads: usize,
    pub decoder_heads: usize,
    pub encoder_ffn: usize,
    pub decoder_ffn: usize,
    pub vocab: usize,
    /// The encoder's positions (1,500) and the decoder's (448).
    pub audio_ctx: usize,
    pub text_ctx: usize,
    /// The special tokens: the start of a transcript, its end, no timestamps, "no speech", the first timestamp, and
    /// a blank (a space alone).
    pub sot: u32,
    pub eot: u32,
    pub no_timestamps: u32,
    pub no_speech: u32,
    pub timestamp_begin: u32,
    pub blank: u32,
    /// A model of many languages: its language tokens (first and last) and the task's (to transcribe).
    pub languages: Option<(u32, u32)>,
    pub transcribe: Option<u32>,
    /// Tokens never chosen, and never chosen first.
    pub suppress: Vec<u32>,
    pub begin_suppress: Vec<u32>,
}

struct Norm {
    w: Vec<f32>,
    b: Vec<f32>,
}

struct Attention<'a> {
    q: Matrix<'a>,
    q_bias: Vec<f32>,
    k: Matrix<'a>,
    v: Matrix<'a>,
    v_bias: Vec<f32>,
    o: Matrix<'a>,
    o_bias: Vec<f32>,
}

struct Mlp<'a> {
    fc1: Matrix<'a>,
    fc1_bias: Vec<f32>,
    fc2: Matrix<'a>,
    fc2_bias: Vec<f32>,
}

struct EncoderLayer<'a> {
    norm1: Norm,
    attention: Attention<'a>,
    norm2: Norm,
    mlp: Mlp<'a>,
}

struct DecoderLayer<'a> {
    norm1: Norm,
    attention: Attention<'a>,
    norm2: Norm,
    cross: Attention<'a>,
    norm3: Norm,
    mlp: Mlp<'a>,
}

/// A speech recognition model, over its file's bytes.
pub struct Whisper<'a> {
    pub settings: Settings,
    pub tokenizer: Tokenizer,
    /// The mel filters (mels by 201 frequencies), the window, and e^(-2πik/400).
    filters: Vec<f32>,
    window: Vec<f64>,
    twiddles: Vec<(f64, f64)>,
    /// The first convolution in full precision (it reads the spectrogram itself): width by 3 x mels.
    conv1: Vec<f32>,
    conv1_bias: Vec<f32>,
    conv2: Matrix<'a>,
    conv2_bias: Vec<f32>,
    encoder_pos: Vec<f32>,
    encoder: Vec<EncoderLayer<'a>>,
    encoder_norm: Norm,
    embed: Matrix<'a>,
    decoder_pos: Vec<f32>,
    decoder: Vec<DecoderLayer<'a>>,
    decoder_norm: Norm,
}

/// What it heard: the words, their tokens, the chance it was no speech at all, and the tokens' mean log-likelihood
/// (as faster-whisper reckons it: over the tokens and the end).
#[derive(Clone, Debug, PartialEq)]
pub struct Transcript {
    pub text: String,
    pub tokens: Vec<u32>,
    pub no_speech: f32,
    pub avg_logprob: f32,
    /// The language it was heard in (a model of many languages; English for one of English alone).
    pub language: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// The first convolution, over the spectrogram's frames; the second, over the encoder's positions.
    Conv1,
    Conv2,
    /// In an encoder layer: each position's query, key and value; each position's attention over all; the rest.
    Qkv,
    Attend,
    Out,
    /// Each decoder layer's keys and values for the encoder's positions.
    Cross,
    /// The prompt read (and, for a model of many languages told none, the language found first).
    Prompt,
    /// A token at a time.
    Decode,
}

/// A recording being transcribed (`Whisper::begin`, then `advance` until it gives the transcript).
pub struct Hearing {
    features: Vec<f32>,
    /// The first convolution's output (3,000 frames), then the encoder's state (1,500 positions).
    conv: Vec<f32>,
    x: Vec<f32>,
    q: Vec<f32>,
    k: Vec<f32>,
    v: Vec<f32>,
    /// The keys laid out by head and dimension (`by_head`).
    kt: Vec<f32>,
    att: Vec<f32>,
    phase: Phase,
    layer: usize,
    at: usize,
    /// Each decoder layer's keys and values: for the encoder's positions, and for the tokens so far.
    cross: Vec<(Vec<f32>, Vec<f32>)>,
    past: Vec<(Vec<f32>, Vec<f32>)>,
    language: Option<String>,
    prompt: Vec<u32>,
    chosen: Vec<u32>,
    /// The scores for the next token.
    next: Vec<f32>,
    no_speech: f32,
    logprob: f64,
    done: usize,
}

impl<'a> Whisper<'a> {
    /// The model in a file (Error::Config: the file isn't a speech recognition model).
    pub fn load(file: &File<'a>) -> Result<Whisper<'a>, Error> {
        let c = file.config()?;
        if c.str("arch") != Some("whisper") {
            return Err(Error::Config);
        }
        let n = |k: &str| c.usize(&alloc::format!("whisper.{k}"));
        let id = |k: &str| n(k).map(|v| v as u32);
        let list = |k: &str| -> Vec<u32> {
            c.str(&alloc::format!("whisper.{k}")).map_or(Vec::new(), |s| s.split(',').filter_map(|v| v.trim().parse().ok()).collect())
        };
        let languages = match (id("language_first"), id("language_last")) {
            (Ok(a), Ok(b)) if a <= b => Some((a, b)),
            _ => None,
        };
        let s = Settings {
            mels: n("mels")?,
            width: n("width")?,
            encoder_layers: n("encoder_layers")?,
            decoder_layers: n("decoder_layers")?,
            encoder_heads: n("encoder_heads")?,
            decoder_heads: n("decoder_heads")?,
            encoder_ffn: n("encoder_ffn")?,
            decoder_ffn: n("decoder_ffn")?,
            vocab: n("vocab")?,
            audio_ctx: n("audio_ctx")?,
            text_ctx: n("text_ctx")?,
            sot: id("sot")?,
            eot: id("eot")?,
            no_timestamps: id("no_timestamps")?,
            no_speech: id("no_speech")?,
            timestamp_begin: id("timestamp_begin")?,
            blank: id("blank")?,
            languages,
            transcribe: id("transcribe").ok(),
            suppress: list("suppress"),
            begin_suppress: list("begin_suppress"),
        };
        let w = s.width;
        if s.encoder_heads == 0 || s.decoder_heads == 0 || !w.is_multiple_of(s.encoder_heads) || !w.is_multiple_of(s.decoder_heads)
            || s.audio_ctx * 2 != FRAMES || s.mels == 0 || s.text_ctx < 8
            || [s.sot, s.eot, s.no_timestamps, s.no_speech, s.timestamp_begin, s.blank].iter().any(|&t| t as usize >= s.vocab)
            || (languages.is_some() && s.transcribe.is_none())
        {
            return Err(Error::Config);
        }
        let vector = |name: &str, len: usize| -> Result<Vec<f32>, Error> {
            let v = crate::net::vector(file, name)?;
            if v.len() == len { Ok(v) } else { Err(Error::Damaged) }
        };
        let norm = |name: &str| -> Result<Norm, Error> {
            Ok(Norm { w: vector(name, w)?, b: vector(&alloc::format!("{name}_bias"), w)? })
        };
        let attention = |p: &str| -> Result<Attention<'a>, Error> {
            Ok(Attention {
                q: crate::net::matrix(file, &alloc::format!("{p}.q"), w, w)?,
                q_bias: vector(&alloc::format!("{p}.q_bias"), w)?,
                k: crate::net::matrix(file, &alloc::format!("{p}.k"), w, w)?,
                v: crate::net::matrix(file, &alloc::format!("{p}.v"), w, w)?,
                v_bias: vector(&alloc::format!("{p}.v_bias"), w)?,
                o: crate::net::matrix(file, &alloc::format!("{p}.o"), w, w)?,
                o_bias: vector(&alloc::format!("{p}.o_bias"), w)?,
            })
        };
        let mlp = |p: &str, ffn: usize| -> Result<Mlp<'a>, Error> {
            Ok(Mlp {
                fc1: crate::net::matrix(file, &alloc::format!("{p}.fc1"), ffn, w)?,
                fc1_bias: vector(&alloc::format!("{p}.fc1_bias"), ffn)?,
                fc2: crate::net::matrix(file, &alloc::format!("{p}.fc2"), w, ffn)?,
                fc2_bias: vector(&alloc::format!("{p}.fc2_bias"), w)?,
            })
        };
        let mut encoder = Vec::with_capacity(s.encoder_layers);
        for i in 0..s.encoder_layers {
            let p = alloc::format!("whisper.encoder.{i}");
            encoder.push(EncoderLayer {
                norm1: norm(&alloc::format!("{p}.norm1"))?,
                attention: attention(&alloc::format!("{p}.attention"))?,
                norm2: norm(&alloc::format!("{p}.norm2"))?,
                mlp: mlp(&p, s.encoder_ffn)?,
            });
        }
        let mut decoder = Vec::with_capacity(s.decoder_layers);
        for i in 0..s.decoder_layers {
            let p = alloc::format!("whisper.decoder.{i}");
            decoder.push(DecoderLayer {
                norm1: norm(&alloc::format!("{p}.norm1"))?,
                attention: attention(&alloc::format!("{p}.attention"))?,
                norm2: norm(&alloc::format!("{p}.norm2"))?,
                cross: attention(&alloc::format!("{p}.cross"))?,
                norm3: norm(&alloc::format!("{p}.norm3"))?,
                mlp: mlp(&p, s.decoder_ffn)?,
            });
        }
        let tokenizer = crate::load_tokenizer(file)?;
        if tokenizer.len() < s.vocab.min(s.timestamp_begin as usize + 1) {
            return Err(Error::Damaged);
        }
        let twiddles = (0..N_FFT)
            .map(|k| {
                let (sn, cn) = math::sin_cos(-2.0 * core::f64::consts::PI * k as f64 / N_FFT as f64);
                (cn, sn)
            })
            .collect();
        let window = (0..N_FFT).map(|k| 0.5 - 0.5 * math::sin_cos(2.0 * core::f64::consts::PI * k as f64 / N_FFT as f64).1).collect();
        Ok(Whisper {
            filters: mel_filters(s.mels),
            window,
            twiddles,
            conv1: vector("whisper.conv1", w * 3 * s.mels)?,
            conv1_bias: vector("whisper.conv1_bias", w)?,
            conv2: crate::net::matrix(file, "whisper.conv2", w, 3 * w)?,
            conv2_bias: vector("whisper.conv2_bias", w)?,
            encoder_pos: vector("whisper.encoder_pos", s.audio_ctx * w)?,
            encoder,
            encoder_norm: norm("whisper.encoder_norm")?,
            embed: crate::net::matrix(file, "whisper.embed", s.vocab, w)?,
            decoder_pos: vector("whisper.decoder_pos", s.text_ctx * w)?,
            decoder,
            decoder_norm: norm("whisper.decoder_norm")?,
            tokenizer,
            settings: s,
        })
    }

    /// Whether the model knows many languages (else English alone).
    pub fn multilingual(&self) -> bool {
        self.settings.languages.is_some()
    }

    /// The sound's features as the encoder reads them: `mels` rows of 3,000 frames (30 seconds; a longer sound's
    /// first 30). `samples`: 16,000 a second, from -1 to 1.
    pub fn features(&self, samples: &[f32]) -> Vec<f32> {
        let mels = self.settings.mels;
        // A hop of silence after it, as faster-whisper adds; frames centred on every 160th sample (the sound
        // reflected at its ends), the last left out, as faster-whisper does.
        let mut padded: Vec<f64> = samples.iter().map(|&s| s as f64).collect();
        padded.resize(samples.len() + HOP, 0.0);
        let n = padded.len();
        let half = N_FFT / 2;
        let at = |i: isize| -> f64 {
            // (Reflected: ... x2 x1 | x0 x1 ... xn-1 | xn-2 xn-3 ...)
            let mut i = i;
            let last = n as isize - 1;
            while i < 0 || i > last {
                if i < 0 {
                    i = -i;
                }
                if i > last {
                    i = 2 * last - i;
                }
                if last == 0 {
                    return padded[0];
                }
            }
            padded[i as usize]
        };
        let frames = (n / HOP).min(FRAMES);
        let mut power = vec![0.0f32; frames * BINS];
        let mut frame = vec![(0.0f64, 0.0f64); N_FFT];
        let mut out = vec![(0.0f64, 0.0f64); N_FFT];
        for f in 0..frames {
            for (k, slot) in frame.iter_mut().enumerate() {
                *slot = (at((f * HOP + k) as isize - half as isize) * self.window[k], 0.0);
            }
            fft(&frame, 0, 1, N_FFT, &mut out, &self.twiddles);
            for (b, p) in power[f * BINS..(f + 1) * BINS].iter_mut().enumerate() {
                let (re, im) = out[b];
                *p = (re * re + im * im) as f32;
            }
        }
        // Through the mel filters, as log10, floored 8 below the loudest, scaled.
        let mut features = vec![0.0f32; mels * FRAMES];
        let mut loudest = f32::NEG_INFINITY;
        for m in 0..mels {
            let filter = &self.filters[m * BINS..(m + 1) * BINS];
            for f in 0..frames {
                let e = quant::dot(filter, &power[f * BINS..(f + 1) * BINS]).max(1e-10);
                let v = (math::ln_f64(e as f64) / core::f64::consts::LN_10) as f32;
                features[m * FRAMES + f] = v;
                loudest = loudest.max(v);
            }
        }
        for m in 0..mels {
            for v in &mut features[m * FRAMES..m * FRAMES + frames] {
                *v = (v.max(loudest - 8.0) + 4.0) / 4.0;
            }
        }
        features
    }

    /// The whole of it at once: the transcript of `samples` (16,000 a second, up to 30 seconds), in `language` (a
    /// code such as "en"; None: the model's own guess, or English for a model of English alone).
    pub fn transcribe(&self, samples: &[f32], language: Option<&str>, pool: &dyn Pool) -> Transcript {
        let mut h = self.begin(samples, language);
        loop {
            if let Some(t) = self.advance(&mut h, usize::MAX, pool) {
                return t;
            }
        }
    }

    /// The encoder's output for a recording (1,500 positions, each as wide as the model), for checking.
    pub fn encoded(&self, samples: &[f32], pool: &dyn Pool) -> Vec<f32> {
        let mut h = self.begin(samples, Some("en"));
        while h.phase != Phase::Cross {
            self.advance(&mut h, usize::MAX, pool);
        }
        h.x
    }

    /// A recording to transcribe, a step at a time (`advance`).
    pub fn begin(&self, samples: &[f32], language: Option<&str>) -> Hearing {
        let s = &self.settings;
        let language = match (self.multilingual(), language) {
            (false, _) => Some(String::from("en")),
            (true, Some(code)) if self.language_token(code).is_some() => Some(String::from(code)),
            (true, _) => None,
        };
        Hearing {
            features: self.features(samples),
            conv: Vec::new(),
            x: Vec::new(),
            q: Vec::new(),
            k: Vec::new(),
            v: Vec::new(),
            kt: Vec::new(),
            att: Vec::new(),
            phase: Phase::Conv1,
            layer: 0,
            at: 0,
            cross: Vec::new(),
            past: (0..s.decoder_layers).map(|_| (Vec::new(), Vec::new())).collect(),
            language,
            prompt: Vec::new(),
            chosen: Vec::new(),
            next: Vec::new(),
            no_speech: 0.0,
            logprob: 0.0,
            done: 0,
        }
    }

    /// How far a transcription has got, of how far the encoder goes (a frame, or a layer's part for a position, one
    /// each; the tokens after aren't counted).
    pub fn progress(&self, h: &Hearing) -> (usize, usize) {
        let s = &self.settings;
        let total = FRAMES + s.audio_ctx * (1 + 3 * s.encoder_layers + s.decoder_layers);
        (h.done.min(total), total)
    }

    /// One more step: the part it's at, for up to `most` more positions (at least one), or a token. The transcript,
    /// once it's done.
    pub fn advance(&self, h: &mut Hearing, most: usize, pool: &dyn Pool) -> Option<Transcript> {
        let s = &self.settings;
        let w = s.width;
        let n = s.audio_ctx;
        let most = most.max(1);
        let from = h.at;
        match h.phase {
            Phase::Conv1 => {
                // Each frame's three neighbours (every mel), through the first convolution, GELU.
                let end = from.saturating_add(most).min(FRAMES);
                if h.conv.len() != FRAMES * w {
                    h.conv = vec![0.0f32; FRAMES * w];
                }
                let mels = s.mels;
                let features = &h.features;
                let conv1 = &self.conv1;
                let bias = &self.conv1_bias;
                let out = SyncPtr(h.conv.as_mut_ptr());
                pool.run(end - from, &|j| {
                    let t = from + j;
                    let mut col = vec![0.0f32; 3 * mels];
                    for c in 0..mels {
                        for k in 0..3 {
                            let f = t as isize + k as isize - 1;
                            if (0..FRAMES as isize).contains(&f) {
                                col[c * 3 + k] = features[c * FRAMES + f as usize];
                            }
                        }
                    }
                    // (Each job writes its own frame's row.)
                    let row = unsafe { core::slice::from_raw_parts_mut(out.get().add(t * w), w) };
                    for (o, r) in row.iter_mut().enumerate() {
                        *r = bias[o] + quant::dot(&conv1[o * 3 * mels..(o + 1) * 3 * mels], &col);
                    }
                    vector::gelu(row);
                });
                h.done += end - from;
                h.at = end;
                if end == FRAMES {
                    h.features = Vec::new();
                    (h.phase, h.at) = (Phase::Conv2, 0);
                }
            }
            Phase::Conv2 => {
                // Every other frame's three neighbours (every channel), through the second convolution, GELU, plus
                // the position.
                let end = from.saturating_add(most).min(n);
                if h.x.len() != n * w {
                    h.x = vec![0.0f32; n * w];
                }
                let ready: Vec<Prepared> = (from..end)
                    .map(|t| {
                        let mut col = vec![0.0f32; 3 * w];
                        for k in 0..3 {
                            let f = 2 * t as isize + k as isize - 1;
                            if (0..FRAMES as isize).contains(&f) {
                                let row = &h.conv[f as usize * w..(f as usize + 1) * w];
                                for (c, &v) in row.iter().enumerate() {
                                    col[c * 3 + k] = v;
                                }
                            }
                        }
                        Prepared::new(&col)
                    })
                    .collect();
                let x = &mut h.x[from * w..end * w];
                self.conv2.times_into(&ready, x, pool);
                add_bias(x, &self.conv2_bias);
                vector::gelu(x);
                for (v, p) in x.iter_mut().zip(&self.encoder_pos[from * w..end * w]) {
                    *v += p;
                }
                h.done += end - from;
                h.at = end;
                if end == n {
                    h.conv = Vec::new();
                    (h.phase, h.at, h.layer) = (Phase::Qkv, 0, 0);
                }
            }
            Phase::Qkv => {
                let l = &self.encoder[h.layer];
                let end = from.saturating_add(most).min(n);
                if h.q.len() != n * w || h.k.len() != n * w || h.v.len() != n * w {
                    h.q = vec![0.0f32; n * w];
                    h.k = vec![0.0f32; n * w];
                    h.v = vec![0.0f32; n * w];
                }
                let normed = layer_norm(&h.x[from * w..end * w], &l.norm1, w);
                let ready: Vec<Prepared> = normed.chunks_exact(w).map(Prepared::new).collect();
                let a = &l.attention;
                a.q.times_into(&ready, &mut h.q[from * w..end * w], pool);
                a.k.times_into(&ready, &mut h.k[from * w..end * w], pool);
                a.v.times_into(&ready, &mut h.v[from * w..end * w], pool);
                add_bias(&mut h.q[from * w..end * w], &a.q_bias);
                add_bias(&mut h.v[from * w..end * w], &a.v_bias);
                h.done += end - from;
                h.at = end;
                if end == n {
                    (h.phase, h.at) = (Phase::Attend, 0);
                }
            }
            Phase::Attend => {
                let end = from.saturating_add(most).min(n);
                if h.att.len() != n * w {
                    h.att = vec![0.0f32; n * w];
                }
                if from == 0 {
                    // (The keys laid out by head and dimension, for the scores to be worked out eight keys at a time.)
                    h.kt = by_head(&h.k, n, s.encoder_heads, w);
                }
                attend(&h.q[from * w..end * w], &h.kt, &h.v, n, s.encoder_heads, w, &mut h.att[from * w..end * w], pool);
                h.done += end - from;
                h.at = end;
                if end == n {
                    (h.phase, h.at) = (Phase::Out, 0);
                }
            }
            Phase::Out => {
                let l = &self.encoder[h.layer];
                let end = from.saturating_add(most).min(n);
                let x = &mut h.x[from * w..end * w];
                project_add(&l.attention.o, &l.attention.o_bias, &h.att[from * w..end * w], w, x, pool);
                feed_forward(&l.mlp, &l.norm2, w, x, pool);
                h.done += end - from;
                h.at = end;
                if end == n {
                    h.layer += 1;
                    h.at = 0;
                    if h.layer < self.encoder.len() {
                        h.phase = Phase::Qkv;
                    } else {
                        // The encoder's output: its last layer norm, for every decoder layer to read.
                        h.x = layer_norm(&h.x, &self.encoder_norm, w);
                        (h.q, h.k, h.v, h.att, h.kt) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
                        h.cross = (0..s.decoder_layers).map(|_| (vec![0.0f32; n * w], vec![0.0f32; n * w])).collect();
                        (h.phase, h.layer) = (Phase::Cross, 0);
                    }
                }
            }
            Phase::Cross => {
                let c = &self.decoder[h.layer].cross;
                let end = from.saturating_add(most).min(n);
                let ready: Vec<Prepared> = h.x[from * w..end * w].chunks_exact(w).map(Prepared::new).collect();
                let (k, v) = &mut h.cross[h.layer];
                c.k.times_into(&ready, &mut k[from * w..end * w], pool);
                c.v.times_into(&ready, &mut v[from * w..end * w], pool);
                add_bias(&mut v[from * w..end * w], &c.v_bias);
                h.done += end - from;
                h.at = end;
                if end == n {
                    let keys = by_head(&h.cross[h.layer].0, n, s.decoder_heads, w);
                    h.cross[h.layer].0 = keys;
                    h.layer += 1;
                    h.at = 0;
                    if h.layer == self.decoder.len() {
                        h.x = Vec::new();
                        h.phase = Phase::Prompt;
                    }
                }
            }
            Phase::Prompt => {
                // A model of many languages told none: the language it hears is the likeliest language token after
                // the start of the transcript (as faster-whisper finds it).
                if h.language.is_none() {
                    let scores = self.decode_scores(h, &[s.sot], 0, pool);
                    let (first, last) = s.languages.unwrap_or((s.sot, s.sot));
                    let best = (first..=last).max_by(|&a, &b| scores[a as usize].total_cmp(&scores[b as usize])).unwrap_or(first);
                    h.language = Some(self.language_code(best).unwrap_or_else(|| String::from("en")));
                    for p in &mut h.past {
                        p.0.clear();
                        p.1.clear();
                    }
                }
                let mut prompt = vec![s.sot];
                if self.multilingual() {
                    let code = h.language.clone().unwrap_or_else(|| String::from("en"));
                    prompt.push(self.language_token(&code).unwrap_or(s.sot + 1));
                    prompt.push(s.transcribe.unwrap_or(s.sot));
                }
                prompt.push(s.no_timestamps);
                let rows = self.decode_rows(h, &prompt, 0, pool);
                // The chance of no speech, at the start of the transcript, before anything is suppressed.
                let at_sot = &rows[0..s.vocab];
                let max = at_sot.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                let sum: f64 = at_sot.iter().map(|&v| math::exp(v - max) as f64).sum();
                h.no_speech = (math::exp(at_sot[s.no_speech as usize] - max) as f64 / sum) as f32;
                h.next = rows[(prompt.len() - 1) * s.vocab..].to_vec();
                h.prompt = prompt;
                h.phase = Phase::Decode;
            }
            Phase::Decode => {
                let mut scores: Vec<f64> = h.next.iter().map(|&v| v as f64).collect();
                for &t in &s.suppress {
                    if let Some(v) = scores.get_mut(t as usize) {
                        *v = f64::NEG_INFINITY;
                    }
                }
                for v in scores.iter_mut().skip(s.timestamp_begin as usize) {
                    *v = f64::NEG_INFINITY;
                }
                if h.chosen.is_empty() {
                    for &t in &s.begin_suppress {
                        if let Some(v) = scores.get_mut(t as usize) {
                            *v = f64::NEG_INFINITY;
                        }
                    }
                }
                let (best, &top) = scores.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap_or((s.eot as usize, &0.0));
                let sum: f64 = scores.iter().filter(|v| v.is_finite()).map(|&v| math::exp((v - top) as f32) as f64).sum();
                h.logprob -= math::ln_f64(sum);
                let token = best as u32;
                let position = h.prompt.len() + h.chosen.len();
                if token == s.eot || h.chosen.len() >= MOST_TOKENS || position + 1 >= s.text_ctx {
                    return Some(self.finish(h));
                }
                h.chosen.push(token);
                h.next = self.decode_rows(h, &[token], position, pool);
            }
        }
        None
    }

    /// The transcript, from the tokens chosen.
    fn finish(&self, h: &mut Hearing) -> Transcript {
        let s = &self.settings;
        let mut bytes = Vec::new();
        for &t in &h.chosen {
            if t < s.eot {
                bytes.extend_from_slice(self.tokenizer.bytes(t));
            }
        }
        // (The mean is over the tokens and the end, as faster-whisper reckons it.)
        let avg_logprob = (h.logprob / (h.chosen.len() + 1) as f64) as f32;
        Transcript {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            tokens: core::mem::take(&mut h.chosen),
            no_speech: h.no_speech,
            avg_logprob,
            language: h.language.clone().unwrap_or_else(|| String::from("en")),
        }
    }

    /// The token for a language's code ("en": <|en|>), if the model knows it.
    pub fn language_token(&self, code: &str) -> Option<u32> {
        let (first, last) = self.settings.languages?;
        let id = self.tokenizer.special(&alloc::format!("<|{code}|>"))?;
        (first..=last).contains(&id).then_some(id)
    }

    /// The code of a language token.
    fn language_code(&self, token: u32) -> Option<String> {
        let b = self.tokenizer.bytes(token);
        let t = core::str::from_utf8(b).ok()?;
        Some(String::from(t.strip_prefix("<|")?.strip_suffix("|>")?))
    }

    /// The decoder's scores for each of `tokens` (at positions `start`.. on), the tokens' keys and values kept.
    fn decode_rows(&self, h: &mut Hearing, tokens: &[u32], start: usize, pool: &dyn Pool) -> Vec<f32> {
        let s = &self.settings;
        let w = s.width;
        let heads = s.decoder_heads;
        let t = tokens.len();
        let mut x = vec![0.0f32; t * w];
        let mut row = vec![0.0f32; w];
        for (i, &token) in tokens.iter().enumerate() {
            self.embed.row(token as usize % s.vocab, &mut row);
            let pos = &self.decoder_pos[(start + i) * w..(start + i + 1) * w];
            for ((o, e), p) in x[i * w..(i + 1) * w].iter_mut().zip(&row).zip(pos) {
                *o = e + p;
            }
        }
        for (l, layer) in self.decoder.iter().enumerate() {
            // Attention over the tokens so far (each sees itself and those before).
            let normed = layer_norm(&x, &layer.norm1, w);
            let ready: Vec<Prepared> = normed.chunks_exact(w).map(Prepared::new).collect();
            let a = &layer.attention;
            let mut q = a.q.times_all(&ready, pool);
            add_bias(&mut q, &a.q_bias);
            let k = a.k.times_all(&ready, pool);
            let mut v = a.v.times_all(&ready, pool);
            add_bias(&mut v, &a.v_bias);
            let (pk, pv) = &mut h.past[l];
            pk.extend_from_slice(&k);
            pv.extend_from_slice(&v);
            let seen = pk.len() / w;
            let mut att = vec![0.0f32; t * w];
            for i in 0..t {
                // (Token i sees the first seen - t + i + 1 keys.)
                let upto = seen - t + i + 1;
                let kt = by_head(&pk[..upto * w], upto, heads, w);
                attend(&q[i * w..(i + 1) * w], &kt, &pv[..upto * w], upto, heads, w, &mut att[i * w..(i + 1) * w], pool);
            }
            project_add(&a.o, &a.o_bias, &att, w, &mut x, pool);
            // Attention over the encoder's positions.
            let normed = layer_norm(&x, &layer.norm2, w);
            let ready: Vec<Prepared> = normed.chunks_exact(w).map(Prepared::new).collect();
            let c = &layer.cross;
            let mut q = c.q.times_all(&ready, pool);
            add_bias(&mut q, &c.q_bias);
            let (ck, cv) = &h.cross[l];
            attend(&q, ck, cv, s.audio_ctx, heads, w, &mut att, pool);
            project_add(&c.o, &c.o_bias, &att, w, &mut x, pool);
            feed_forward(&layer.mlp, &layer.norm3, w, &mut x, pool);
        }
        let normed = layer_norm(&x, &self.decoder_norm, w);
        let ready: Vec<Prepared> = normed.chunks_exact(w).map(Prepared::new).collect();
        self.embed.times_all(&ready, pool)
    }

    /// The scores after `tokens` alone, the tokens' keys and values left as they were.
    fn decode_scores(&self, h: &mut Hearing, tokens: &[u32], start: usize, pool: &dyn Pool) -> Vec<f32> {
        let kept: Vec<usize> = h.past.iter().map(|p| p.0.len()).collect();
        let rows = self.decode_rows(h, tokens, start, pool);
        for (p, &k) in h.past.iter_mut().zip(&kept) {
            p.0.truncate(k);
            p.1.truncate(k);
        }
        let v = self.settings.vocab;
        rows[rows.len() - v..].to_vec()
    }
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

fn add_bias(x: &mut [f32], bias: &[f32]) {
    for row in x.chunks_exact_mut(bias.len()) {
        for (v, b) in row.iter_mut().zip(bias) {
            *v += b;
        }
    }
}

/// Each row of `x` normalised (mean 0, variance 1), times w, plus b.
fn layer_norm(x: &[f32], n: &Norm, width: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; x.len()];
    for (row, o) in x.chunks_exact(width).zip(out.chunks_exact_mut(width)) {
        let mean = row.iter().map(|&v| v as f64).sum::<f64>() / width as f64;
        let var = row.iter().map(|&v| (v as f64 - mean) * (v as f64 - mean)).sum::<f64>() / width as f64;
        let r = 1.0 / math::sqrt64(var + EPS as f64);
        for (((o, &v), &g), &b) in o.iter_mut().zip(row).zip(&n.w).zip(&n.b) {
            *o = ((v as f64 - mean) * r) as f32 * g + b;
        }
    }
    out
}

/// x += (rows of `input`) times `m`, plus its bias.
fn project_add(m: &Matrix, bias: &[f32], input: &[f32], width: usize, x: &mut [f32], pool: &dyn Pool) {
    let ready: Vec<Prepared> = input.chunks_exact(m.cols).map(Prepared::new).collect();
    let mut out = vec![0.0f32; ready.len() * width];
    m.times_into(&ready, &mut out, pool);
    for ((v, o), b) in x.iter_mut().zip(&out).zip(bias.iter().cycle()) {
        *v += o + b;
    }
}

/// x += the feed-forward part of x (after its layer norm).
fn feed_forward(mlp: &Mlp, norm: &Norm, width: usize, x: &mut [f32], pool: &dyn Pool) {
    let normed = layer_norm(x, norm, width);
    let ready: Vec<Prepared> = normed.chunks_exact(width).map(Prepared::new).collect();
    drop(normed);
    let mut mid = mlp.fc1.times_all(&ready, pool);
    drop(ready);
    add_bias(&mut mid, &mlp.fc1_bias);
    vector::gelu(&mut mid);
    project_add(&mlp.fc2, &mlp.fc2_bias, &mid, width, x, pool);
}

/// Each head's keys laid out by dimension (`vector::scores`): of `n` keys (rows of `width`), each head's hd rows of n.
fn by_head(k: &[f32], n: usize, heads: usize, width: usize) -> Vec<f32> {
    let hd = width / heads;
    let mut kt = vec![0.0f32; n * width];
    for i in 0..n {
        for (c, &v) in k[i * width..(i + 1) * width].iter().enumerate() {
            let (h, d) = (c / hd, c % hd);
            kt[(h * hd + d) * n + i] = v;
        }
    }
    kt
}

/// Attention for some queries (`q`, rows of `width`) over `n` keys (by head: `by_head`) and values (rows of
/// `width`), each head on its own: into `out` (the queries' rows).
#[allow(clippy::too_many_arguments)]
fn attend(q: &[f32], kt: &[f32], v: &[f32], n: usize, heads: usize, width: usize, out: &mut [f32], pool: &dyn Pool) {
    let hd = width / heads;
    let queries = q.len() / width;
    let scale = 1.0 / math::sqrt(hd as f32);
    let out_ptr = SyncPtr(out.as_mut_ptr());
    // A job per head and a few queries.
    const QUERIES: usize = 8;
    let blocks = queries.div_ceil(QUERIES);
    pool.run(heads * blocks, &|j| {
        let (hh, block) = (j % heads, j / heads);
        let (first, last) = (block * QUERIES, ((block + 1) * QUERIES).min(queries));
        let count = last - first;
        let mut qs = vec![0.0f32; count * hd];
        for (i, t) in (first..last).enumerate() {
            qs[i * hd..(i + 1) * hd].copy_from_slice(&q[t * width + hh * hd..t * width + (hh + 1) * hd]);
        }
        let mut scores = vec![0.0f32; count * n];
        vector::scores(&qs, hd, &kt[hh * hd * n..(hh + 1) * hd * n], n, scale, &mut scores);
        for (i, t) in (first..last).enumerate() {
            let row = &mut scores[i * n..(i + 1) * n];
            let max = row.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            let sum = vector::exp_shifted(row, max);
            let inv = 1.0 / sum;
            for p in row.iter_mut() {
                *p *= inv;
            }
            // (Each job writes its own queries' rows of its own head.)
            let o = unsafe { core::slice::from_raw_parts_mut(out_ptr.get().add(t * width + hh * hd), hd) };
            vector::weighted_sum(row, &v[hh * hd..], width, hd, o);
        }
    });
}

/// The discrete Fourier transform of `n` complex numbers (`x[start]`, `x[start + stride]`...) into `out[..n]`, with
/// `twiddles` = e^(-2πik/N) (N a multiple of n): split into p transforms of n/p (p = 4, 2, 3 or 5, the first that
/// divides n), else directly.
fn fft(x: &[(f64, f64)], start: usize, stride: usize, n: usize, out: &mut [(f64, f64)], twiddles: &[(f64, f64)]) {
    let total = twiddles.len();
    let step = total / n;
    if n == 1 {
        out[0] = x[start];
        return;
    }
    let p = [4, 2, 3, 5].into_iter().find(|p| n.is_multiple_of(*p)).unwrap_or(n);
    if p == n {
        for (k, o) in out.iter_mut().take(n).enumerate() {
            let mut acc = (0.0, 0.0);
            for j in 0..n {
                let (a, b) = x[start + j * stride];
                let (c, s) = twiddles[(j * k * step) % total];
                acc.0 += a * c - b * s;
                acc.1 += a * s + b * c;
            }
            *o = acc;
        }
        return;
    }
    let m = n / p;
    let mut sub = vec![(0.0f64, 0.0f64); n];
    for q in 0..p {
        fft(x, start + q * stride, stride * p, m, &mut sub[q * m..(q + 1) * m], twiddles);
    }
    for k in 0..m {
        for j in 0..p {
            let index = k + j * m;
            let mut acc = (0.0, 0.0);
            for q in 0..p {
                let (a, b) = sub[q * m + k];
                let (c, s) = twiddles[(q * index * step) % total];
                acc.0 += a * c - b * s;
                acc.1 += a * s + b * c;
            }
            out[index] = acc;
        }
    }
}

/// The mel filters (`mels` by 201 frequencies of a 400-sample frame at 16 kHz), Slaney's, as librosa and
/// faster-whisper make them: triangles between points evenly spaced in mels from 0 to 8,000 Hz (linear below
/// 1,000 Hz, logarithmic above), each scaled to about the same energy.
pub fn mel_filters(mels: usize) -> Vec<f32> {
    const MAX_MEL: f64 = 45.245_640_471_924_965;
    let f_sp = 200.0 / 3.0;
    let min_log_mel = 1000.0 / f_sp;
    let logstep = math::ln_f64(6.4) / 27.0;
    let freqs: Vec<f64> = (0..mels + 2)
        .map(|i| {
            let mel = MAX_MEL * i as f64 / (mels + 1) as f64;
            if mel >= min_log_mel { 1000.0 * math::exp_f64(logstep * (mel - min_log_mel)) } else { f_sp * mel }
        })
        .collect();
    let mut out = vec![0.0f32; mels * BINS];
    for i in 0..mels {
        let enorm = 2.0 / (freqs[i + 2] - freqs[i]);
        for b in 0..BINS {
            let f = b as f64 * RATE as f64 / N_FFT as f64;
            let lower = (f - freqs[i]) / (freqs[i + 1] - freqs[i]);
            let upper = (freqs[i + 2] - f) / (freqs[i + 2] - freqs[i + 1]);
            out[i * BINS + b] = (lower.min(upper).max(0.0) * enorm) as f32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::Serial;

    #[test]
    fn fourier_transform_is_the_direct_one() {
        let mut r = crate::testing::Rng(7);
        let x: Vec<(f64, f64)> = (0..N_FFT).map(|_| (r.float() as f64, 0.0)).collect();
        let twiddles: Vec<(f64, f64)> = (0..N_FFT)
            .map(|k| {
                let (s, c) = math::sin_cos(-2.0 * core::f64::consts::PI * k as f64 / N_FFT as f64);
                (c, s)
            })
            .collect();
        let mut out = vec![(0.0, 0.0); N_FFT];
        fft(&x, 0, 1, N_FFT, &mut out, &twiddles);
        for (k, &(re, im)) in out.iter().enumerate() {
            let (mut a, mut b) = (0.0f64, 0.0f64);
            for (j, &(v, _)) in x.iter().enumerate() {
                let (c, s) = twiddles[(j * k) % N_FFT];
                a += v * c;
                b += v * s;
            }
            assert!((re - a).abs() < 1e-9 && (im - b).abs() < 1e-9, "bin {k}: {re} {im} vs {a} {b}");
        }
    }

    #[test]
    fn mel_filters_are_faster_whispers() {
        // (faster-whisper's FeatureExtractor.get_mel_filters(16000, 400, n), as float32.)
        let f = mel_filters(80);
        let sum: f64 = f.iter().map(|&v| v as f64).sum();
        assert!((sum - 1.999_024_102_9).abs() < 1e-6, "{sum}");
        assert!((f[1] - 0.024_862_594).abs() < 1e-8 && f[10 * BINS + 20] == 0.0);
        assert!((f[79 * BINS + 190] - 0.002_232_055_1).abs() < 1e-9);
        let f = mel_filters(128);
        let sum: f64 = f.iter().map(|&v| v as f64).sum();
        assert!((sum - 3.190_985_528_5).abs() < 1e-6, "{sum}");
        assert!((f[127 * BINS + 199] - 0.001_114_280_2).abs() < 1e-9);
    }

    #[test]
    fn features_are_faster_whispers() {
        // A second of 440 Hz and a little 3 kHz: faster-whisper's features (padded to 3,000 frames) at a few places.
        let file_bytes = crate::testing::tiny_listening();
        let file = File::open(&file_bytes).unwrap();
        let w = Whisper::load(&file).unwrap();
        let samples: Vec<f32> = (0..16000)
            .map(|i| {
                let t = i as f64 / 16000.0;
                (0.5 * math::sin_cos(2.0 * core::f64::consts::PI * 440.0 * t).0 + 0.1 * math::sin_cos(2.0 * core::f64::consts::PI * 3000.0 * t).0) as f32
            })
            .collect();
        let f = w.features(&samples);
        for (mel, frame, want) in [(0, 0, 0.988_746), (10, 50, 1.348_738), (40, 99, -0.401_552_1), (60, 100, 0.273_332_1), (79, 101, 0.0), (5, 2999, 0.0)] {
            let got = f[mel * FRAMES + frame];
            assert!((got - want).abs() < 2e-5, "mel {mel} frame {frame}: {got} vs {want}");
        }
    }

    #[test]
    fn a_step_at_a_time_is_the_same() {
        let file_bytes = crate::testing::tiny_listening();
        let file = File::open(&file_bytes).unwrap();
        let w = Whisper::load(&file).unwrap();
        assert!(!w.multilingual());
        let sound = crate::testing::recording(1.5, 3);
        let whole = w.transcribe(&sound, None, &Serial);
        assert!(!whole.tokens.is_empty() && whole.tokens.iter().all(|&t| t < w.settings.timestamp_begin && !w.settings.suppress.contains(&t)));
        assert!((0.0..=1.0).contains(&whole.no_speech) && whole.avg_logprob <= 0.0 && whole.language == "en");
        for most in [1, 7, 64, 700] {
            let mut h = w.begin(&sound, None);
            let mut steps = 0;
            let t = loop {
                steps += 1;
                if let Some(t) = w.advance(&mut h, most, &Serial) {
                    break t;
                }
            };
            assert_eq!(t, whole, "{most} a step");
            assert!(most > 64 || steps > 1500 / most);
        }
        // A file that isn't one: refused.
        let qwen = crate::testing::tiny(false);
        assert!(Whisper::load(&File::open(&qwen).unwrap()).is_err());
    }
}

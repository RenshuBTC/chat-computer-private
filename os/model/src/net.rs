//! The network: Qwen 3.5's (and 3.6's and 3.8's) language model, dense or with experts, a token or many at a time.
//!
//! Each layer normalises its input and mixes in what came before, then normalises again and goes through its
//! feed-forward part, each added to what went in. Three layers in four mix by Gated DeltaNet, a linear attention:
//! a fixed-size state per head that each token decays, corrects towards its own value (the delta rule) and reads;
//! the fourth by ordinary attention over every token so far (8 or more query heads sharing 2 to 4 key-value heads,
//! a gate on its output, rotary positions on a quarter of each head). The feed-forward part is SwiGLU, or 8 of 256
//! small experts (and one shared) chosen per token.
//!
//! Pictures are read as rows in place of tokens (made by the vision part, vision.rs): a picture's rows are "tokens"
//! PICTURE | slot << 16 | row, whose rows the caller has (`Rows`). Their positions are three (Qwen's M-RoPE: a time,
//! a row and a column, the rotary pairs taking turns at which they turn by); text's three are the same, from where the
//! last picture ended on.

use crate::file::{Error, File};
use crate::math;
use crate::quant::{self, Dtype, Prepared, rms_norm};
use crate::token::Tokenizer;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Tokens read at once: enough that a weight read from memory serves many, few enough that their numbers stay in
/// the processor's caches.
pub const BATCH: usize = 64;

/// Runs jobs, on as many processors as it has: the model's arithmetic splits each big step into many.
pub trait Pool: Sync {
    /// Calls `f(i)` for every i in 0..jobs, in any order, then returns.
    fn run(&self, jobs: usize, f: &(dyn Fn(usize) + Sync));
}

/// One processor: the jobs one after another.
pub struct Serial;

impl Pool for Serial {
    fn run(&self, jobs: usize, f: &(dyn Fn(usize) + Sync)) {
        for i in 0..jobs {
            f(i);
        }
    }
}

/// The model's settings.
#[derive(Clone, Debug)]
pub struct Settings {
    pub hidden: usize,
    pub layers: usize,
    pub heads: usize,
    pub kv_heads: usize,
    pub head_dim: usize,
    pub intermediate: usize,
    pub experts: usize,
    pub experts_used: usize,
    pub expert_intermediate: usize,
    pub shared_intermediate: usize,
    pub lk_heads: usize,
    pub lv_heads: usize,
    pub lk_dim: usize,
    pub lv_dim: usize,
    pub conv_kernel: usize,
    /// Which layers are full attention (every `interval`th, the last of each run).
    pub interval: usize,
    pub vocab: usize,
    pub rope_theta: f64,
    pub rotary_dim: usize,
    pub eps: f32,
    pub context: usize,
    /// How many rotary pairs turn by a picture's time, row and column (M-RoPE's sections), and whether they take
    /// turns (interleaved) rather than coming one section after another.
    pub mrope: [usize; 3],
    pub mrope_interleaved: bool,
    /// Which converter made the file (convert::CONVERTER; 1, made before the vision part was kept, says nothing).
    pub converter: u32,
}

impl Settings {
    pub fn full_attention(&self, layer: usize) -> bool {
        (layer + 1).is_multiple_of(self.interval)
    }

    fn key_dim(&self) -> usize {
        self.lk_heads * self.lk_dim
    }

    fn value_dim(&self) -> usize {
        self.lv_heads * self.lv_dim
    }

    fn conv_dim(&self) -> usize {
        2 * self.key_dim() + self.value_dim()
    }
}

/// A weight matrix in the file: rows of `cols` numbers.
#[derive(Clone, Copy)]
pub struct Matrix<'a> {
    pub dtype: Dtype,
    pub rows: usize,
    pub cols: usize,
    pub data: &'a [u8],
}

impl Matrix<'_> {
    /// out = this · x, its rows split among the pool's processors.
    pub fn times(&self, x: &Prepared, out: &mut [f32], pool: &dyn Pool) {
        self.times_into(core::slice::from_ref(x), out, pool);
    }

    /// For each token's vector: this · x, one after another in `out` (rows numbers each). Each row is read once
    /// for all of them.
    pub fn times_into(&self, xs: &[Prepared], out: &mut [f32], pool: &dyn Pool) {
        let rows = self.rows;
        let n = xs.len();
        if n == 0 {
            return;
        }
        let rows_out = if n == 1 { rows.min(out.len()) } else { rows };
        assert!(out.len() >= rows_out + (n - 1) * rows);
        const ROWS_PER_JOB: usize = 64;
        let jobs = rows_out.div_ceil(ROWS_PER_JOB);
        let out_ptr = SyncPtr(out.as_mut_ptr());
        let m = *self;
        let xs: Vec<&Prepared> = xs.iter().collect();
        let xs = &xs[..];
        pool.run(jobs, &|j| {
            let start = j * ROWS_PER_JOB;
            let end = (start + ROWS_PER_JOB).min(rows_out);
            // (On the stack for a few tokens, the usual case: one job after another needs no memory from the heap,
            // which every core would otherwise take in turn, in Chat Computer's own kernel.)
            let mut few = [0.0f32; 16];
            let mut many = Vec::new();
            let row_out: &mut [f32] = if n <= few.len() {
                &mut few[..n]
            } else {
                many.resize(n, 0.0f32);
                &mut many
            };
            for r in start..end {
                quant::dot_row_many(m.dtype, m.data, m.cols, r, xs, row_out);
                for (t, v) in row_out.iter().enumerate() {
                    // (Each job writes its own rows.)
                    unsafe { *out_ptr.get().add(t * rows + r) = *v };
                }
            }
        });
    }

    /// For each token's vector: this · x, in a new vector (rows numbers per token).
    pub fn times_all(&self, xs: &[Prepared], pool: &dyn Pool) -> Vec<f32> {
        let mut out = vec![0.0f32; self.rows * xs.len()];
        self.times_into(xs, &mut out, pool);
        out
    }

    /// Row r as f32s.
    pub fn row(&self, r: usize, out: &mut [f32]) {
        quant::row_f32(self.dtype, self.data, self.cols, r, out);
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

struct Linear<'a> {
    qkv: Matrix<'a>,
    z: Matrix<'a>,
    b: Matrix<'a>,
    a: Matrix<'a>,
    /// The short convolution's weights: per channel, `conv_kernel` of them.
    conv: Vec<f32>,
    /// -exp(A_log), per value head.
    neg_a: Vec<f32>,
    dt_bias: Vec<f32>,
    norm: Vec<f32>,
    out: Matrix<'a>,
}

struct Full<'a> {
    q: Matrix<'a>,
    k: Matrix<'a>,
    v: Matrix<'a>,
    o: Matrix<'a>,
    q_norm: Vec<f32>,
    k_norm: Vec<f32>,
}

enum Mixer<'a> {
    Linear(Linear<'a>),
    Full(Full<'a>),
}

enum Ffn<'a> {
    Dense { gate: Matrix<'a>, up: Matrix<'a>, down: Matrix<'a> },
    Experts {
        router: Matrix<'a>,
        /// Every expert's gate and up rows (2 x expert_intermediate each, one after another), and down rows.
        gate_up: Matrix<'a>,
        down: Matrix<'a>,
        shared_gate: Matrix<'a>,
        shared_up: Matrix<'a>,
        shared_down: Matrix<'a>,
        shared_weight: Matrix<'a>,
    },
}

struct Layer<'a> {
    norm1: Vec<f32>,
    norm2: Vec<f32>,
    mixer: Mixer<'a>,
    ffn: Ffn<'a>,
}

/// A direction taken out of what the model thinks with (its residual stream), at every layer and every token: a
/// model's refusals are written along one direction, and taking that out as the model runs is the same as taking it
/// out of every weight that writes there (Arditi et al., 2024, "Refusal in language models is mediated by a single
/// direction"). The weights themselves stay as they were.
pub struct Ablation {
    /// Unit length, as long as the model is wide.
    pub direction: Vec<f32>,
    /// How much of the direction's part goes: 1, all of it.
    pub strength: f32,
}

/// The model, over its file's bytes.
pub struct Model<'a> {
    pub settings: Settings,
    pub tokenizer: Tokenizer,
    embed: Matrix<'a>,
    head: Matrix<'a>,
    norm: Vec<f32>,
    layers: Vec<Layer<'a>>,
    /// The rotary frequencies (one per pair of rotated numbers), and which of a token's three positions each pair
    /// turns by (0: time, 1: row, 2: column; text's three are the same).
    inv_freq: Vec<f64>,
    axis: Vec<u8>,
    /// A direction taken out as it runs (refusal.rs finds it), if any.
    pub ablation: Option<Ablation>,
    /// What makes pictures into rows (None: the model's file has no vision part, so it can't see).
    pub vision: Option<crate::vision::Vision<'a>>,
}

/// A picture's row read in place of a token: PICTURE | slot << 16 | row.
pub const PICTURE: u32 = 0x8000_0000;

/// The token for row `row` of picture `slot`.
pub fn picture_token(slot: u32, row: u32) -> u32 {
    PICTURE | (slot & 0x7FFF) << 16 | (row & 0xFFFF)
}

/// Which picture and row a token is, if it's one.
pub fn picture_of(token: u32) -> Option<(u32, u32)> {
    (token & PICTURE != 0).then(|| ((token >> 16) & 0x7FFF, token & 0xFFFF))
}

/// Pictures' rows, for the tokens that are them.
pub trait Rows {
    /// Picture `slot`'s rows (each as wide as the model), and how many rows and columns of them its grid has.
    fn rows(&self, slot: u32) -> Option<(&[f32], usize, usize)>;
}

/// No pictures.
pub struct NoPictures;

impl Rows for NoPictures {
    fn rows(&self, _slot: u32) -> Option<(&[f32], usize, usize)> {
        None
    }
}

pub(crate) fn vector(file: &File, name: &str) -> Result<Vec<f32>, Error> {
    let e = file.get(name).ok_or(Error::Missing)?;
    let n = e.elements();
    let mut out = vec![0.0; n];
    quant::row_f32(e.dtype, file.data(e), n, 0, &mut out);
    Ok(out)
}

pub(crate) fn matrix<'a>(file: &File<'a>, name: &str, rows: usize, cols: usize) -> Result<Matrix<'a>, Error> {
    let e = file.get(name).ok_or(Error::Missing)?;
    if e.elements() != rows * cols {
        return Err(Error::Damaged);
    }
    Ok(Matrix { dtype: e.dtype, rows, cols, data: file.data(e) })
}

impl<'a> Model<'a> {
    pub fn load(file: &File<'a>) -> Result<Model<'a>, Error> {
        let c = file.config()?;
        let experts = c.usize("experts").unwrap_or(0);
        let s = Settings {
            hidden: c.usize("hidden")?,
            layers: c.usize("layers")?,
            heads: c.usize("heads")?,
            kv_heads: c.usize("kv_heads")?,
            head_dim: c.usize("head_dim")?,
            intermediate: c.usize("intermediate").unwrap_or(0),
            experts,
            experts_used: if experts > 0 { c.usize("experts_used")? } else { 0 },
            expert_intermediate: if experts > 0 { c.usize("expert_intermediate")? } else { 0 },
            shared_intermediate: if experts > 0 { c.usize("shared_intermediate")? } else { 0 },
            lk_heads: c.usize("linear_key_heads")?,
            lv_heads: c.usize("linear_value_heads")?,
            lk_dim: c.usize("linear_key_dim")?,
            lv_dim: c.usize("linear_value_dim")?,
            conv_kernel: c.usize("conv_kernel")?,
            interval: c.usize("full_attention_interval")?,
            vocab: c.usize("vocab")?,
            rope_theta: c.f64("rope_theta")?,
            rotary_dim: c.usize("rotary_dim")?,
            eps: c.f64("rms_eps")? as f32,
            context: c.usize("context").unwrap_or(32768),
            mrope: {
                let mut m = [0usize; 3];
                if let Some(v) = c.str("mrope_section") {
                    for (slot, part) in m.iter_mut().zip(v.split(',')) {
                        *slot = part.trim().parse().unwrap_or(0);
                    }
                }
                m
            },
            mrope_interleaved: c.str("mrope_interleaved") == Some("1"),
            converter: c.str("converter").and_then(|v| v.parse().ok()).unwrap_or(1),
        };
        if s.interval == 0 || !s.heads.is_multiple_of(s.kv_heads.max(1)) || !s.lv_heads.is_multiple_of(s.lk_heads.max(1))
            || s.rotary_dim > s.head_dim || !s.rotary_dim.is_multiple_of(2) || !s.hidden.is_multiple_of(quant::BLOCK)
            || (experts > 0 && (s.experts_used == 0 || s.experts_used > experts))
        {
            return Err(Error::Config);
        }
        let h = s.hidden;
        let embed = matrix(file, "embed", s.vocab, h)?;
        let head = if file.get("head").is_some() { matrix(file, "head", s.vocab, h)? } else { embed };
        let mut layers = Vec::with_capacity(s.layers);
        for l in 0..s.layers {
            let p = |n: &str| format!("layers.{l}.{n}");
            let mixer = if s.full_attention(l) {
                Mixer::Full(Full {
                    q: matrix(file, &p("q"), s.heads * s.head_dim * 2, h)?,
                    k: matrix(file, &p("k"), s.kv_heads * s.head_dim, h)?,
                    v: matrix(file, &p("v"), s.kv_heads * s.head_dim, h)?,
                    o: matrix(file, &p("o"), h, s.heads * s.head_dim)?,
                    q_norm: vector(file, &p("q_norm"))?,
                    k_norm: vector(file, &p("k_norm"))?,
                })
            } else {
                let a_log = vector(file, &p("a_log"))?;
                Mixer::Linear(Linear {
                    qkv: matrix(file, &p("qkv"), s.conv_dim(), h)?,
                    z: matrix(file, &p("z"), s.value_dim(), h)?,
                    b: matrix(file, &p("b"), s.lv_heads, h)?,
                    a: matrix(file, &p("a"), s.lv_heads, h)?,
                    conv: vector(file, &p("conv"))?,
                    neg_a: a_log.iter().map(|&v| -math::exp(v)).collect(),
                    dt_bias: vector(file, &p("dt_bias"))?,
                    norm: vector(file, &p("linear_norm"))?,
                    out: matrix(file, &p("out"), h, s.value_dim())?,
                })
            };
            let ffn = if experts > 0 {
                let ei = s.expert_intermediate;
                Ffn::Experts {
                    router: matrix(file, &p("router"), experts, h)?,
                    gate_up: matrix(file, &p("experts_gate_up"), experts * 2 * ei, h)?,
                    down: matrix(file, &p("experts_down"), experts * h, ei)?,
                    shared_gate: matrix(file, &p("shared_gate"), s.shared_intermediate, h)?,
                    shared_up: matrix(file, &p("shared_up"), s.shared_intermediate, h)?,
                    shared_down: matrix(file, &p("shared_down"), h, s.shared_intermediate)?,
                    shared_weight: matrix(file, &p("shared_weight"), 1, h)?,
                }
            } else {
                Ffn::Dense {
                    gate: matrix(file, &p("gate"), s.intermediate, h)?,
                    up: matrix(file, &p("up"), s.intermediate, h)?,
                    down: matrix(file, &p("down"), h, s.intermediate)?,
                }
            };
            let layer = Layer { norm1: vector(file, &p("norm1"))?, norm2: vector(file, &p("norm2"))?, mixer, ffn };
            if let Mixer::Linear(lin) = &layer.mixer
                && (lin.conv.len() != s.conv_dim() * s.conv_kernel || lin.neg_a.len() != s.lv_heads
                    || lin.dt_bias.len() != s.lv_heads || lin.norm.len() != s.lv_dim)
            {
                return Err(Error::Damaged);
            }
            if let Mixer::Full(f) = &layer.mixer
                && (f.q_norm.len() != s.head_dim || f.k_norm.len() != s.head_dim)
            {
                return Err(Error::Damaged);
            }
            if layer.norm1.len() != h || layer.norm2.len() != h {
                return Err(Error::Damaged);
            }
            layers.push(layer);
        }
        let norm = vector(file, "norm")?;
        if norm.len() != h {
            return Err(Error::Damaged);
        }
        let inv_freq = (0..s.rotary_dim / 2)
            .map(|i| 1.0 / math::powf64(s.rope_theta, (2 * i) as f64 / s.rotary_dim as f64))
            .collect();
        // Which position each pair turns by: interleaved (time, row, column, time...: the rows' and columns' turns
        // as many as their sections), or one section after another.
        let [t_sec, h_sec, w_sec] = s.mrope;
        let axis = (0..s.rotary_dim / 2)
            .map(|j| {
                if s.mrope_interleaved {
                    if j % 3 == 1 && j < 3 * h_sec {
                        1
                    } else if j % 3 == 2 && j < 3 * w_sec {
                        2
                    } else {
                        0
                    }
                } else if j < t_sec {
                    0
                } else if j < t_sec + h_sec {
                    1
                } else if j < t_sec + h_sec + w_sec {
                    2
                } else {
                    0
                }
            })
            .collect();
        let tokenizer = crate::load_tokenizer(file)?;
        let vision = crate::vision::Vision::load(file)?;
        if vision.as_ref().is_some_and(|v| v.settings.out != h) {
            return Err(Error::Damaged);
        }
        Ok(Model { settings: s, tokenizer, embed, head, norm, layers, inv_freq, axis, ablation: None, vision })
    }

    /// A new conversation's state (nothing seen yet).
    pub fn state(&self) -> State {
        let s = &self.settings;
        let mut linear = Vec::new();
        let mut full = Vec::new();
        for l in 0..s.layers {
            if s.full_attention(l) {
                full.push(Cache { k: Vec::new(), v: Vec::new() });
            } else {
                linear.push(Recurrent {
                    conv: vec![0.0; s.conv_dim() * (s.conv_kernel - 1)],
                    state: vec![0.0; s.lv_heads * s.lk_dim * s.lv_dim],
                });
            }
        }
        State { pos: 0, rope: 0, base: 0, linear, full }
    }

    /// Feeds one token (at the state's next position); with `logits`, the scores of what may come next.
    pub fn step(&self, st: &mut State, token: u32, logits: Option<&mut [f32]>, pool: &dyn Pool) {
        self.forward(st, &[token], logits, pool);
    }

    /// Feeds tokens (from the state's next position on), many at a time: each weight is read once for all of
    /// them, which is what makes reading a long prompt fast. With `logits`, the scores after the last.
    pub fn forward(&self, st: &mut State, tokens: &[u32], logits: Option<&mut [f32]>, pool: &dyn Pool) {
        self.forward_with(st, tokens, &NoPictures, logits, None, pool);
    }

    /// The same, with pictures' rows among the tokens (`pictures` has them).
    pub fn forward_seeing(&self, st: &mut State, tokens: &[u32], pictures: &dyn Rows, logits: Option<&mut [f32]>, pool: &dyn Pool) {
        self.forward_with(st, tokens, pictures, logits, None, pool);
    }

    /// Feeds tokens as forward does, and gives what the model thinks with (the residual stream) after the last of
    /// them: as each layer starts (one for each layer), and after the last (one more).
    pub fn residuals(&self, st: &mut State, tokens: &[u32], pool: &dyn Pool) -> Vec<Vec<f32>> {
        let mut out = Vec::new();
        self.forward_with(st, tokens, &NoPictures, None, Some(&mut out), pool);
        out
    }

    fn forward_with(&self, st: &mut State, tokens: &[u32], pictures: &dyn Rows, mut logits: Option<&mut [f32]>,
                    mut capture: Option<&mut Vec<Vec<f32>>>, pool: &dyn Pool) {
        // (With experts, more at once: each expert is chosen by few of the tokens.)
        let batch = if self.settings.experts > 0 { 4 * BATCH } else { BATCH };
        let chunks = tokens.len().div_ceil(batch).max(1);
        for (i, chunk) in tokens.chunks(batch).enumerate() {
            let last = i + 1 == chunks;
            let (l, c) = if last { (logits.as_deref_mut(), capture.as_deref_mut()) } else { (None, None) };
            self.batch(st, chunk, pictures, l, c, pool);
        }
    }

    /// The direction's part taken out of each token's row (x: n rows, each as wide as the model).
    fn ablate(&self, x: &mut [f32], n: usize) {
        let Some(a) = &self.ablation else { return };
        let h = self.settings.hidden;
        for row in x[..n * h].chunks_exact_mut(h) {
            let along: f32 = row.iter().zip(&a.direction).map(|(v, d)| v * d).sum::<f32>() * a.strength;
            for (v, d) in row.iter_mut().zip(&a.direction) {
                *v -= along * d;
            }
        }
    }

    fn batch(&self, st: &mut State, tokens: &[u32], pictures: &dyn Rows, logits: Option<&mut [f32]>,
             mut capture: Option<&mut Vec<Vec<f32>>>, pool: &dyn Pool) {
        let s = &self.settings;
        let (h, n) = (s.hidden, tokens.len());
        if n == 0 {
            return;
        }
        let mut x = vec![0.0f32; n * h];
        // Each token's three positions: text's from where the last ended on; a picture's row's from where the picture
        // starts (its time), plus its row and column; after a picture, text goes on from as far as its larger side.
        let mut positions = Vec::with_capacity(n);
        for (t, &tok) in tokens.iter().enumerate() {
            let at = &mut x[t * h..(t + 1) * h];
            match picture_of(tok) {
                None => {
                    self.embed.row((tok as usize).min(s.vocab - 1), at);
                    positions.push([st.rope; 3]);
                    st.rope += 1;
                }
                Some((slot, row)) => {
                    let (rows, gh, gw) = match pictures.rows(slot) {
                        Some((rows, gh, gw)) if (row as usize) < gh * gw && rows.len() == gh * gw * h => (Some(rows), gh, gw),
                        // (A picture whose rows aren't there: nothing for its rows, though it takes its places.)
                        Some((_, gh, gw)) => (None, gh.max(1), gw.max(1)),
                        None => (None, 1, 1),
                    };
                    if let Some(rows) = rows {
                        at.copy_from_slice(&rows[row as usize * h..(row as usize + 1) * h]);
                    }
                    if row == 0 {
                        st.base = st.rope;
                    }
                    let (r, c) = ((row as usize / gw) as i64, (row as usize % gw) as i64);
                    positions.push([st.base, st.base + r, st.base + c]);
                    if row as usize + 1 >= gh * gw {
                        st.rope = st.base + gh.max(gw) as i64;
                    }
                }
            }
        }
        self.ablate(&mut x, n);
        let mut normed = vec![0.0f32; n * h];
        let mut out = vec![0.0f32; n * h];
        let (mut li, mut fi) = (0, 0);
        let pos = st.pos;
        for layer in &self.layers {
            if let Some(c) = capture.as_deref_mut() {
                c.push(x[(n - 1) * h..].to_vec());
            }
            let xs = prepare_norm(&x, &layer.norm1, s.eps, n, &mut normed);
            match &layer.mixer {
                Mixer::Linear(lin) => {
                    self.linear(lin, &mut st.linear[li], &xs, &mut out, pool);
                    li += 1;
                }
                Mixer::Full(f) => {
                    self.full(f, &mut st.full[fi], pos, &positions, &xs, &mut out, pool);
                    fi += 1;
                }
            }
            for (a, b) in x.iter_mut().zip(&out) {
                *a += b;
            }
            self.ablate(&mut x, n);
            let xs = prepare_norm(&x, &layer.norm2, s.eps, n, &mut normed);
            self.ffn(&layer.ffn, &xs, &mut out, pool);
            for (a, b) in x.iter_mut().zip(&out) {
                *a += b;
            }
            self.ablate(&mut x, n);
        }
        if let Some(c) = capture {
            c.push(x[(n - 1) * h..].to_vec());
        }
        st.pos += n;
        if let Some(logits) = logits {
            let mut last = vec![0.0f32; h];
            rms_norm(&x[(n - 1) * h..], &self.norm, true, s.eps, &mut last);
            self.head.times(&Prepared::new(&last), logits, pool);
        }
    }

    /// Gated DeltaNet: each token's query, key and value (after a short convolution over the last few tokens),
    /// its decay and its learning rate (beta); each value head's state decays, learns the key-value pair by the
    /// delta rule, and is read by the query, token after token; each read is normalised and gated.
    fn linear(&self, w: &Linear, r: &mut Recurrent, xs: &[Prepared], out: &mut [f32], pool: &dyn Pool) {
        let s = &self.settings;
        let n = xs.len();
        let (kd, vd, cd, kk) = (s.key_dim(), s.value_dim(), s.conv_dim(), s.conv_kernel);
        let (qkv, z, b, a) = (w.qkv.times_all(xs, pool), w.z.times_all(xs, pool), w.b.times_all(xs, pool), w.a.times_all(xs, pool));
        // The convolution: each channel, over its last kk - 1 inputs and this one; SiLU. Channels are apart: in
        // jobs of 256.
        let past = kk - 1;
        let mut mixed = vec![0.0f32; n * cd];
        let mixed_ptr = SyncPtr(mixed.as_mut_ptr());
        let conv_ptr = SyncPtr(r.conv.as_mut_ptr());
        const CHANNELS: usize = 256;
        pool.run(cd.div_ceil(CHANNELS), &|j| {
            for c in j * CHANNELS..((j + 1) * CHANNELS).min(cd) {
                // (Each job has its own channels' history and outputs.)
                let hist = unsafe { core::slice::from_raw_parts_mut(conv_ptr.get().add(c * past), past) };
                let wts = &w.conv[c * kk..(c + 1) * kk];
                for t in 0..n {
                    let now = qkv[t * cd + c];
                    let mut v = wts[past] * now;
                    for i in 0..past {
                        v += wts[i] * hist[i];
                    }
                    // (Shifted along: the oldest goes.)
                    for i in 0..past.saturating_sub(1) {
                        hist[i] = hist[i + 1];
                    }
                    if past > 0 {
                        hist[past - 1] = now;
                    }
                    unsafe { *mixed_ptr.get().add(t * cd + c) = math::silu(v) };
                }
            }
        });
        // Queries and keys normalised to length 1 per head; queries then scaled by 1/sqrt(dk).
        let dk = s.lk_dim;
        let qscale = 1.0 / math::sqrt(dk as f32);
        for t in 0..n {
            let m = &mut mixed[t * cd..(t + 1) * cd];
            for hd in 0..2 * s.lk_heads {
                let scale = if hd < s.lk_heads { qscale } else { 1.0 };
                let v = &mut m[hd * dk..(hd + 1) * dk];
                let norm = 1.0 / math::sqrt(v.iter().map(|t| t * t).sum::<f32>() + 1e-6);
                for t in v.iter_mut() {
                    *t *= norm * scale;
                }
            }
        }
        let dv = s.lv_dim;
        let ratio = s.lv_heads / s.lk_heads;
        let mut read = vec![0.0f32; n * vd];
        let read_ptr = SyncPtr(read.as_mut_ptr());
        let state_ptr = SyncPtr(r.state.as_mut_ptr());
        let mixed = &mixed;
        pool.run(s.lv_heads, &|hv| {
            let hk = hv / ratio;
            // This head's state: dk rows of dv (its own: one job per head).
            let st = unsafe { core::slice::from_raw_parts_mut(state_ptr.get().add(hv * dk * dv), dk * dv) };
            let mut delta = vec![0.0f32; dv];
            for t in 0..n {
                let m = &mixed[t * cd..(t + 1) * cd];
                let qh = &m[hk * dk..(hk + 1) * dk];
                let kh = &m[kd + hk * dk..kd + (hk + 1) * dk];
                let vh = &m[2 * kd + hv * dv..2 * kd + (hv + 1) * dv];
                let beta = math::sigmoid(b[t * s.lv_heads + hv]);
                let g = w.neg_a[hv] * math::softplus(a[t * s.lv_heads + hv] + w.dt_bias[hv]);
                let decay = math::exp(g);
                for v in st.iter_mut() {
                    *v *= decay;
                }
                // What the state says the key's value is; the correction towards the real one.
                delta.fill(0.0);
                for i in 0..dk {
                    quant::axpy(kh[i], &st[i * dv..(i + 1) * dv], &mut delta);
                }
                for j in 0..dv {
                    delta[j] = (vh[j] - delta[j]) * beta;
                }
                for i in 0..dk {
                    quant::axpy(kh[i], &delta, &mut st[i * dv..(i + 1) * dv]);
                }
                let o = unsafe { core::slice::from_raw_parts_mut(read_ptr.get().add(t * vd + hv * dv), dv) };
                o.fill(0.0);
                for i in 0..dk {
                    quant::axpy(qh[i], &st[i * dv..(i + 1) * dv], o);
                }
            }
        });
        // Each head's read: normalised (times the norm's weights), times SiLU of its gate z.
        let mut gated = vec![0.0f32; vd];
        let mut normed = vec![0.0f32; dv];
        let mut ready = Vec::with_capacity(n);
        for t in 0..n {
            for hv in 0..s.lv_heads {
                let o = &read[t * vd + hv * dv..t * vd + (hv + 1) * dv];
                rms_norm(o, &w.norm, false, s.eps, &mut normed);
                for j in 0..dv {
                    gated[hv * dv + j] = normed[j] * math::silu(z[t * vd + hv * dv + j]);
                }
            }
            ready.push(Prepared::new(&gated));
        }
        w.out.times_into(&ready, out, pool);
    }

    /// Attention over every token so far: queries (and their gates), keys and values; queries and keys
    /// normalised per head and turned by their position; the output gated, then projected.
    fn full(&self, w: &Full, c: &mut Cache, pos: usize, positions: &[[i64; 3]], xs: &[Prepared], out: &mut [f32], pool: &dyn Pool) {
        let s = &self.settings;
        let (n, hd) = (xs.len(), s.head_dim);
        let (qd, kvd) = (s.heads * hd, s.kv_heads * hd);
        let (qg, k, v) = (w.q.times_all(xs, pool), w.k.times_all(xs, pool), w.v.times_all(xs, pool));
        let mut q = vec![0.0f32; n * qd];
        let mut gate = vec![0.0f32; n * qd];
        let mut kn = vec![0.0f32; n * kvd];
        let half = s.rotary_dim / 2;
        let mut cs = vec![(0.0f32, 0.0f32); half];
        for t in 0..n {
            for h in 0..s.heads {
                let src = &qg[t * 2 * qd + h * 2 * hd..t * 2 * qd + (h + 1) * 2 * hd];
                rms_norm(&src[..hd], &w.q_norm, true, s.eps, &mut q[t * qd + h * hd..t * qd + (h + 1) * hd]);
                gate[t * qd + h * hd..t * qd + (h + 1) * hd].copy_from_slice(&src[hd..]);
            }
            for h in 0..s.kv_heads {
                let at = t * kvd + h * hd;
                rms_norm(&k[at..at + hd], &w.k_norm, true, s.eps, &mut kn[at..at + hd]);
            }
            // Rotary positions on each head's first rotary_dim numbers: pairs (i, i + rotary_dim/2), each turned by
            // its own one of the token's three positions.
            for (i, f) in self.inv_freq.iter().enumerate() {
                let (sn, cn) = math::sin_cos(positions[t][self.axis[i] as usize] as f64 * f);
                cs[i] = (cn as f32, sn as f32);
            }
            let rotate = |head: &mut [f32]| {
                for i in 0..half {
                    let (cn, sn) = cs[i];
                    let (a, b) = (head[i], head[i + half]);
                    head[i] = a * cn - b * sn;
                    head[i + half] = b * cn + a * sn;
                }
            };
            for h in 0..s.heads {
                rotate(&mut q[t * qd + h * hd..t * qd + (h + 1) * hd]);
            }
            for h in 0..s.kv_heads {
                rotate(&mut kn[t * kvd + h * hd..t * kvd + (h + 1) * hd]);
            }
        }
        c.k.extend_from_slice(&kn);
        c.v.extend_from_slice(&v);
        let group = s.heads / s.kv_heads;
        let scale = 1.0 / math::sqrt(hd as f32);
        let mut att = vec![0.0f32; n * qd];
        let att_ptr = SyncPtr(att.as_mut_ptr());
        let (q, ck, cv) = (&q, &c.k, &c.v);
        // A job per head and token: each token sees itself and everything before it.
        pool.run(s.heads * n, &|j| {
            let (h, t) = (j % s.heads, j / s.heads);
            let kv = h / group;
            let seen = pos + t + 1;
            let qh = &q[t * qd + h * hd..t * qd + (h + 1) * hd];
            let mut scores = vec![0.0f32; seen];
            let mut max = f32::NEG_INFINITY;
            for (i, sc) in scores.iter_mut().enumerate() {
                *sc = quant::dot(qh, &ck[i * kvd + kv * hd..i * kvd + (kv + 1) * hd]) * scale;
                max = max.max(*sc);
            }
            let mut sum = 0.0f32;
            for sc in scores.iter_mut() {
                *sc = math::exp(*sc - max);
                sum += *sc;
            }
            let o = unsafe { core::slice::from_raw_parts_mut(att_ptr.get().add(t * qd + h * hd), hd) };
            o.fill(0.0);
            for (i, sc) in scores.iter().enumerate() {
                quant::axpy(sc / sum, &cv[i * kvd + kv * hd..i * kvd + (kv + 1) * hd], o);
            }
        });
        for (a, g) in att.iter_mut().zip(&gate) {
            *a *= math::sigmoid(*g);
        }
        let ready: Vec<Prepared> = att.chunks_exact(qd).map(Prepared::new).collect();
        w.o.times_into(&ready, out, pool);
    }

    fn ffn(&self, f: &Ffn, xs: &[Prepared], out: &mut [f32], pool: &dyn Pool) {
        let s = &self.settings;
        let (n, h) = (xs.len(), s.hidden);
        match f {
            Ffn::Dense { gate, up, down } => swiglu(gate, up, down, xs, out, pool),
            Ffn::Experts { router, gate_up, down, shared_gate, shared_up, shared_down, shared_weight } => {
                let logits = router.times_all(xs, pool);
                // Per token: softmax over all, the top k kept and made to sum to 1 (as the reference does).
                // Then the (token, expert) pairs, by expert.
                let mut pairs: Vec<(usize, usize, f32)> = Vec::with_capacity(n * s.experts_used);
                for t in 0..n {
                    let l = &logits[t * s.experts..(t + 1) * s.experts];
                    let max = l.iter().fold(f32::NEG_INFINITY, |m, &v| m.max(v));
                    let mut probs: Vec<(f32, usize)> = l.iter().enumerate().map(|(i, &v)| (math::exp(v - max), i)).collect();
                    let total: f32 = probs.iter().map(|p| p.0).sum();
                    for p in probs.iter_mut() {
                        p.0 /= total;
                    }
                    probs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(core::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
                    probs.truncate(s.experts_used);
                    let kept: f32 = probs.iter().map(|p| p.0).sum();
                    for &(p, e) in &probs {
                        pairs.push((e, t, p / kept));
                    }
                }
                pairs.sort_by_key(|&(e, t, _)| (e, t));
                // Each expert's gate and up rows, for its tokens: a job per expert and block of rows.
                let ei = s.expert_intermediate;
                let mut groups: Vec<(usize, usize, usize)> = Vec::new(); // expert, first pair, pairs
                for (i, &(e, _, _)) in pairs.iter().enumerate() {
                    match groups.last_mut() {
                        Some(g) if g.0 == e => g.2 += 1,
                        _ => groups.push((e, i, 1)),
                    }
                }
                let mut gu = vec![0.0f32; pairs.len() * 2 * ei];
                rows_for_groups(gate_up, &groups, 2 * ei, xs, &pairs, &mut gu, pool);
                let hidden: Vec<Prepared> = gu
                    .chunks_exact(2 * ei)
                    .map(|p| {
                        let act: Vec<f32> = p[..ei].iter().zip(&p[ei..]).map(|(g, u)| math::silu(*g) * u).collect();
                        Prepared::new(&act)
                    })
                    .collect();
                let mut parts = vec![0.0f32; pairs.len() * h];
                rows_for_pairs(down, &groups, h, &hidden, &mut parts, pool);
                out.fill(0.0);
                for (i, &(_, t, wt)) in pairs.iter().enumerate() {
                    quant::axpy(wt, &parts[i * h..(i + 1) * h], &mut out[t * h..(t + 1) * h]);
                }
                let mut shared = vec![0.0f32; n * h];
                swiglu(shared_gate, shared_up, shared_down, xs, &mut shared, pool);
                let sw = shared_weight.times_all(xs, pool);
                for t in 0..n {
                    quant::axpy(math::sigmoid(sw[t]), &shared[t * h..(t + 1) * h], &mut out[t * h..(t + 1) * h]);
                }
            }
        }
    }

    /// The model's chat markers and others, by their text.
    pub fn special(&self, text: &str) -> Result<u32, String> {
        self.tokenizer.special(text).ok_or_else(|| format!("the tokenizer has no {text}"))
    }
}

/// down(SiLU(gate x) * up x), for each token.
fn swiglu(gate: &Matrix, up: &Matrix, down: &Matrix, xs: &[Prepared], out: &mut [f32], pool: &dyn Pool) {
    let (g, u) = (gate.times_all(xs, pool), up.times_all(xs, pool));
    let ready: Vec<Prepared> = g
        .chunks_exact(gate.rows)
        .zip(u.chunks_exact(up.rows))
        .map(|(g, u)| {
            let act: Vec<f32> = g.iter().zip(u).map(|(a, b)| math::silu(*a) * b).collect();
            Prepared::new(&act)
        })
        .collect();
    down.times_into(&ready, out, pool);
}

/// Each token's input normalised (the norm's weights are 1 + w) and made ready for the rows.
fn prepare_norm(x: &[f32], w: &[f32], eps: f32, n: usize, normed: &mut [f32]) -> Vec<Prepared> {
    let h = x.len() / n;
    (0..n)
        .map(|t| {
            rms_norm(&x[t * h..(t + 1) * h], w, true, eps, &mut normed[t * h..(t + 1) * h]);
            Prepared::new(&normed[t * h..(t + 1) * h])
        })
        .collect()
}

/// The experts' gate and up rows (`per` of them each), for the tokens each expert was chosen for: groups are
/// (expert, first pair, pairs); out has `per` numbers for each pair.
fn rows_for_groups(m: &Matrix, groups: &[(usize, usize, usize)], per: usize, xs: &[Prepared], pairs: &[(usize, usize, f32)],
                   out: &mut [f32], pool: &dyn Pool) {
    let inputs: Vec<&Prepared> = pairs.iter().map(|p| &xs[p.1]).collect();
    rows_for_pairs_of(m, groups, per, &inputs, out, pool);
}

/// The experts' down rows (`per` each, the model's width), for each pair's own input.
fn rows_for_pairs(m: &Matrix, groups: &[(usize, usize, usize)], per: usize, xs: &[Prepared], out: &mut [f32], pool: &dyn Pool) {
    let inputs: Vec<&Prepared> = xs.iter().collect();
    rows_for_pairs_of(m, groups, per, &inputs, out, pool);
}

/// Each group's expert's `per` rows times the group's pairs' inputs: a job per expert and block of rows.
fn rows_for_pairs_of(m: &Matrix, groups: &[(usize, usize, usize)], per: usize, inputs: &[&Prepared], out: &mut [f32], pool: &dyn Pool) {
    const ROWS: usize = 64;
    let blocks = per.div_ceil(ROWS);
    let out_ptr = SyncPtr(out.as_mut_ptr());
    pool.run(groups.len() * blocks, &|j| {
        let (e, first, count) = groups[j / blocks];
        let start = (j % blocks) * ROWS;
        let mut row_out = vec![0.0f32; count];
        for r in start..(start + ROWS).min(per) {
            quant::dot_row_many(m.dtype, m.data, m.cols, e * per + r, &inputs[first..first + count], &mut row_out);
            for (k, v) in row_out.iter().enumerate() {
                // (Each job writes its own rows of its own pairs.)
                unsafe { *out_ptr.get().add((first + k) * per + r) = *v };
            }
        }
    });
}

/// A linear-attention layer's memory: the convolution's last inputs, and each value head's state.
#[derive(Clone)]
pub struct Recurrent {
    conv: Vec<f32>,
    state: Vec<f32>,
}

/// An attention layer's keys and values so far (per position, every key-value head's).
pub struct Cache {
    k: Vec<f32>,
    v: Vec<f32>,
}

/// What the model has seen of a conversation: how many tokens (each one's keys and values kept), and the next
/// text's position (the same as how many, until a picture: a picture takes as many positions as its larger side
/// has rows, however many rows it has), with where the picture being read started.
pub struct State {
    pub pos: usize,
    pub rope: i64,
    base: i64,
    linear: Vec<Recurrent>,
    full: Vec<Cache>,
}

/// A state as it was at a position: the linear-attention layers' memories (which only go forward) are copied;
/// the attention layers' keys and values up to there are still in the state, which only adds to them.
pub struct Snapshot {
    pub pos: usize,
    rope: i64,
    base: i64,
    linear: Vec<Recurrent>,
}

impl Snapshot {
    /// Its size in memory.
    pub fn bytes(&self) -> usize {
        self.linear.iter().map(|r| 4 * (r.conv.len() + r.state.len())).sum()
    }
}

impl State {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot { pos: self.pos, rope: self.rope, base: self.base, linear: self.linear.clone() }
    }

    /// Back to a snapshot of this same state, taken at or before where it is now.
    pub fn rewind(&mut self, snap: &Snapshot) {
        assert!(snap.pos <= self.pos && snap.linear.len() == self.linear.len());
        for (r, s) in self.linear.iter_mut().zip(&snap.linear) {
            r.conv.copy_from_slice(&s.conv);
            r.state.copy_from_slice(&s.state);
        }
        for c in &mut self.full {
            let per = c.k.len() / self.pos.max(1);
            c.k.truncate(snap.pos * per);
            c.v.truncate(snap.pos * per);
        }
        self.pos = snap.pos;
        self.rope = snap.rope;
        self.base = snap.base;
    }

    /// The attention layers' keys and values' size in memory.
    pub fn cache_bytes(&self) -> usize {
        self.full.iter().map(|c| 4 * (c.k.len() + c.v.len())).sum()
    }

    /// The state as it was at `snap` (taken from this state), as bytes: the attention layers' keys and values up to
    /// there (this state's own, which only adds to them) and the snapshot's linear-attention memories.
    pub fn export(&self, snap: &Snapshot) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(snap.pos as u64).to_le_bytes());
        let put = |v: &[f32], out: &mut Vec<u8>| {
            for x in v {
                out.extend_from_slice(&x.to_le_bytes());
            }
        };
        for c in &self.full {
            let per = c.k.len() / self.pos.max(1);
            put(&c.k[..snap.pos * per], &mut out);
            put(&c.v[..snap.pos * per], &mut out);
        }
        for r in &snap.linear {
            put(&r.conv, &mut out);
            put(&r.state, &mut out);
        }
        out
    }
}

impl Model<'_> {
    /// A state from what State::export wrote for this same model (None if it doesn't fit it).
    pub fn import(&self, bytes: &[u8]) -> Option<State> {
        let mut st = self.state();
        let pos = u64::from_le_bytes(bytes.get(..8)?.try_into().ok()?) as usize;
        let s = &self.settings;
        let kvd = s.kv_heads * s.head_dim;
        let full = st.full.len();
        let linear: usize = st.linear.iter().map(|r| r.conv.len() + r.state.len()).sum();
        let want = pos.checked_mul(kvd)?.checked_mul(2 * full)?.checked_add(linear)?.checked_mul(4)?.checked_add(8)?;
        if bytes.len() != want || pos > s.context {
            return None;
        }
        let mut at = 8;
        let mut take = |n: usize| -> Vec<f32> {
            let v = bytes[at..at + 4 * n].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
            at += 4 * n;
            v
        };
        for c in st.full.iter_mut() {
            c.k = take(pos * kvd);
            c.v = take(pos * kvd);
        }
        for r in st.linear.iter_mut() {
            r.conv = take(r.conv.len());
            r.state = take(r.state.len());
        }
        st.pos = pos;
        // (What's kept is where a conversation's first message ends, which has no pictures: its positions are its
        // tokens'.)
        st.rope = pos as i64;
        Some(st)
    }
}

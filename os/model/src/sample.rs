//! Choosing the next token from the model's scores: the likeliest (greedy), or at random among the likeliest, as
//! Qwen recommends its models be used (a temperature, the top k, the top p of the probability).

use crate::math;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug)]
pub struct Sampling {
    /// 0: always the likeliest.
    pub temperature: f32,
    pub top_k: usize,
    pub top_p: f32,
    /// Tokens already said have this taken off their scores (0: not at all), against saying the same over and over.
    pub presence_penalty: f32,
}

impl Sampling {
    /// Qwen's recommendation for 3.5's and 3.6's thinking on precise work (code, a computer's tools): their model
    /// cards' "precise coding tasks" settings.
    pub const THINKING: Sampling = Sampling { temperature: 0.6, top_k: 20, top_p: 0.95, presence_penalty: 0.0 };
    /// Qwen's recommendation for 3.8's thinking (its model card, and its generation_config.json).
    pub const THINKING_38: Sampling = Sampling { temperature: 1.0, top_k: 20, top_p: 0.95, presence_penalty: 0.0 };
    /// Qwen's recommendation for answering without thinking ("instruct" mode), 3.5 to 3.8.
    pub const ANSWERING: Sampling = Sampling { temperature: 0.7, top_k: 20, top_p: 0.8, presence_penalty: 1.5 };
    pub const GREEDY: Sampling = Sampling { temperature: 0.0, top_k: 1, top_p: 1.0, presence_penalty: 0.0 };

    /// What the model's makers recommend, for a style (chat.rs) and whether it thinks.
    pub fn recommended(style: &str, think: bool) -> Sampling {
        match (style, think) {
            (_, false) => Sampling::ANSWERING,
            ("qwen3.8", true) => Sampling::THINKING_38,
            _ => Sampling::THINKING,
        }
    }
}

/// Which tokens have been said (for the presence penalty): a bit each.
pub struct Said(Vec<u64>);

impl Said {
    pub fn new(vocab: usize) -> Said {
        Said(alloc::vec![0; vocab.div_ceil(64)])
    }

    pub fn add(&mut self, t: u32) {
        if let Some(w) = self.0.get_mut(t as usize / 64) {
            *w |= 1 << (t % 64);
        }
    }

    pub fn has(&self, t: u32) -> bool {
        self.0.get(t as usize / 64).is_some_and(|w| w & (1 << (t % 64)) != 0)
    }
}

/// A small random number generator (xorshift64*): the same seed, the same choices.
pub struct Random(pub u64);

impl Random {
    pub fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let r = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (r >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// The next token.
pub fn choose(logits: &[f32], said: &Said, s: &Sampling, rng: &mut Random) -> u32 {
    let mut best = 0usize;
    for (i, &v) in logits.iter().enumerate() {
        if v > logits[best] {
            best = i;
        }
    }
    if s.temperature <= 0.0 || s.top_k == 1 {
        return best as u32;
    }
    // The top k by score (after the penalty), then softmax at the temperature, then the top p of it.
    let k = s.top_k.max(1).min(logits.len());
    let mut top: Vec<(f32, u32)> = Vec::with_capacity(k + 1);
    for (i, &v) in logits.iter().enumerate() {
        let v = if s.presence_penalty != 0.0 && said.has(i as u32) { v - s.presence_penalty } else { v };
        if top.len() < k || v > top[top.len() - 1].0 {
            let at = top.partition_point(|&(w, _)| w >= v);
            top.insert(at, (v, i as u32));
            top.truncate(k);
        }
    }
    let max = top[0].0;
    let mut probs: Vec<f32> = top.iter().map(|&(v, _)| math::exp((v - max) / s.temperature)).collect();
    let total: f32 = probs.iter().sum();
    let mut kept = 0;
    let mut acc = 0.0;
    for p in probs.iter_mut() {
        *p /= total;
        acc += *p;
        kept += 1;
        if acc >= s.top_p {
            break;
        }
    }
    let total: f32 = probs[..kept].iter().sum();
    let mut r = rng.next_f32() * total;
    for (i, &p) in probs[..kept].iter().enumerate() {
        if r < p {
            return top[i].1;
        }
        r -= p;
    }
    top[kept - 1].1
}

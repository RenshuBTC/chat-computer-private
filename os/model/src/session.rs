//! A conversation with the model, kept going: the state it has read, reused for the next request as far as that
//! request starts the same way (in a conversation, every request starts with the last one, then its answer), so
//! only what's new is read. The linear-attention layers' memories only go forward, so a few snapshots are kept
//! (where each request's prompt ended) to go back to when a request differs earlier.
//!
//! The model's own answers are put back in as the very tokens it chose (the same words can be split into tokens in
//! more than one way, and reading them again in another would mean reading them again at all).

use crate::chat::{self, Answer, Message, Options, Problem, Reply, Said as Saying, Style};
use crate::json::Json;
use crate::net::{Model, Pool, Snapshot, State};
use crate::sample::{self, Random, Said, Sampling};
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

/// At most this many snapshots are kept (each is the linear-attention layers' memory: tens of megabytes).
const SNAPSHOTS: usize = 4;
/// And this many of the model's answers, to put back as their tokens.
const ANSWERS: usize = 64;

pub struct Session<'m> {
    pub model: &'m Model<'m>,
    pub style: Style,
    state: State,
    /// The tokens the state has read.
    seen: Vec<u32>,
    /// The scores after the last of them.
    logits: Vec<f32>,
    /// With the scores where each was taken.
    snapshots: Vec<(Snapshot, Vec<f32>)>,
    /// The model's recent answers, how it was asked for them (thinking or not), and their tokens.
    answers: Vec<(Answer, bool, Vec<u32>)>,
}

/// How a request went.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    /// Tokens in the prompt, and how many of them had to be read (the rest were already).
    pub prompt: usize,
    pub read: usize,
    pub generated: usize,
    /// Why it ended: the model ended its turn, or ran out of tokens, or was stopped.
    pub ended: Ended,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ended {
    #[default]
    Turn,
    Length,
    Stopped,
}

/// The same answer, as the template writes it (each part trimmed).
fn same_answer(a: &Answer, b: &Answer) -> bool {
    let t = |s: &str| s.trim().to_string();
    t(&a.thinking) == t(&b.thinking) && t(&a.text) == t(&b.text) && a.calls == b.calls
}

/// What to ask.
pub struct Request<'a> {
    pub tools: &'a [Json],
    pub messages: &'a [Message],
    pub options: Options,
    pub sampling: Sampling,
    pub max_tokens: usize,
    pub seed: u64,
}

impl<'m> Session<'m> {
    pub fn new(model: &'m Model<'m>, style: Style) -> Session<'m> {
        Session {
            model,
            style,
            state: model.state(),
            seen: Vec::new(),
            logits: vec![0.0; model.settings.vocab],
            snapshots: Vec::new(),
            answers: Vec::new(),
        }
    }

    /// The prompt for a request, with the model's earlier answers as their own tokens.
    pub fn prompt(&self, r: &Request) -> Result<Vec<u32>, Problem> {
        let known = |a: &Answer| {
            self.answers.iter().rev().find(|(b, _, _)| same_answer(a, b)).map(|(_, think, t)| (*think, t.clone()))
        };
        let p = chat::render_known(self.style, r.tools, r.messages, &r.options, &known)?;
        let mut tokens = Vec::new();
        p.tokens(&self.model.tokenizer, &mut tokens).map_err(|_| Problem::NoMessages)?;
        Ok(tokens)
    }

    /// Reads the prompt (only what wasn't read already), then writes the answer, telling `on` what each token
    /// adds; `on` returning false stops it.
    pub fn ask(&mut self, r: &Request, pool: &dyn Pool, on: &mut dyn FnMut(&Saying, &Answer) -> bool) -> Result<(Answer, Stats), Problem> {
        let think = r.options.think.unwrap_or(self.style.thinks);
        let prompt = self.prompt(r)?;
        let mut stats = Stats { prompt: prompt.len(), ..Stats::default() };
        stats.read = self.read(&prompt, pool);
        // Where the prompt ends: the next request starts the same way, then this answer.
        self.keep_snapshot();
        let tok = &self.model.tokenizer;
        let mut reply = Reply::new(tok, think).map_err(|_| Problem::NoMessages)?;
        let mut said = Said::new(self.model.settings.vocab);
        let mut rng = Random(r.seed | 1);
        let mut generated = Vec::new();
        loop {
            if generated.len() >= r.max_tokens {
                stats.ended = Ended::Length;
                let s = reply.finish();
                on(&s, &reply.answer);
                break;
            }
            let next = sample::choose(&self.logits, &said, &r.sampling, &mut rng);
            let s = reply.push(tok, next);
            if reply.done {
                // (The turn's end isn't read: the next request has it, then what follows.)
                on(&s, &reply.answer);
                break;
            }
            said.add(next);
            generated.push(next);
            if !on(&s, &reply.answer) {
                stats.ended = Ended::Stopped;
                let s = reply.finish();
                on(&s, &reply.answer);
                break;
            }
            self.model.forward(&mut self.state, &[next], Some(&mut self.logits), pool);
            self.seen.push(next);
        }
        stats.generated = generated.len();
        let answer = reply.answer.clone();
        // (However it ended: what was cut short comes back as it was cut, and its tokens are these.)
        if self.answers.len() >= ANSWERS {
            self.answers.remove(0);
        }
        self.answers.push((answer.clone(), think, generated));
        Ok((answer, stats))
    }

    /// Brings the state to having read `tokens`, reusing what it has read as far as that's the same; how many
    /// tokens it read.
    pub fn read(&mut self, tokens: &[u32], pool: &dyn Pool) -> usize {
        let same = self.seen.iter().zip(tokens).take_while(|(a, b)| a == b).count();
        if tokens.is_empty() {
            return 0;
        }
        if same == tokens.len() {
            if same == self.seen.len() {
                return 0;
            }
            // A prompt read before (asked again): back to where it ended, scores and all.
            if let Some(i) = self.snapshots.iter().rposition(|(s, _)| s.pos == same) {
                self.back_to(i);
                return 0;
            }
        }
        if same < self.seen.len() || same == tokens.len() {
            // Back to the last snapshot at or before where they part (at least one token before the end: the
            // scores after the last are needed), or to the start.
            let limit = same.min(tokens.len() - 1);
            match self.snapshots.iter().rposition(|(s, _)| s.pos <= limit) {
                Some(i) => self.back_to(i),
                None => {
                    self.state = self.model.state();
                    self.snapshots.clear();
                    self.seen.clear();
                }
            }
        }
        let from = self.seen.len();
        self.model.forward(&mut self.state, &tokens[from..], Some(&mut self.logits), pool);
        self.seen.extend_from_slice(&tokens[from..]);
        tokens.len() - from
    }

    /// Back to snapshot i (those after it no longer apply).
    fn back_to(&mut self, i: usize) {
        self.snapshots.truncate(i + 1);
        let (snap, logits) = &self.snapshots[i];
        self.state.rewind(snap);
        self.logits.copy_from_slice(logits);
        self.seen.truncate(snap.pos);
    }

    fn keep_snapshot(&mut self) {
        if self.snapshots.last().is_some_and(|(s, _)| s.pos == self.state.pos) {
            return;
        }
        if self.snapshots.len() >= SNAPSHOTS {
            self.snapshots.remove(0);
        }
        self.snapshots.push((self.state.snapshot(), self.logits.clone()));
    }

    /// The memory the conversation takes (the attention layers' keys and values, and the snapshots).
    pub fn bytes(&self) -> usize {
        self.state.cache_bytes() + self.snapshots.iter().map(|(s, l)| s.bytes() + 4 * l.len()).sum::<usize>()
    }

    /// Forgets everything read.
    pub fn clear(&mut self) {
        self.state = self.model.state();
        self.seen.clear();
        self.snapshots.clear();
        self.answers.clear();
    }
}

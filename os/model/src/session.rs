//! A conversation with the model, kept going: the state it has read, reused for the next request as far as that
//! request starts the same way (in a conversation, every request starts with the last one, then its answer), so
//! only what's new is read. The linear-attention layers' memories only go forward, so a few snapshots are kept
//! (where each request's prompt ended) to go back to when a request differs earlier.
//!
//! The model's own answers are put back in as the very tokens it chose (the same words can be split into tokens in
//! more than one way, and reading them again in another would mean reading them again at all).
//!
//! A request is answered all at once (`ask`), or a step at a time (`begin`, `step` until the turn is over, `end`):
//! a part of the prompt read, or one token written, per step, for a caller with other things to do between them
//! (Chat Computer's own kernel, whose screen and keyboard go on while the model reads and writes). Such a caller can
//! also read the system prompt ahead of the first question (`ahead`, `read_ahead`), while the person is still typing.

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
    /// With the scores where each was taken (none for the anchor), and whether it's kept for good.
    snapshots: Vec<Kept>,
    /// The model's recent answers, how it was asked for them (thinking or not), and their tokens.
    answers: Vec<(Answer, bool, Vec<u32>)>,
}

/// A snapshot kept, with the scores where it was taken (empty when the reading goes on past it), and whether it's
/// the anchor: where the system prompt ends, which every conversation starts with, kept for good (and on disk).
struct Kept {
    snap: Snapshot,
    logits: Vec<f32>,
    anchor: bool,
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

/// A turn being answered a step at a time.
pub struct Turn {
    prompt: Vec<u32>,
    anchor: Option<usize>,
    /// Still reading the prompt (the state has read what `seen` says).
    reading: bool,
    think: bool,
    reply: Reply,
    said: Said,
    rng: Random,
    sampling: Sampling,
    max_tokens: usize,
    generated: Vec<u32>,
    stats: Stats,
    finished: bool,
}

impl Turn {
    /// What it has said so far.
    pub fn answer(&self) -> &Answer {
        &self.reply.answer
    }

    /// Still reading the prompt.
    pub fn reading(&self) -> bool {
        self.reading
    }

    /// Over: the model ended its turn, ran out of tokens, or was stopped.
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// How many tokens its prompt is.
    pub fn prompt_tokens(&self) -> usize {
        self.prompt.len()
    }
}

/// What a step did.
#[derive(Debug)]
pub enum Step {
    /// Read more of the prompt: `done` of its `total` tokens are read now.
    Read { done: usize, total: usize },
    /// Wrote a token: what it adds (`Said::Done` at the turn's end).
    Said(Saying),
    /// Nothing: the turn is over.
    Over,
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
        let mut t = self.begin(r)?;
        while t.reading {
            self.step(&mut t, usize::MAX, pool);
        }
        loop {
            match self.step(&mut t, 0, pool) {
                Step::Said(s) => {
                    if t.finished {
                        on(&s, &t.reply.answer);
                        break;
                    }
                    if !on(&s, &t.reply.answer) {
                        let s = self.stop(&mut t);
                        on(&s, &t.reply.answer);
                        break;
                    }
                }
                Step::Over => break,
                Step::Read { .. } => {}
            }
        }
        Ok(self.end(t))
    }

    /// Starts answering a request: its prompt made, the state back to where it last read the same; nothing read.
    pub fn begin(&mut self, r: &Request) -> Result<Turn, Problem> {
        let think = r.options.think.unwrap_or(self.style.thinks);
        let prompt = self.prompt(r)?;
        let reply = Reply::new(&self.model.tokenizer, think).map_err(|_| Problem::NoMessages)?;
        // The anchor: where the first message (the system prompt, with the tools) ends.
        let end = self.model.tokenizer.special("<|im_end|>");
        let anchor = prompt.iter().position(|&t| Some(t) == end).map(|i| i + 1);
        let from = self.rewind_for(&prompt);
        let stats = Stats { prompt: prompt.len(), read: from.map_or(0, |f| prompt.len() - f), ..Stats::default() };
        if from.is_none() {
            // Where the prompt ends: the next request starts the same way, then this answer.
            self.keep_snapshot();
        }
        Ok(Turn {
            prompt,
            anchor,
            reading: from.is_some(),
            think,
            reply,
            said: Said::new(self.model.settings.vocab),
            rng: Random(r.seed | 1),
            sampling: r.sampling,
            max_tokens: r.max_tokens,
            generated: Vec::new(),
            stats,
            finished: false,
        })
    }

    /// One step: up to `read` more of the prompt's tokens read (at least one), or, once it's all read, one token
    /// written.
    pub fn step(&mut self, t: &mut Turn, read: usize, pool: &dyn Pool) -> Step {
        if t.finished {
            return Step::Over;
        }
        if t.reading {
            let to = self.seen.len().saturating_add(read.max(1)).min(t.prompt.len());
            self.read_to(&t.prompt, to, t.anchor, pool);
            if to == t.prompt.len() {
                t.reading = false;
                self.keep_snapshot();
            }
            return Step::Read { done: to, total: t.prompt.len() };
        }
        if t.generated.len() >= t.max_tokens {
            t.stats.ended = Ended::Length;
            t.finished = true;
            return Step::Said(t.reply.finish());
        }
        let next = sample::choose(&self.logits, &t.said, &t.sampling, &mut t.rng);
        let s = t.reply.push(&self.model.tokenizer, next);
        if t.reply.done {
            // (The turn's end isn't read: the next request has it, then what follows.)
            t.finished = true;
            return Step::Said(s);
        }
        t.said.add(next);
        t.generated.push(next);
        self.model.forward(&mut self.state, &[next], Some(&mut self.logits), pool);
        self.seen.push(next);
        Step::Said(s)
    }

    /// Stops the turn where it is: what it was saying, finished.
    pub fn stop(&mut self, t: &mut Turn) -> Saying {
        if t.finished {
            return Saying::Nothing;
        }
        t.stats.ended = Ended::Stopped;
        t.finished = true;
        if t.reading {
            // (Stopped before a word: the prompt read so far stays read.)
            t.reading = false;
            return Saying::Done;
        }
        t.reply.finish()
    }

    /// The turn's answer and how it went; its tokens kept, to put back as they were.
    pub fn end(&mut self, t: Turn) -> (Answer, Stats) {
        let mut stats = t.stats;
        stats.generated = t.generated.len();
        let answer = t.reply.answer.clone();
        // (However it ended: what was cut short comes back as it was cut, and its tokens are these.)
        if self.answers.len() >= ANSWERS {
            self.answers.remove(0);
        }
        self.answers.push((answer.clone(), t.think, t.generated));
        (answer, stats)
    }

    /// The tokens of a request's first message (the system prompt, with the tools), to read ahead of the question
    /// (`read_ahead`); none if the prompt has no end to its first message.
    pub fn ahead(&self, r: &Request) -> Result<Vec<u32>, Problem> {
        let mut prompt = self.prompt(r)?;
        let end = self.model.tokenizer.special("<|im_end|>");
        let anchor = prompt.iter().position(|&t| Some(t) == end).map_or(0, |i| i + 1);
        prompt.truncate(anchor);
        Ok(prompt)
    }

    /// Reads a request's first message ahead of the question (`ahead`'s tokens), up to `read` more of them a call
    /// (at least one), for a caller with other things to do between parts; true once it's all read, kept for good
    /// where it ends (the anchor), so every request that starts the same way reads only what comes after it.
    pub fn read_ahead(&mut self, tokens: &[u32], read: usize, pool: &dyn Pool) -> bool {
        if tokens.is_empty() {
            return true;
        }
        let same = self.seen.iter().zip(tokens).take_while(|(a, b)| a == b).count();
        if same == tokens.len() && self.snapshots.iter().any(|k| k.anchor && k.snap.pos == tokens.len()) {
            return true;
        }
        if same < self.seen.len() {
            match self.snapshots.iter().rposition(|k| k.snap.pos <= same) {
                Some(i) => self.back_to(i),
                None => {
                    self.state = self.model.state();
                    self.snapshots.clear();
                    self.seen.clear();
                }
            }
        }
        let from = self.seen.len();
        let to = from.saturating_add(read.max(1)).min(tokens.len());
        if from < to {
            self.model.forward(&mut self.state, &tokens[from..to], None, pool);
            self.seen.extend_from_slice(&tokens[from..to]);
        }
        if to < tokens.len() {
            return false;
        }
        self.snapshots.retain(|k| !k.anchor && k.snap.pos <= to);
        self.snapshots.push(Kept { snap: self.state.snapshot(), logits: Vec::new(), anchor: true });
        true
    }

    /// Brings the state to having read `tokens`, reusing what it has read as far as that's the same; how many
    /// tokens it read.
    pub fn read(&mut self, tokens: &[u32], pool: &dyn Pool) -> usize {
        self.read_anchored(tokens, None, pool)
    }

    /// The same, with a snapshot kept for good where `anchor` is (if the reading passes it).
    pub fn read_anchored(&mut self, tokens: &[u32], anchor: Option<usize>, pool: &dyn Pool) -> usize {
        let Some(from) = self.rewind_for(tokens) else { return 0 };
        self.read_to(tokens, tokens.len(), anchor, pool);
        tokens.len() - from
    }

    /// Gets the state ready to read `tokens`: back to where it last read the same, as far as that goes. Where the
    /// reading goes on from; None when there's nothing to read (`tokens` were read, scores and all).
    fn rewind_for(&mut self, tokens: &[u32]) -> Option<usize> {
        let same = self.seen.iter().zip(tokens).take_while(|(a, b)| a == b).count();
        if tokens.is_empty() {
            return None;
        }
        if same == tokens.len() {
            if same == self.seen.len() {
                return None;
            }
            // A prompt read before (asked again): back to where it ended, scores and all.
            if let Some(i) = self.snapshots.iter().rposition(|k| k.snap.pos == same && !k.logits.is_empty()) {
                self.back_to(i);
                return None;
            }
        }
        if same < self.seen.len() || same == tokens.len() {
            // Back to the last snapshot at or before where they part (at least one token before the end: the
            // scores after the last are needed), or to the start.
            let limit = same.min(tokens.len() - 1);
            match self.snapshots.iter().rposition(|k| k.snap.pos <= limit) {
                Some(i) => self.back_to(i),
                None => {
                    self.state = self.model.state();
                    self.snapshots.clear();
                    self.seen.clear();
                }
            }
        }
        Some(self.seen.len())
    }

    /// Reads `tokens` on from where the state is, up to `to` (the scores after the last only when that's the
    /// end), keeping a snapshot for good where `anchor` is if the reading passes it.
    fn read_to(&mut self, tokens: &[u32], to: usize, anchor: Option<usize>, pool: &dyn Pool) {
        let from = self.seen.len();
        if let Some(a) = anchor.filter(|&a| from < a && a < tokens.len() && a <= to) {
            self.model.forward(&mut self.state, &tokens[from..a], None, pool);
            self.seen.extend_from_slice(&tokens[from..a]);
            self.snapshots.retain(|k| !k.anchor);
            self.snapshots.push(Kept { snap: self.state.snapshot(), logits: Vec::new(), anchor: true });
        }
        let at = self.seen.len();
        if at < to {
            let last = to == tokens.len();
            self.model.forward(&mut self.state, &tokens[at..to], if last { Some(&mut self.logits) } else { None }, pool);
            self.seen.extend_from_slice(&tokens[at..to]);
        }
    }

    /// Back to snapshot i (those after it no longer apply).
    fn back_to(&mut self, i: usize) {
        self.snapshots.truncate(i + 1);
        let k = &self.snapshots[i];
        self.state.rewind(&k.snap);
        if !k.logits.is_empty() {
            self.logits.copy_from_slice(&k.logits);
        }
        self.seen.truncate(k.snap.pos);
    }

    fn keep_snapshot(&mut self) {
        if self.snapshots.last().is_some_and(|k| k.snap.pos == self.state.pos && !k.logits.is_empty()) {
            return;
        }
        if self.snapshots.iter().filter(|k| !k.anchor).count() >= SNAPSHOTS
            && let Some(oldest) = self.snapshots.iter().position(|k| !k.anchor)
        {
            self.snapshots.remove(oldest);
        }
        self.snapshots.push(Kept { snap: self.state.snapshot(), logits: self.logits.clone(), anchor: false });
    }

    /// The anchor, to be kept on disk: the tokens up to it, and the state there (its keys and values are the
    /// state's own up to there).
    pub fn anchor(&self) -> Option<(&[u32], &State, &Snapshot)> {
        let k = self.snapshots.iter().find(|k| k.anchor)?;
        Some((&self.seen[..k.snap.pos], &self.state, &k.snap))
    }

    /// Starts from an anchor kept on disk: as if `tokens` had been read (state is the state there).
    pub fn restore(&mut self, tokens: Vec<u32>, state: State) {
        let snap = state.snapshot();
        self.state = state;
        self.seen = tokens;
        self.snapshots = vec![Kept { snap, logits: Vec::new(), anchor: true }];
    }

    /// The memory the conversation takes (the attention layers' keys and values, and the snapshots).
    pub fn bytes(&self) -> usize {
        self.state.cache_bytes() + self.snapshots.iter().map(|k| k.snap.bytes() + 4 * k.logits.len()).sum::<usize>()
    }

    /// Forgets everything read.
    pub fn clear(&mut self) {
        self.state = self.model.state();
        self.seen.clear();
        self.snapshots.clear();
        self.answers.clear();
    }
}

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
//!
//! Pictures in a request are each given a slot (by their mark: the same picture, the same slot), their rows read as
//! tokens PICTURE | slot << 16 | row, so a request is the same as an earlier one as far as its pictures are too. A
//! picture's rows are made (the vision part) when the reading gets to them, and kept for the most recent pictures; a
//! picture no request has any more (its conversation over, or its oldest part left out) is let go.

use crate::chat::{self, Answer, Message, Options, Picture, Problem, Reply, Said as Saying, Style};
use crate::json::Json;
use crate::net::{self, Model, Pool, Rows, Snapshot, State};
use alloc::sync::Arc;
use crate::sample::{self, Random, Said, Sampling};
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

/// At most this many snapshots are kept (each is the linear-attention layers' memory: tens of megabytes).
const SNAPSHOTS: usize = 4;
/// And this many of the model's answers, to put back as their tokens.
const ANSWERS: usize = 64;
/// The most rows a picture is read at (unless the caller says fewer), and how many pictures' rows are kept made.
pub const MOST_ROWS: usize = 1024;
const PICTURES_KEPT: usize = 16;

/// A picture the session has been shown: the picture (until no request has it but this: then it's let go, and the
/// same picture given again later gets a slot of its own), the most rows it's read at, its grid of rows, and its rows
/// once they're made.
struct Seen {
    picture: Option<Arc<Picture>>,
    most: usize,
    gh: usize,
    gw: usize,
    rows: Option<Vec<f32>>,
}

/// The pictures' rows, by slot.
struct Pictures<'p>(&'p [Seen]);

impl Rows for Pictures<'_> {
    fn rows(&self, slot: u32) -> Option<(&[f32], usize, usize)> {
        let s = self.0.get(slot as usize)?;
        Some((s.rows.as_deref().unwrap_or(&[]), s.gh, s.gw))
    }
}

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
    /// The pictures it's been shown (a slot each), and the most rows a new one is read at; a picture's rows being
    /// made a step at a time (its slot, and where it's got), and how many patches such a step works on at most (a
    /// caller suits it to how long a step should take; all of a part of a block, unless it says).
    pictures: Vec<Seen>,
    pub most_rows: usize,
    encoding: Option<(u32, crate::vision::Encoding)>,
    pub see: usize,
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

    /// The tokens it has written so far.
    pub fn generated(&self) -> &[u32] {
        &self.generated
    }
}

/// What a step did.
#[derive(Debug)]
pub enum Step {
    /// Read more of the prompt: `done` of its `total` tokens are read now.
    Read { done: usize, total: usize },
    /// Looked at a picture: `done` of the `total` parts of making its rows done (Vision::progress; the reading goes
    /// on after).
    Seeing { done: usize, total: usize },
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
            pictures: Vec::new(),
            most_rows: MOST_ROWS,
            encoding: None,
            see: usize::MAX,
        }
    }

    /// The prompt for a request, with the model's earlier answers as their own tokens, and its pictures' rows as
    /// their slots' (each picture given a slot, if it hasn't one).
    pub fn prompt(&mut self, r: &Request) -> Result<Vec<u32>, Problem> {
        let known = |a: &Answer| {
            self.answers.iter().rev().find(|(b, _, _)| same_answer(a, b)).map(|(_, think, t)| (*think, t.clone()))
        };
        let p = chat::render_known(self.style, r.tools, r.messages, &r.options, &known)?;
        if self.model.vision.is_none() && p.parts.iter().any(|part| matches!(part, chat::Part::Picture(_))) {
            return Err(Problem::Blind);
        }
        // (The pictures no request has any more but this one, let go: their pixels and their rows.)
        for s in self.pictures.iter_mut() {
            if s.picture.as_ref().is_some_and(|p| Arc::strong_count(p) == 1) {
                s.picture = None;
                s.rows = None;
            }
        }
        let mut tokens = Vec::new();
        let (model, most) = (self.model, self.most_rows);
        let pictures = &mut self.pictures;
        let mut blind = false;
        let made = p.tokens_seeing(&model.tokenizer, &mut tokens, &mut |pic, out| {
            let Some(vision) = &model.vision else {
                blind = true;
                return Err("blind");
            };
            let same = |s: &Seen| s.most == most && s.picture.as_ref().is_some_and(|p| p.mark == pic.mark && **p == **pic);
            let slot = match pictures.iter().position(same) {
                Some(slot) => slot,
                None => {
                    if pictures.len() >= 0x7FFF {
                        return Err("too many pictures");
                    }
                    let (h, w) = vision.size_for(pic.height, pic.width, most);
                    let (gh, gw) = vision.rows_for(h, w);
                    pictures.push(Seen { picture: Some(pic.clone()), most, gh, gw, rows: None });
                    pictures.len() - 1
                }
            };
            let s = &pictures[slot];
            out.extend((0..(s.gh * s.gw) as u32).map(|row| net::picture_token(slot as u32, row)));
            Ok(())
        });
        if blind {
            return Err(Problem::Blind);
        }
        made.map_err(|_| Problem::NoMessages)?;
        Ok(tokens)
    }

    /// The rows of every picture among `tokens` made, if they aren't (the vision part, a picture at a time: this is
    /// what takes long for a picture); the oldest pictures' rows let go beyond the most kept.
    fn see(&mut self, tokens: &[u32], pool: &dyn Pool) {
        let Some(vision) = &self.model.vision else { return };
        let mut slots: Vec<u32> = tokens.iter().filter_map(|&t| net::picture_of(t).map(|(s, _)| s)).collect();
        slots.dedup();
        for slot in slots {
            let Some(s) = self.pictures.get_mut(slot as usize) else { continue };
            let Some(picture) = &s.picture else { continue };
            if s.rows.is_some() {
                continue;
            }
            let sized = vision.prepare(&picture.rgb, picture.width, picture.height, s.most);
            let patches = vision.patches(&sized);
            let p = vision.settings.patch;
            s.rows = Some(vision.encode(&patches, sized.height / p, sized.width / p, pool));
            self.keep_rows(slot);
        }
    }

    /// Only the most recent pictures' rows kept (`slot`'s just made): a picture seen again is made again.
    fn keep_rows(&mut self, slot: u32) {
        let made: Vec<usize> = (0..self.pictures.len()).filter(|&i| self.pictures[i].rows.is_some()).collect();
        if made.len() > PICTURES_KEPT {
            for &i in made.iter().filter(|&&i| i != slot as usize).take(made.len() - PICTURES_KEPT) {
                self.pictures[i].rows = None;
            }
        }
    }

    /// A step of making the rows of the first picture among `tokens` that has none (begun, if it isn't: resized and
    /// cut into patches; then a part of the vision part's work, for up to `see` patches): how far it's got, of how
    /// far it goes; None if every picture there has its rows.
    fn see_a_step(&mut self, tokens: &[u32], pool: &dyn Pool) -> Option<(usize, usize)> {
        let vision = self.model.vision.as_ref()?;
        let slot = tokens.iter().find_map(|&t| {
            net::picture_of(t)
                .map(|(s, _)| s)
                .filter(|&s| self.pictures.get(s as usize).is_some_and(|p| p.rows.is_none() && p.picture.is_some()))
        })?;
        if !matches!(&self.encoding, Some((s, _)) if *s == slot) {
            let p = &self.pictures[slot as usize];
            let picture = p.picture.as_ref()?;
            let sized = vision.prepare(&picture.rgb, picture.width, picture.height, p.most);
            let patches = vision.patches(&sized);
            let ps = vision.settings.patch;
            let e = vision.begin(patches, sized.height / ps, sized.width / ps);
            let (done, total) = vision.progress(&e);
            self.encoding = Some((slot, e));
            return Some((done, total));
        }
        let (_, e) = self.encoding.as_mut()?;
        match vision.advance(e, self.see, pool) {
            Some(rows) => {
                let (_, total) = vision.progress(e);
                self.pictures[slot as usize].rows = Some(rows);
                self.encoding = None;
                self.keep_rows(slot);
                Some((total, total))
            }
            None => Some(vision.progress(e)),
        }
    }

    /// The model reads `tokens` (pictures' rows among them), with the scores after the last if `logits`.
    fn forward(&mut self, tokens: &[u32], logits: bool, pool: &dyn Pool) {
        self.see(tokens, pool);
        let rows = Pictures(&self.pictures);
        let scores = if logits { Some(&mut self.logits[..]) } else { None };
        self.model.forward_seeing(&mut self.state, tokens, &rows, scores, pool);
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
                Step::Read { .. } | Step::Seeing { .. } => {}
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
            // (A picture among what's next, its rows not made: a step of making them first.)
            let next = t.prompt[self.seen.len().min(to)..to].to_vec();
            if let Some((done, total)) = self.see_a_step(&next, pool) {
                return Step::Seeing { done, total };
            }
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
        self.forward(&[next], true, pool);
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
        // (The first message has no pictures: those after it are left out, as everything after it is.)
        let p = chat::render_known(self.style, r.tools, r.messages, &r.options, &|_| None)?;
        let mut prompt = Vec::new();
        p.tokens_seeing(&self.model.tokenizer, &mut prompt, &mut |_, _| Ok(())).map_err(|_| Problem::NoMessages)?;
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
            self.forward(&tokens[from..to], false, pool);
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
            self.forward(&tokens[from..a], false, pool);
            self.seen.extend_from_slice(&tokens[from..a]);
            self.snapshots.retain(|k| !k.anchor);
            self.snapshots.push(Kept { snap: self.state.snapshot(), logits: Vec::new(), anchor: true });
        }
        let at = self.seen.len();
        if at < to {
            let last = to == tokens.len();
            self.forward(&tokens[at..to], last, pool);
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

    /// The scores after the last token read.
    pub fn scores(&self) -> &[f32] {
        &self.logits
    }

    /// How many pictures' pixels it holds: those a request it was given still has.
    pub fn pictures_held(&self) -> usize {
        self.pictures.iter().filter(|s| s.picture.is_some()).count()
    }

    /// Forgets everything read.
    pub fn clear(&mut self) {
        self.state = self.model.state();
        self.seen.clear();
        self.snapshots.clear();
        self.answers.clear();
        self.pictures.clear();
        self.encoding = None;
    }
}

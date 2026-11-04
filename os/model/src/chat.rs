//! A conversation as the model reads it, and what it says back: its thinking, its words and its tool calls.
//!
//! Each published model comes with its chat template, a small program in Jinja that writes the conversation out;
//! this is the same program written in Rust, so nothing else is needed to run one. The models' templates differ in
//! a few switches (Style), found in the template when a model is converted (convert.rs) and kept in its file:
//!
//! - Qwen 3.5 0.8B and 2B answer straight away unless asked to think; 3.5 4B and 9B, 3.6 and 3.8 think first;
//! - 3.8 keeps the thinking of every earlier answer, the others only of those since the person last wrote;
//! - 3.8 is told in the system prompt how hard to think, and writes no system prompt that would be empty.
//!
//! One thing is done on purpose differently from the templates: the model's special tokens (<|im_start|>,
//! <tool_call> and the rest) come only from the template's own writing. Text that a person, a web page, a file or a
//! tool brings is read as text even when it spells out a special token, so it can't end a turn and start a system
//! prompt of its own. (Run through the Jinja template and a tokenizer, it would.)
//!
//! A message from the person, or a tool's result, may have pictures in it (`Content`), written as the templates
//! write them: <|vision_start|><|image_pad|><|vision_end|>, the pad becoming the picture's rows (session.rs).

use crate::json::{self, Json};
use crate::token::Tokenizer;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

/// A picture for the model to see: its pixels (RGB, a byte a colour, row after row), and a mark of them (FNV-1a over
/// its size and pixels), by which it's known again.
#[derive(Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
    pub mark: u64,
}

impl Picture {
    pub fn new(width: usize, height: usize, rgb: Vec<u8>) -> Picture {
        let size = [(width as u64).to_le_bytes(), (height as u64).to_le_bytes()].concat();
        let mark = crate::file::fnv(crate::file::fnv(0xcbf2_9ce4_8422_2325, &size), &rgb);
        Picture { width, height, rgb, mark }
    }
}

/// A part of a message: words, or a picture.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Text(String),
    Picture(Arc<Picture>),
}

/// How a model family's template writes a conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    /// Thinks before answering unless asked not to.
    pub thinks: bool,
    /// Every earlier answer keeps its thinking (otherwise only those since the person's last message do).
    pub keeps_thinking: bool,
    /// The system prompt says how hard to think.
    pub effort: bool,
    /// A system message with nothing in it is still written out.
    pub empty_system: bool,
}

impl Style {
    /// Qwen 3.5 0.8B and 2B.
    pub const QWEN35_SMALL: Style = Style { thinks: false, keeps_thinking: false, effort: false, empty_system: true };
    /// Qwen 3.5 4B, 9B and up; 3.6 is the same (it only offers to keep earlier thinking, which Options can ask).
    pub const QWEN35: Style = Style { thinks: true, keeps_thinking: false, effort: false, empty_system: true };
    /// Qwen 3.8.
    pub const QWEN38: Style = Style { thinks: true, keeps_thinking: true, effort: true, empty_system: false };

    /// The style a converted model's file names ("chat.style" in its settings).
    pub fn named(name: &str) -> Option<Style> {
        match name {
            "qwen3.5-small" => Some(Style::QWEN35_SMALL),
            "qwen3.5" => Some(Style::QWEN35),
            "qwen3.8" => Some(Style::QWEN38),
            _ => None,
        }
    }

    /// Which of the known templates this is, by the lines where they differ; None for any other (a model whose
    /// template isn't one of these isn't converted, rather than spoken to in a way it wasn't taught).
    pub fn of_template(t: &str) -> Option<&'static str> {
        let common = ["<tool_call>\\n<function=", "\\n<tool_response>\\n", "'<|im_start|>' + message.role + '\\n'"];
        if !common.iter().all(|c| t.contains(c)) {
            return None;
        }
        let thinks_unless_off = t.contains("{%- if enable_thinking is defined and enable_thinking is false %}");
        let thinks_if_on = t.contains("{%- if enable_thinking is defined and enable_thinking is true %}");
        let keeps = t.contains("preserve_thinking is undefined or preserve_thinking is true");
        let effort = t.contains("Reasoning effort is set to xhigh.");
        match (thinks_unless_off, thinks_if_on, keeps, effort) {
            (false, true, false, false) => Some("qwen3.5-small"),
            (true, false, false, false) => Some("qwen3.5"),
            (true, false, true, true) => Some("qwen3.8"),
            _ => None,
        }
    }
}

/// How hard to think (said only to models whose style has it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effort {
    /// "xhigh", the models' own choice.
    Most,
    Medium,
    Low,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Think first (None: as the model does unless told).
    pub think: Option<bool>,
    /// Keep earlier answers' thinking (None: as the model's template does).
    pub keep_thinking: Option<bool>,
    pub effort: Effort,
}

impl Default for Options {
    fn default() -> Options {
        Options { think: None, keep_thinking: None, effort: Effort::Most }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    System(String),
    User(String),
    Assistant(Answer),
    /// What a tool said back.
    Tool(String),
    /// From the person, with pictures among the words.
    UserParts(Vec<Content>),
    /// A tool's result with pictures (one that shows the model a picture).
    ToolParts(Vec<Content>),
}

impl Message {
    fn is_tool(&self) -> bool {
        matches!(self, Message::Tool(_) | Message::ToolParts(_))
    }
}

/// What the model said in one turn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Answer {
    pub thinking: String,
    pub text: String,
    pub calls: Vec<Call>,
}

/// A tool the model calls, and what it gives it: each argument as the model wrote it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Call {
    pub name: String,
    pub args: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    NoMessages,
    /// Nothing from the person to answer (only tool results).
    NoQuestion,
    /// A system message after the first.
    SystemNotFirst,
    /// A tool's result before anything that called it.
    ToolFirst,
    /// A picture, for a model that can't see (its file has no vision part).
    Blind,
}

/// The model's special tokens the templates write.
const SPECIALS: [&str; 8] = [
    "<|im_start|>",
    "<|im_end|>",
    "<think>",
    "</think>",
    "<tool_call>",
    "</tool_call>",
    "<tool_response>",
    "</tool_response>",
];

/// Said when the model may call tools (the templates' own words).
const TOOLS_HEAD: &str = "# Tools\n\nYou have access to the following functions:\n\n<tools>";
const TOOLS_RULES: &str = "\n\nIf you choose to call a function ONLY reply in the following format with NO suffix:\n\n<tool_call>\n<function=example_function_name>\n<parameter=example_parameter_1>\nvalue_1\n</parameter>\n<parameter=example_parameter_2>\nThis is the value for the second parameter\nthat can span\nmultiple lines\n</parameter>\n</function>\n</tool_call>\n\n<IMPORTANT>\nReminder:\n- Function calls MUST follow the specified format: an inner <function=...></function> block must be nested within <tool_call></tool_call> XML tags\n- Required parameters MUST be specified\n- You may provide optional reasoning for your function call in natural language BEFORE the function call, but NOT after\n- If there is no function call available, answer the question like normal with your current knowledge and do not tell the user about function calls\n</IMPORTANT>";
const EFFORT_MOST: &str = "Reasoning effort is set to xhigh. Please think carefully through the task, validate key assumptions, consider plausible alternatives, and prioritize correctness, consistency, and clarity in the final answer.";
const EFFORT_LOW: &str = "Reasoning effort is set to low. Keep your thinking brief and focused, moving directly to the conclusion without unnecessary elaboration.";

/// A conversation written out: the template's special tokens, and text between them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prompt {
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Special(&'static str),
    Text(String),
    /// Tokens as they are: one of the model's own answers, as it chose them.
    Tokens(Vec<u32>),
    /// A picture, between <|vision_start|> and <|vision_end|>: its rows.
    Picture(Arc<Picture>),
}

impl Prompt {
    /// The template's own writing: its special tokens are special.
    fn markup(&mut self, mut s: &str) {
        while !s.is_empty() {
            let next = SPECIALS.iter().filter_map(|t| s.find(t).map(|at| (at, *t))).min_by_key(|&(at, _)| at);
            match next {
                Some((at, t)) => {
                    self.text(&s[..at]);
                    self.parts.push(Part::Special(t));
                    s = &s[at + t.len()..];
                }
                None => {
                    self.text(s);
                    return;
                }
            }
        }
    }

    /// Anything else: only ever text.
    fn text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        if let Some(Part::Text(t)) = self.parts.last_mut() {
            t.push_str(s);
        } else {
            self.parts.push(Part::Text(s.to_string()));
        }
    }

    /// As one string (the same as the model's Jinja template writes).
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for p in &self.parts {
            match p {
                Part::Special(s) => out.push_str(s),
                Part::Text(t) => out.push_str(t),
                Part::Tokens(_) => out.push_str("(tokens)"),
                Part::Picture(_) => out.push_str("<|image_pad|>"),
            }
        }
        out
    }

    /// As tokens: the text between special tokens is read as a tokenizer reads text between them. (A picture
    /// can't be: see tokens_seeing.)
    pub fn tokens(&self, tok: &Tokenizer, out: &mut Vec<u32>) -> Result<(), &'static str> {
        self.tokens_seeing(tok, out, &mut |_, _| Err("a picture, with nothing to see it"))
    }

    /// As tokens, each picture's by `picture` (which adds them to the list).
    pub fn tokens_seeing(&self, tok: &Tokenizer, out: &mut Vec<u32>,
                         picture: &mut dyn FnMut(&Arc<Picture>, &mut Vec<u32>) -> Result<(), &'static str>) -> Result<(), &'static str> {
        for p in &self.parts {
            match p {
                Part::Special(s) => out.push(tok.special(s).ok_or(*s)?),
                Part::Text(t) => tok.encode(t, out),
                Part::Tokens(t) => out.extend_from_slice(t),
                Part::Picture(pic) => picture(pic, out)?,
            }
        }
        Ok(())
    }

    /// A message's parts as the template writes them (its pictures between their markers), the whole stripped of
    /// white space at its ends as the template strips it.
    fn content(&mut self, parts: &[Content]) {
        let first = parts.iter().position(|c| !matches!(c, Content::Text(t) if strip(t).is_empty()));
        let last = parts.iter().rposition(|c| !matches!(c, Content::Text(t) if strip(t).is_empty()));
        let (Some(first), Some(last)) = (first, last) else { return };
        for (i, c) in parts.iter().enumerate().take(last + 1).skip(first) {
            match c {
                Content::Text(t) => {
                    let mut t = t.as_str();
                    if i == first {
                        t = t.trim_start_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c));
                    }
                    if i == last {
                        t = t.trim_end_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c));
                    }
                    self.text(t);
                }
                Content::Picture(p) => {
                    self.parts.push(Part::Special("<|vision_start|>"));
                    self.parts.push(Part::Picture(p.clone()));
                    self.parts.push(Part::Special("<|vision_end|>"));
                }
            }
        }
    }
}

/// Whitespace as Python's str.strip() takes it (Rust's, and the four separator controls).
fn strip(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

/// The conversation as the model reads it, up to where it starts its answer.
pub fn render(style: Style, tools: &[Json], messages: &[Message], o: &Options) -> Result<Prompt, Problem> {
    render_known(style, tools, messages, o, &|_| None)
}

/// The model's own answer, as it chose its tokens: whether it was asked to think, and the tokens it wrote after the
/// prompt (up to its turn's end).
pub type Known<'a> = dyn Fn(&Answer) -> Option<(bool, Vec<u32>)> + 'a;

/// The same, with the model's own earlier answers (those `known` has) as the tokens it chose, where the turn is
/// written as it was asked for (with its thinking).
pub fn render_known(style: Style, tools: &[Json], messages: &[Message], o: &Options, known: &Known) -> Result<Prompt, Problem> {
    let first = messages.first().ok_or(Problem::NoMessages)?;
    let think = o.think.unwrap_or(style.thinks);
    let keep = o.keep_thinking.unwrap_or(style.keeps_thinking);
    let effort = match (style.effort && think, o.effort) {
        (true, Effort::Most) => Some(EFFORT_MOST),
        (true, Effort::Low) => Some(EFFORT_LOW),
        _ => None,
    };
    let system = match first {
        Message::System(s) => Some(strip(s)),
        _ => None,
    };
    let mut p = Prompt::default();
    if !tools.is_empty() {
        p.markup("<|im_start|>system\n");
        if let Some(e) = effort {
            p.markup(e);
            p.markup("\n\n");
        }
        p.markup(TOOLS_HEAD);
        for t in tools {
            p.markup("\n");
            p.text(&json::dumps(t));
        }
        p.markup("\n</tools>");
        p.markup(TOOLS_RULES);
        if let Some(s) = system.filter(|s| !s.is_empty()) {
            p.markup("\n\n");
            p.text(s);
        }
        p.markup("<|im_end|>\n");
    } else {
        match (system.filter(|s| !s.is_empty() || style.empty_system), effort) {
            (Some(s), Some(e)) => {
                p.markup("<|im_start|>system\n");
                p.markup(e);
                p.markup("\n\n");
                p.text(s);
                p.markup("<|im_end|>\n");
            }
            (Some(s), None) => {
                p.markup("<|im_start|>system\n");
                p.text(s);
                p.markup("<|im_end|>\n");
            }
            (None, Some(e)) => {
                p.markup("<|im_start|>system\n");
                p.markup(e);
                p.markup("<|im_end|>\n");
            }
            (None, None) => {}
        }
    }
    // The person's last message (tool results come back as the person's too, wrapped in <tool_response>).
    let last_question = messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| match m {
            Message::User(c) => {
                let c = strip(c);
                !(c.starts_with("<tool_response>") && c.ends_with("</tool_response>"))
            }
            Message::UserParts(_) => true,
            _ => false,
        })
        .map(|(i, _)| i)
        .ok_or(Problem::NoQuestion)?;
    for (i, m) in messages.iter().enumerate() {
        match m {
            Message::System(_) if i > 0 => return Err(Problem::SystemNotFirst),
            Message::System(_) => {}
            Message::User(c) => {
                p.markup("<|im_start|>user\n");
                p.text(strip(c));
                p.markup("<|im_end|>\n");
            }
            Message::UserParts(parts) => {
                p.markup("<|im_start|>user\n");
                p.content(parts);
                p.markup("<|im_end|>\n");
            }
            Message::Assistant(a) => {
                let text = strip(&a.text);
                p.markup("<|im_start|>assistant\n");
                let with_thinking = keep || i > last_question;
                if with_thinking
                    && let Some((thought, tokens)) = known(a)
                    && (thought || strip(&a.thinking).is_empty())
                {
                    p.markup(if thought { "<think>\n" } else { "<think>\n\n</think>\n\n" });
                    p.parts.push(Part::Tokens(tokens));
                    p.markup("<|im_end|>\n");
                    continue;
                }
                if with_thinking {
                    p.markup("<think>\n");
                    p.text(strip(&a.thinking));
                    p.markup("\n</think>\n\n");
                }
                p.text(text);
                for (k, c) in a.calls.iter().enumerate() {
                    p.markup(match (k, text.is_empty()) {
                        (0, true) => "",
                        (0, false) => "\n\n",
                        _ => "\n",
                    });
                    p.markup("<tool_call>\n<function=");
                    p.text(&c.name);
                    p.markup(">\n");
                    for (name, value) in &c.args {
                        p.markup("<parameter=");
                        p.text(name);
                        p.markup(">\n");
                        p.text(value);
                        p.markup("\n</parameter>\n");
                    }
                    p.markup("</function>\n</tool_call>");
                }
                p.markup("<|im_end|>\n");
            }
            Message::Tool(_) | Message::ToolParts(_) => {
                if i == 0 {
                    return Err(Problem::ToolFirst);
                }
                if !messages[i - 1].is_tool() {
                    p.markup("<|im_start|>user");
                }
                p.markup("\n<tool_response>\n");
                match m {
                    Message::Tool(c) => p.text(strip(c)),
                    Message::ToolParts(parts) => p.content(parts),
                    _ => {}
                }
                p.markup("\n</tool_response>");
                if !messages.get(i + 1).is_some_and(Message::is_tool) {
                    p.markup("<|im_end|>\n");
                }
            }
        }
    }
    p.markup("<|im_start|>assistant\n");
    p.markup(if think { "<think>\n" } else { "<think>\n\n</think>\n\n" });
    Ok(p)
}

/// What the model is writing, read token by token: first its thinking (if it was asked to think), then its words,
/// then its tool calls.
pub struct Reply {
    pub answer: Answer,
    stage: Stage,
    /// The last token's bytes that aren't a whole character yet.
    held: Vec<u8>,
    /// The tool call being written.
    call: Vec<u8>,
    /// Nothing but whitespace said yet since the thinking ended.
    started: bool,
    ids: Ids,
    pub done: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Thinking,
    Talking,
    Calling,
    /// Between and after tool calls, where nothing but a newline belongs.
    Called,
}

#[derive(Clone, Copy)]
struct Ids {
    think_end: u32,
    call: u32,
    call_end: u32,
    ends: [u32; 2],
}

/// What a token added.
#[derive(Debug, PartialEq)]
pub enum Said {
    Nothing,
    Thinking(String),
    Text(String),
    /// A tool call, finished (its number in answer.calls).
    Call(usize),
    /// The turn's end.
    Done,
}

impl Reply {
    /// For a prompt that opened the model's thinking (`think`) or closed it.
    pub fn new(tok: &Tokenizer, think: bool) -> Result<Reply, &'static str> {
        let id = |s: &'static str| tok.special(s).ok_or(s);
        Ok(Reply {
            answer: Answer::default(),
            stage: if think { Stage::Thinking } else { Stage::Talking },
            held: Vec::new(),
            call: Vec::new(),
            started: false,
            ids: Ids {
                think_end: id("</think>")?,
                call: id("<tool_call>")?,
                call_end: id("</tool_call>")?,
                ends: [id("<|im_end|>")?, id("<|endoftext|>")?],
            },
            done: false,
        })
    }

    /// Whether the model is still thinking.
    pub fn thinking(&self) -> bool {
        self.stage == Stage::Thinking
    }

    pub fn push(&mut self, tok: &Tokenizer, token: u32) -> Said {
        if self.done {
            return Said::Nothing;
        }
        if self.ids.ends.contains(&token) {
            return self.finish();
        }
        match (self.stage, token) {
            (Stage::Thinking, t) if t == self.ids.think_end => {
                self.flush();
                self.stage = Stage::Talking;
                Said::Nothing
            }
            (Stage::Talking | Stage::Called, t) if t == self.ids.call => {
                self.flush();
                self.stage = Stage::Calling;
                self.call.clear();
                Said::Nothing
            }
            (Stage::Calling, t) if t == self.ids.call_end => {
                self.stage = Stage::Called;
                self.answer.calls.push(parse_call(&String::from_utf8_lossy(&self.call)));
                Said::Call(self.answer.calls.len() - 1)
            }
            (Stage::Calling, t) => {
                self.call.extend_from_slice(tok.bytes(t));
                Said::Nothing
            }
            (Stage::Called, _) => Said::Nothing,
            (stage, t) => {
                self.held.extend_from_slice(tok.bytes(t));
                let whole = match core::str::from_utf8(&self.held) {
                    Ok(_) => self.held.len(),
                    // An unfinished character at the end waits for the next token; anything else broken is shown
                    // as the replacement character.
                    Err(e) if e.error_len().is_none() => e.valid_up_to(),
                    Err(_) => self.held.len(),
                };
                let s = String::from_utf8_lossy(&self.held[..whole]).into_owned();
                self.held.drain(..whole);
                self.said(stage, s)
            }
        }
    }

    fn said(&mut self, stage: Stage, s: String) -> Said {
        if s.is_empty() {
            return Said::Nothing;
        }
        match stage {
            Stage::Thinking => {
                // (Leading whitespace isn't shown; what's kept is trimmed at the end.)
                let shown = if self.answer.thinking.trim_start().is_empty() { s.trim_start().to_string() } else { s.clone() };
                self.answer.thinking.push_str(&s);
                if shown.is_empty() { Said::Nothing } else { Said::Thinking(shown) }
            }
            _ => {
                let shown = if self.started { s.clone() } else { s.trim_start().to_string() };
                if !shown.is_empty() {
                    self.started = true;
                }
                self.answer.text.push_str(&s);
                if shown.is_empty() { Said::Nothing } else { Said::Text(shown) }
            }
        }
    }

    /// Whatever bytes were held back, as they are.
    fn flush(&mut self) {
        if self.held.is_empty() {
            return;
        }
        let s = String::from_utf8_lossy(&self.held).into_owned();
        self.held.clear();
        match self.stage {
            Stage::Thinking => self.answer.thinking.push_str(&s),
            _ => self.answer.text.push_str(&s),
        }
    }

    /// The turn is over (the model ended it, or it's being cut short): what's kept is trimmed as the template
    /// trims it, and a tool call the model didn't finish is read as far as it got.
    pub fn finish(&mut self) -> Said {
        if self.done {
            return Said::Nothing;
        }
        self.flush();
        self.done = true;
        let call = (self.stage == Stage::Calling && !self.call.is_empty()).then(|| parse_call(&String::from_utf8_lossy(&self.call)));
        self.answer.thinking = strip(&self.answer.thinking).to_string();
        self.answer.text = strip(&self.answer.text).to_string();
        if let Some(c) = call.filter(|c| !c.name.is_empty()) {
            self.answer.calls.push(c);
            return Said::Call(self.answer.calls.len() - 1);
        }
        Said::Done
    }
}

/// A tool call as the model writes it: <function=NAME> then <parameter=NAME>VALUE</parameter>s then </function>
/// (each closing tag may be missing; a value loses one newline at each end, which the format puts there).
pub fn parse_call(s: &str) -> Call {
    let mut call = Call::default();
    let Some(at) = s.find("<function=") else { return call };
    let body = &s[at + "<function=".len()..];
    let body = &body[..body.find("</function>").unwrap_or(body.len())];
    let (name, mut rest) = match body.find('>') {
        Some(gt) => (&body[..gt], &body[gt + 1..]),
        None => (body, ""),
    };
    call.name = strip(name).to_string();
    while let Some(at) = rest.find("<parameter=") {
        let after = &rest[at + "<parameter=".len()..];
        let Some(gt) = after.find('>') else { break };
        let name = strip(&after[..gt]).to_string();
        let value = &after[gt + 1..];
        let end = [value.find("</parameter>"), value.find("<parameter=")].into_iter().flatten().min().unwrap_or(value.len());
        let mut v = &value[..end];
        v = v.strip_prefix('\n').unwrap_or(v);
        v = v.strip_suffix('\n').unwrap_or(v);
        call.args.push((name, v.to_string()));
        rest = &value[end..];
        rest = rest.strip_prefix("</parameter>").unwrap_or(rest);
    }
    call
}

impl Call {
    /// The arguments as JSON, each read as the type the tool's description gives it (as vLLM reads them for these
    /// models): text stays text; integers, numbers and true/false are read as such; objects and lists as JSON;
    /// "null" is null. An argument the tool doesn't describe, or one that doesn't read as its type, stays text.
    pub fn arguments(&self, tool: Option<&Json>) -> Json {
        let props = tool
            .and_then(|t| t.get("function").or(Some(t)))
            .and_then(|f| f.get("parameters"))
            .and_then(|p| p.get("properties"));
        let mut out = Vec::new();
        for (name, value) in &self.args {
            let kind = props
                .and_then(|p| p.get(name))
                .and_then(|p| p.get("type"))
                .and_then(|t| t.str().or_else(|| t.array().and_then(|a| a.iter().find_map(|x| x.str().filter(|s| *s != "null")))))
                .map(|s| s.trim().to_ascii_lowercase());
            out.push((name.clone(), typed(value, kind.as_deref())));
        }
        Json::Object(out)
    }
}

fn typed(value: &str, kind: Option<&str>) -> Json {
    let text = || Json::Str(value.to_string());
    let Some(kind) = kind else { return text() };
    if value.eq_ignore_ascii_case("null") {
        return Json::Null;
    }
    if ["string", "str", "text", "varchar", "char", "enum"].contains(&kind) {
        return text();
    }
    if ["int", "uint", "long", "short", "unsigned"].iter().any(|k| kind.starts_with(k)) {
        return match value.trim().parse::<i64>() {
            Ok(n) => Json::Number(n as f64),
            Err(_) => text(),
        };
    }
    if kind.starts_with("num") || kind.starts_with("float") {
        return match crate::file::parse_f64(value.trim()) {
            Some(n) if n.is_finite() => Json::Number(n),
            _ => text(),
        };
    }
    if ["boolean", "bool", "binary"].contains(&kind) {
        return Json::Bool(value.trim().eq_ignore_ascii_case("true"));
    }
    json::parse(value.trim()).unwrap_or_else(text)
}

#[cfg(test)]
mod tests;

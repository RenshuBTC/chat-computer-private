//! The engine's own tests, on small models made here with random weights (a dense one and one with experts), so
//! they need no download: reading many tokens at once gives exactly what reading them one at a time does,
//! snapshots go back exactly, a session's reuse changes no answer, and the arithmetic's kernels agree. With
//! pictures too (a small model that sees): their rows read at once as one at a time, the template's writing of
//! them, a session reusing what it read only for the same picture.
//! (Against the published models and the reference implementation: tools/refcheck.)

use crate::chat::{Answer, Message, Options, Style};
use crate::file::File;
use crate::net::{Model, Serial};
use crate::quant::{self, Dtype, Prepared};
use crate::sample::Sampling;
use crate::session::{Request, Session};
use crate::testing::{tiny, tiny_seeing, Rng};
use alloc::vec;
use alloc::vec::Vec;

fn tokens(n: usize, seed: u64) -> Vec<u32> {
    let mut r = Rng(seed);
    (0..n).map(|_| ((r.float() + 1.0) * 127.9) as u32).collect()
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

#[test]
fn many_tokens_at_once_are_read_as_one_at_a_time() {
    for experts in [false, true] {
        let bytes = tiny(experts);
        let file = File::open(&bytes).unwrap();
        let m = Model::load(&file).unwrap();
        // More than one batch's worth, and positions far enough for attention to matter.
        let toks = tokens(300, 7);
        let (mut one, mut many) = (m.state(), m.state());
        let (mut a, mut b) = (vec![0.0; 300], vec![0.0; 300]);
        for (i, &t) in toks.iter().enumerate() {
            m.step(&mut one, t, (i + 1 == toks.len()).then_some(&mut a[..]), &Serial);
        }
        m.forward(&mut many, &toks, Some(&mut b), &Serial);
        assert_eq!(max_diff(&a, &b), 0.0, "experts: {experts}");
        assert!(a.iter().all(|v| v.is_finite()) && max_diff(&a, &[0.0; 300]) > 0.01);
    }
}

#[test]
fn a_snapshot_goes_back_exactly() {
    let bytes = tiny(false);
    let file = File::open(&bytes).unwrap();
    let m = Model::load(&file).unwrap();
    let (first, second, other) = (tokens(50, 1), tokens(30, 2), tokens(30, 3));
    let mut st = m.state();
    let mut logits = vec![0.0; 300];
    m.forward(&mut st, &first, None, &Serial);
    let snap = st.snapshot();
    m.forward(&mut st, &second, None, &Serial);
    st.rewind(&snap);
    m.forward(&mut st, &other, Some(&mut logits), &Serial);
    let mut fresh = m.state();
    let mut want = vec![0.0; 300];
    let all: Vec<u32> = first.iter().chain(&other).copied().collect();
    m.forward(&mut fresh, &all, Some(&mut want), &Serial);
    assert_eq!(max_diff(&logits, &want), 0.0);
    assert_eq!(st.pos, 80);
}

#[test]
fn a_session_reads_only_what_is_new_and_answers_the_same() {
    let bytes = tiny(true);
    let file = File::open(&bytes).unwrap();
    let m = Model::load(&file).unwrap();
    let o = Options { think: Some(false), keep_thinking: Some(true), ..Options::default() };
    let ask = |s: &mut Session, msgs: &[Message]| {
        let r = Request { tools: &[], messages: msgs, options: o, sampling: Sampling::GREEDY, max_tokens: 12, seed: 1 };
        s.ask(&r, &Serial, &mut |_, _| true).unwrap()
    };
    let mut s = Session::new(&m, Style::QWEN35);
    let mut msgs = vec![Message::System("Be brief.".into()), Message::User("Hello there".into())];
    let (a1, st1) = ask(&mut s, &msgs);
    assert_eq!(st1.read, st1.prompt);
    msgs.push(Message::Assistant(a1.clone()));
    msgs.push(Message::User("And again".into()));
    let (a2, st2) = ask(&mut s, &msgs);
    // Only the new turn was read: the first prompt and the answer's own tokens were there already.
    assert!(st2.read < st2.prompt - st1.prompt, "{} of {}", st2.read, st2.prompt);
    let mut fresh = Session::new(&m, Style::QWEN35);
    ask(&mut fresh, &msgs[..2]);
    let (b2, _) = ask(&mut fresh, &msgs);
    assert_eq!(a2, b2);
    // The last message changed: from the first prompt's snapshot, and the same answer as a new session's.
    *msgs.last_mut().unwrap() = Message::User("Something else".into());
    let (a3, st3) = ask(&mut s, &msgs);
    assert!(st3.read < st3.prompt);
    let mut fresh = Session::new(&m, Style::QWEN35);
    ask(&mut fresh, &msgs[..2]);
    let (b3, _) = ask(&mut fresh, &msgs);
    assert_eq!(a3, b3);
    // Asked again: nothing to read.
    let (a4, st4) = ask(&mut s, &msgs);
    assert_eq!((a4, st4.read), (a3, 0));
}

#[test]
fn a_step_at_a_time_answers_as_all_at_once() {
    use crate::session::Step;
    let bytes = tiny(true);
    let file = File::open(&bytes).unwrap();
    let m = Model::load(&file).unwrap();
    let o = Options { think: Some(false), keep_thinking: Some(true), ..Options::default() };
    let mut msgs = vec![Message::System("A system prompt that's read in parts.".into()), Message::User("Hello there".into())];
    fn r(msgs: &[Message], o: Options) -> Request<'_> {
        Request { tools: &[], messages: msgs, options: o, sampling: Sampling::GREEDY, max_tokens: 10, seed: 3 }
    }
    let mut whole = Session::new(&m, Style::QWEN35);
    let mut parts = Session::new(&m, Style::QWEN35);
    for round in 0..2 {
        let (want, want_st) = whole.ask(&r(&msgs, o), &Serial, &mut |_, _| true).unwrap();
        let mut t = parts.begin(&r(&msgs, o)).unwrap();
        let (mut reads, mut said) = (0, 0);
        loop {
            match parts.step(&mut t, 7, &Serial) {
                Step::Read { done, total } => {
                    reads += 1;
                    assert!(done <= total);
                }
                Step::Seeing { .. } => panic!("a picture, where there's none"),
                Step::Said(_) => said += 1,
                Step::Over => break,
            }
        }
        let (got, st) = parts.end(t);
        assert_eq!((got.clone(), st.read, st.prompt, st.generated), (want.clone(), want_st.read, want_st.prompt, want_st.generated));
        assert_eq!(reads, want_st.read.div_ceil(7), "round {round}");
        assert!(said >= st.generated);
        msgs.push(Message::Assistant(got));
        msgs.push(Message::User("And again".into()));
    }
    // Stopped part way through reading: nothing said, and the next request reads only what's left.
    let mut t = parts.begin(&r(&msgs, o)).unwrap();
    parts.step(&mut t, 2, &Serial);
    parts.stop(&mut t);
    assert!(matches!(parts.step(&mut t, 2, &Serial), Step::Over));
    let (a, st) = parts.end(t);
    assert_eq!((a.text.as_str(), st.generated), ("", 0));
    let (b, st) = parts.ask(&r(&msgs, o), &Serial, &mut |_, _| true).unwrap();
    let (c, _) = whole.ask(&r(&msgs, o), &Serial, &mut |_, _| true).unwrap();
    assert!(st.read < st.prompt && b == c);
}

#[test]
fn the_kernels_agree() {
    use crate::cpu::Level;
    let mut r = Rng(42);
    for (dtype, cols) in [(Dtype::Q8, 256usize), (Dtype::Q4, 256), (Dtype::Q8, 96), (Dtype::Q4, 32)] {
        let rows = 5;
        let w = r.vec(rows * cols, 1.0);
        let mut data = Vec::new();
        if dtype == Dtype::Q8 { quant::quantize_q8(&w, &mut data) } else { quant::quantize_q4(&w, &mut data) }
        let xs: Vec<Prepared> = (0..9).map(|_| Prepared::new(&r.vec(cols, 2.0))).collect();
        let refs: Vec<&Prepared> = xs.iter().collect();
        for row in 0..rows {
            let mut want = vec![0.0f32; xs.len()];
            quant::dot_row_many_at(Level::Plain, dtype, &data, cols, row, &refs, &mut want);
            // Exactly the same integer sums; the floats only in their order of adding up.
            for level in [Level::Avx2, Level::AvxVnni, Level::Avx512Vnni] {
                let mut got = vec![0.0f32; xs.len()];
                quant::dot_row_many_at(level, dtype, &data, cols, row, &refs, &mut got);
                for (g, e) in got.iter().zip(&want) {
                    assert!((g - e).abs() <= 1e-4 * e.abs().max(1.0), "{dtype:?} {level:?}: {g} vs {e}");
                }
            }
            // And against the numbers themselves, before quantizing.
            for (t, x) in xs.iter().enumerate() {
                let exact: f32 = w[row * cols..(row + 1) * cols].iter().zip(&x.f).map(|(a, b)| a * b).sum();
                let close = if dtype == Dtype::Q8 { 0.02 } else { 0.25 };
                assert!((want[t] - exact).abs() <= close * (cols as f32).sqrt(), "{dtype:?}: {} vs {exact}", want[t]);
            }
        }
    }
}

#[test]
fn the_system_prompt_read_ahead_is_read_once() {
    let bytes = tiny(true);
    let file = File::open(&bytes).unwrap();
    let m = Model::load(&file).unwrap();
    let o = Options { think: Some(false), keep_thinking: Some(true), ..Options::default() };
    let tool = crate::json::parse(r#"{"type":"function","function":{"name":"look","description":"Looks.","parameters":{"type":"object","properties":{}}}}"#).unwrap();
    let tools = [tool];
    let system = Message::System("A system prompt read before anything's asked.".into());
    let mut msgs = vec![system.clone(), Message::User("Hello there".into())];
    let r = |msgs: &[Message]| -> (Answer, crate::session::Stats, Answer, crate::session::Stats, usize) {
        let r = Request { tools: &tools, messages: msgs, options: o, sampling: Sampling::GREEDY, max_tokens: 6, seed: 5 };
        let mut whole = Session::new(&m, Style::QWEN35);
        let (want, want_st) = whole.ask(&r, &Serial, &mut |_, _| true).unwrap();
        let mut ahead = Session::new(&m, Style::QWEN35);
        let first = ahead.ahead(&r).unwrap();
        let mut calls = 0;
        while !ahead.read_ahead(&first, 5, &Serial) {
            calls += 1;
        }
        assert_eq!(calls + 1, first.len().div_ceil(5));
        // Read already: nothing more to do.
        assert!(ahead.read_ahead(&first, 5, &Serial));
        let (got, st) = ahead.ask(&r, &Serial, &mut |_, _| true).unwrap();
        (want, want_st, got, st, first.len())
    };
    let (want, want_st, got, st, first) = r(&msgs);
    assert_eq!(got, want);
    // Only what follows the system prompt was read for the question.
    assert_eq!((st.prompt, st.read, want_st.read), (want_st.prompt, want_st.prompt - first, want_st.prompt));
    // A conversation that goes on, and a new one: the system prompt isn't read again.
    msgs.push(Message::Assistant(got));
    msgs.push(Message::User("And again".into()));
    let (want, _, got, _, _) = r(&msgs);
    assert_eq!(got, want);
    fn rq<'a>(tools: &'a [crate::json::Json], msgs: &'a [Message], o: Options) -> Request<'a> {
        Request { tools, messages: msgs, options: o, sampling: Sampling::GREEDY, max_tokens: 6, seed: 5 }
    }
    let mut s = Session::new(&m, Style::QWEN35);
    let first = s.ahead(&rq(&tools, &msgs, o)).unwrap();
    while !s.read_ahead(&first, 64, &Serial) {}
    s.ask(&rq(&tools, &msgs, o), &Serial, &mut |_, _| true).unwrap();
    let fresh = vec![system, Message::User("Something new".into())];
    let (_, st) = s.ask(&rq(&tools, &fresh, o), &Serial, &mut |_, _| true).unwrap();
    assert_eq!(st.read, st.prompt - first.len());
}

#[test]
fn a_damaged_file_is_refused() {
    let mut bytes = tiny(false);
    assert!(File::open(&bytes[..100]).is_err());
    let at = bytes.windows(5).position(|w| w == b"embed").unwrap();
    bytes[at..at + 5].copy_from_slice(b"embex");
    let file = File::open(&bytes).unwrap();
    assert!(Model::load(&file).is_err());
    assert_eq!(File::open(b"not a model").err(), Some(crate::Error::NotModel));
}

#[test]
fn the_anchor_kept_on_disk_reads_back_exactly() {
    let bytes = tiny(false);
    let file = File::open(&bytes).unwrap();
    let m = Model::load(&file).unwrap();
    let o = Options { think: Some(false), keep_thinking: Some(true), ..Options::default() };
    let msgs = vec![Message::System("A long system prompt, read once.".into()), Message::User("Hello there".into())];
    let r = Request { tools: &[], messages: &msgs, options: o, sampling: Sampling::GREEDY, max_tokens: 8, seed: 1 };
    let mut s = Session::new(&m, Style::QWEN35);
    let (a, _) = s.ask(&r, &Serial, &mut |_, _| true).unwrap();
    let (tokens, state, snap) = s.anchor().expect("an anchor where the system prompt ends");
    let (tokens, saved) = (tokens.to_vec(), state.export(snap));
    // A new session (as after a restart) from what was kept: only what follows the system prompt is read.
    let restored = m.import(&saved).unwrap();
    assert_eq!(restored.pos, tokens.len());
    let mut again = Session::new(&m, Style::QWEN35);
    again.restore(tokens.clone(), restored);
    let (b, st) = again.ask(&r, &Serial, &mut |_, _| true).unwrap();
    assert_eq!(a, b);
    assert_eq!(st.read, st.prompt - tokens.len());
    // Bytes that don't fit the model aren't taken.
    assert!(m.import(&saved[..saved.len() - 4]).is_none());
}

/// A picture for the model, made here.
fn a_picture(width: usize, height: usize, seed: u64) -> alloc::sync::Arc<crate::chat::Picture> {
    alloc::sync::Arc::new(crate::chat::Picture::new(width, height, crate::testing::picture(width, height, seed)))
}

#[test]
fn the_template_writes_pictures_as_qwens_does() {
    use crate::chat::{render, Content};
    let pic = a_picture(16, 16, 1);
    let msgs = vec![
        Message::UserParts(vec![Content::Text("  Look:\n".into()), Content::Picture(pic.clone()), Content::Text(" what's this?  ".into())]),
        Message::Assistant(Answer { calls: vec![crate::chat::Call { name: "show".into(), args: vec![] }], ..Answer::default() }),
        Message::ToolParts(vec![Content::Picture(pic.clone()), Content::Text("The picture.".into())]),
    ];
    let o = Options { think: Some(false), ..Options::default() };
    let p = render(Style::QWEN35, &[], &msgs, &o).unwrap();
    // (As Qwen 3.5's Jinja template writes it: each content's parts joined, then stripped.)
    assert_eq!(
        p.to_text(),
        "<|im_start|>user\nLook:\n<|vision_start|><|image_pad|><|vision_end|> what's this?<|im_end|>\n\
         <|im_start|>assistant\n<think>\n\n</think>\n\n<tool_call>\n<function=show>\n</function>\n</tool_call><|im_end|>\n\
         <|im_start|>user\n<tool_response>\n<|vision_start|><|image_pad|><|vision_end|>The picture.\n</tool_response><|im_end|>\n\
         <|im_start|>assistant\n<think>\n\n</think>\n\n"
    );
}

#[test]
fn pictures_rows_at_once_are_read_as_one_at_a_time() {
    use crate::chat::Content;
    for experts in [false, true] {
        let bytes = tiny_seeing(experts);
        let file = File::open(&bytes).unwrap();
        let m = Model::load(&file).unwrap();
        assert!(m.vision.is_some());
        let mut s = Session::new(&m, Style::QWEN35);
        // A picture of 40 by 24 pixels: read at 40 by 24 (5 by 3 rows), between words.
        let msgs = vec![Message::UserParts(vec![Content::Text("Hi".into()), Content::Picture(a_picture(40, 24, 2)), Content::Text("what".into())])];
        let o = Options { think: Some(false), ..Options::default() };
        let r = Request { tools: &[], messages: &msgs, options: o, sampling: Sampling::GREEDY, max_tokens: 8, seed: 1 };
        let toks = s.prompt(&r).unwrap();
        let rows = toks.iter().filter(|&&t| crate::net::picture_of(t).is_some()).count();
        assert_eq!(rows, 15);
        s.read(&toks, &Serial);
        let all = s.scores().to_vec();
        // The same tokens one at a time (in a session of their own, so the rows are made again).
        let mut s2 = Session::new(&m, Style::QWEN35);
        let toks2 = s2.prompt(&r).unwrap();
        assert_eq!(toks, toks2);
        for i in 1..=toks.len() {
            s2.read(&toks[..i], &Serial);
        }
        assert_eq!(max_diff(&all, s2.scores()), 0.0, "experts: {experts}");
        assert!(all.iter().all(|v| v.is_finite()));
        // And the picture matters: another picture, other scores.
        let msgs3 = vec![Message::UserParts(vec![Content::Text("Hi".into()), Content::Picture(a_picture(40, 24, 3)), Content::Text("what".into())])];
        let r3 = Request { messages: &msgs3, ..r };
        let mut s3 = Session::new(&m, Style::QWEN35);
        let t3 = s3.prompt(&r3).unwrap();
        s3.read(&t3, &Serial);
        assert!(max_diff(&all, s3.scores()) > 0.001);
    }
}

#[test]
fn a_session_reuses_what_it_read_only_for_the_same_picture() {
    use crate::chat::Content;
    let bytes = tiny_seeing(true);
    let file = File::open(&bytes).unwrap();
    let m = Model::load(&file).unwrap();
    let o = Options { think: Some(false), keep_thinking: Some(true), ..Options::default() };
    let ask = |s: &mut Session, msgs: &[Message]| {
        let r = Request { tools: &[], messages: msgs, options: o, sampling: Sampling::GREEDY, max_tokens: 6, seed: 1 };
        s.ask(&r, &Serial, &mut |_, _| true).unwrap()
    };
    let with = |seed: u64, q: &str| Message::UserParts(vec![Content::Picture(a_picture(32, 32, seed)), Content::Text(q.into())]);
    let mut s = Session::new(&m, Style::QWEN35);
    let mut msgs = vec![Message::System("Look.".into()), with(5, "What is it?")];
    let (a1, st1) = ask(&mut s, &msgs);
    assert_eq!(st1.read, st1.prompt);
    // The conversation goes on: only the new turn is read (the picture's rows were).
    msgs.push(Message::Assistant(a1));
    msgs.push(Message::User("And?".into()));
    let (a2, st2) = ask(&mut s, &msgs);
    assert!(st2.read < st2.prompt - st1.prompt, "{} of {}", st2.read, st2.prompt);
    {
        let mut fresh = Session::new(&m, Style::QWEN35);
        let (f2, _) = ask(&mut fresh, &msgs);
        assert_eq!(a2, f2);
    }
    // Another picture in the first message's place: read again from it, and answered as a new session answers.
    let msgs2 = vec![Message::System("Look.".into()), with(6, "What is it?")];
    let (b1, sb) = ask(&mut s, &msgs2);
    assert!(sb.read >= 16, "only {} read", sb.read);
    let mut fresh = Session::new(&m, Style::QWEN35);
    let (fb, _) = ask(&mut fresh, &msgs2);
    assert_eq!(b1, fb);
    // The first picture, which no request has any more, let go; the second kept while its request has it.
    assert_eq!(s.pictures_held(), 2);
    drop(msgs);
    let _ = ask(&mut s, &msgs2);
    assert_eq!(s.pictures_held(), 1);
    // A model without a vision part says it can't see.
    let blind_bytes = tiny(true);
    let blind_file = File::open(&blind_bytes).unwrap();
    let blind = Model::load(&blind_file).unwrap();
    let mut s = Session::new(&blind, Style::QWEN35);
    let r = Request { tools: &[], messages: &msgs2, options: o, sampling: Sampling::GREEDY, max_tokens: 6, seed: 1 };
    assert_eq!(s.begin(&r).err(), Some(crate::chat::Problem::Blind));
}

#[test]
fn pictures_are_sized_as_qwens_processor_sizes_them() {
    use crate::vision::smart_resize;
    // (As transformers' smart_resize gives them, for Qwen 3.5's bounds: 32 pixels a row, 65,536 to 16,777,216.)
    for &(h, w, want) in &[
        ((375, 500), (384, 512)),
        ((1080, 1920), (1088, 1920)),
        ((100, 100), (256, 256)),
        ((4000, 3000), (4000, 3008)),
        ((20, 4000), (32, 4000)),
        ((6000, 8000), (3520, 4704)),
    ].map(|(hw, want)| (hw.0, hw.1, want)) {
        assert_eq!(smart_resize(h, w, 32, 65536, 16_777_216), want, "{h}x{w}");
    }
}

#[test]
fn a_picture_seen_a_step_at_a_time_answers_as_all_at_once() {
    use crate::chat::Content;
    use crate::session::Step;
    let bytes = tiny_seeing(false);
    let file = File::open(&bytes).unwrap();
    let m = Model::load(&file).unwrap();
    let o = Options { think: Some(false), ..Options::default() };
    let msgs = vec![
        Message::User("First, no picture.".into()),
        Message::Assistant(Answer { text: "Fine.".into(), ..Answer::default() }),
        Message::UserParts(vec![Content::Picture(a_picture(56, 40, 9)), Content::Text("And this?".into())]),
    ];
    let r = Request { tools: &[], messages: &msgs, options: o, sampling: Sampling::GREEDY, max_tokens: 8, seed: 2 };
    let mut whole = Session::new(&m, Style::QWEN35);
    let (want, _) = whole.ask(&r, &Serial, &mut |_, _| true).unwrap();
    let mut parts = Session::new(&m, Style::QWEN35);
    parts.see = 16;
    let mut t = parts.begin(&r).unwrap();
    let (mut seeing, mut last) = (Vec::new(), 0);
    loop {
        match parts.step(&mut t, 5, &Serial) {
            Step::Seeing { done, total } => seeing.push((done, total)),
            Step::Read { done, .. } => last = done,
            Step::Said(_) => {}
            Step::Over => break,
        }
    }
    let (got, st) = parts.end(t);
    assert_eq!(got, want);
    // (56x40 pixels, 4-pixel patches: 140 patches, in 35 groups. Begun; each patch embedded and through each of the
    // 2 blocks' 3 parts, 16 patches a step; then the merger's, 4 groups a step: then read.)
    let total = 140 * (1 + 3 * 2) + 35;
    assert!(seeing.iter().all(|&(_, t)| t == total));
    assert_eq!(seeing.len(), 1 + 9 + 2 * 3 * 9 + 9);
    assert_eq!(seeing.first(), Some(&(0, total)));
    assert_eq!(seeing.last(), Some(&(total, total)));
    assert!(seeing.windows(2).all(|w| w[0].0 < w[1].0));
    assert_eq!(last, st.prompt);
}

#[test]
fn a_pictures_rows_are_the_same_however_many_patches_a_step_takes() {
    let bytes = tiny_seeing(false);
    let file = File::open(&bytes).unwrap();
    let m = Model::load(&file).unwrap();
    let v = m.vision.as_ref().unwrap();
    let pic = a_picture(56, 40, 3);
    let sized = v.prepare(&pic.rgb, 56, 40, usize::MAX);
    let patches = v.patches(&sized);
    let ps = v.settings.patch;
    let (gh, gw) = (sized.height / ps, sized.width / ps);
    let whole = v.encode(&patches, gh, gw, &Serial);
    for most in [1, 3, 7, 64] {
        let mut e = v.begin(patches.clone(), gh, gw);
        let mut last = v.progress(&e).0;
        let rows = loop {
            if let Some(rows) = v.advance(&mut e, most, &Serial) {
                break rows;
            }
            let (done, total) = v.progress(&e);
            assert!(done > last && done < total, "{most} a step: {done} of {total} after {last}");
            last = done;
        };
        assert_eq!(v.progress(&e), (140 * 7 + 35, 140 * 7 + 35));
        assert!(rows == whole, "{most} patches a step made different rows");
    }
}

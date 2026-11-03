//! The engine's own tests, on small models made here with random weights (a dense one and one with experts), so
//! they need no download: reading many tokens at once gives exactly what reading them one at a time does,
//! snapshots go back exactly, a session's reuse changes no answer, and the arithmetic's kernels agree.
//! (Against the published models and the reference implementation: tools/refcheck.)

use crate::chat::{Message, Options, Style};
use crate::file::File;
use crate::net::{Model, Serial};
use crate::quant::{self, Dtype, Prepared};
use crate::sample::Sampling;
use crate::session::{Request, Session};
use crate::testing::{tiny, Rng};
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

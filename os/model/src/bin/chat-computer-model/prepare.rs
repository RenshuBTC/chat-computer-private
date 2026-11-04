//! Getting a model ready, by the engine itself (the desktop app has it do this): which model this computer runs
//! well (as big as its memory runs well, with room on its disk to get it ready), its files downloaded from Qwen's
//! own repository at the revision pinned in models.json (each checked against its pinned SHA-256; a download that
//! stops carries on where it stopped), converted into the engine's own file, its refusals found and taken out
//! (find-refusal), and the downloads removed. It's ready when its file has its name, and not before.
//!
//! A file made before the engine kept the part of the model that sees (one without that part, for a model that sees,
//! made by the converter from before: convert::CONVERTER) is made again the same way, the one there answering
//! meanwhile: the new one waits beside it, finished (NAME.ccm.new, with NAME.refusal.new), until it can take its
//! place, at once where the system lets a file in use be replaced, else (Windows, while the app serves the old one)
//! when `models` is next asked, which the app does once it has stopped serving it.
//!
//!     chat-computer-model models DIR [--catalogue FILE]
//!     chat-computer-model prepare DIR [--model NAME|auto] [--catalogue FILE] [--threads N]
//!
//! DIR holds the models: NAME.ccm, NAME.refusal, and download-NAME/ while one comes. `models` says (JSON) which
//! there are, which are here (and which of those are to be made again: "outdated"), and which this computer would
//! get, having put any file made again in its place first; `prepare` gets one ready, saying how it goes on
//! stderr, a line at a time ("downloading DONE TOTAL"; "converting", then its "STEP/STEPS weight"; "uncensoring",
//! then the refusal search's "STEP/STEPS what"; "ready FILE", or "failed WHY"), and the file on stdout.
//!
//! Downloads go over Chat Computer's own HTTPS (fetch.rs).

use crate::{fetch, sys};
use crypto::Hash;
use model::json::{self, Json};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The models (Chat Computer's chatcomputer/brain/models.json, as it was when this program was built).
const CATALOGUE: &str = include_str!("../../../../../chatcomputer/brain/models.json");
const HUB: &str = "https://huggingface.co";

/// Where the models' files come from: Hugging Face, or (CHATCOMPUTER_HUB, an https address) a copy of it, such as
/// Chat Computer's tests' own. (Each file is still checked against the SHA-256 pinned in the catalogue.)
fn hub() -> String {
    std::env::var("CHATCOMPUTER_HUB").ok().filter(|h| h.starts_with("https://")).unwrap_or_else(|| HUB.to_string())
}
/// The disk space left for everything else once a model is ready (when choosing one).
const ROOM_LEFT: u64 = 4 << 30;
const TRIES: usize = 8;

#[derive(Debug)]
pub struct FileSpec {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug)]
pub struct Model {
    pub name: String,
    pub label: String,
    pub repo: String,
    pub revision: String,
    pub memory_gb: f64,
    pub bits: u32,
    pub files: Vec<FileSpec>,
    /// Whether it sees (its weights have a vision part the engine runs: the catalogue says).
    pub sees: bool,
}

impl Model {
    pub fn download_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// About how big its own file is: its weights at `bits` a weight (the vision part's, a few in a hundred of
    /// them, at 8 even in a 4-bit model: a little more than this says).
    pub fn file_bytes(&self) -> u64 {
        let weights: u64 = self.files.iter().filter(|f| f.name.ends_with(".safetensors")).map(|f| f.size).sum();
        (weights as f64 / 2.0 * (self.bits as f64 + 0.5) / 8.0 * 1.02) as u64
    }
}

/// The models in a catalogue (biggest first).
pub fn catalogue(text: &str) -> Result<Vec<Model>, String> {
    let j = json::parse(text).ok_or("the catalogue isn't JSON")?;
    let list = j.get("models").and_then(Json::array).ok_or("the catalogue has no models")?;
    let mut out = Vec::new();
    for m in list {
        let s = |k: &str| m.get(k).and_then(Json::str).map(str::to_string).ok_or(format!("a model without {k}"));
        let mut files = Vec::new();
        for f in m.get("files").and_then(Json::array).ok_or("a model without files")? {
            let name = f.get("name").and_then(Json::str).ok_or("a file without a name")?.to_string();
            // (Each is downloaded into the model's own folder and nowhere else.)
            if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
                return Err(format!("a file name that isn't plain: {name}"));
            }
            let sha256 = f.get("sha256").and_then(Json::str).ok_or("a file without its SHA-256")?.to_ascii_lowercase();
            if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("{name}'s SHA-256 isn't one"));
            }
            files.push(FileSpec { name, size: f.get("size").and_then(Json::num).ok_or("a file without its size")? as u64, sha256 });
        }
        let name = s("name")?;
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b)) || name.starts_with('.') {
            return Err(format!("a model name that isn't plain: {name}"));
        }
        out.push(Model {
            name,
            label: s("label")?,
            repo: s("repo")?,
            revision: s("revision")?,
            memory_gb: m.get("memory_gb").and_then(Json::num).unwrap_or(0.0),
            bits: m.get("bits").and_then(Json::num).unwrap_or(4.0) as u32,
            files,
            sees: m.get("sees").and_then(Json::bool).unwrap_or(false),
        });
    }
    Ok(out)
}

/// Whether this computer's memory runs the model well (with room for everything else: a computer said to have 32 GB
/// shows a little less, some being the graphics' or the firmware's).
pub fn runs_in(m: &Model, memory: u64) -> bool {
    memory as f64 / (1u64 << 30) as f64 + 1.5 >= m.memory_gb
}

pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn path(&self, m: &Model) -> PathBuf {
        self.dir.join(format!("{}.ccm", m.name))
    }

    pub fn refusal_path(&self, m: &Model) -> PathBuf {
        self.dir.join(format!("{}.refusal", m.name))
    }

    pub fn downloads(&self, m: &Model) -> PathBuf {
        self.dir.join(format!("download-{}", m.name))
    }

    pub fn ready(&self, m: &Model) -> Option<PathBuf> {
        let p = self.path(m);
        p.metadata().ok().filter(|md| md.is_file() && md.len() > 0).map(|_| p)
    }

    /// Where a file made again waits, finished, until it can take the old one's place: it, and its refusal
    /// direction.
    pub fn made_again(&self, m: &Model) -> (PathBuf, PathBuf) {
        (self.dir.join(format!("{}.ccm.new", m.name)), self.dir.join(format!("{}.refusal.new", m.name)))
    }

    /// Whether its file was made before the engine kept the part that sees, for a model that sees: the file has
    /// none of it, and no converter's said it made it. Such a file is made again.
    pub fn outdated(&self, m: &Model) -> bool {
        let Some(settings) = self.ready(m).filter(|_| m.sees).and_then(|p| settings_of(&p)) else { return false };
        !settings.lines().filter_map(|l| l.split_once('=')).map(|(k, _)| k.trim()).any(|k| k == "converter" || k.starts_with("vision."))
    }

    /// A file made again, finished, put in the old one's place, with its refusal direction (without one, the old
    /// file's goes, as it isn't this file's: it's looked for again). Whether it could be: on Windows, not while
    /// something has the old one open (the app serving it), so it waits, and the next try puts it there.
    pub fn put_in_place(&self, m: &Model) -> bool {
        let (file, refusal) = self.made_again(m);
        if !file.is_file() || std::fs::rename(&file, self.path(m)).is_err() {
            return false;
        }
        if std::fs::rename(&refusal, self.refusal_path(m)).is_err() {
            let _ = std::fs::remove_file(self.refusal_path(m));
        }
        true
    }

    /// Whether its refusals have been looked for (and taken out, if it had any), the engine's current way.
    pub fn uncensored(&self, m: &Model) -> bool {
        let mut head = [0u8; 20];
        std::fs::File::open(self.refusal_path(m)).and_then(|mut f| f.read_exact(&mut head)).is_ok()
            && &head[..8] == model::refusal::MAGIC
            && u32::from_le_bytes(head[16..20].try_into().unwrap()) == model::refusal::METHOD
    }

    /// Room needed to get it ready: what's still to download, its own file, and a little more.
    pub fn needed(&self, m: &Model) -> u64 {
        let d = self.downloads(m);
        let have: u64 = m
            .files
            .iter()
            .map(|f| {
                let size = |p: PathBuf| p.metadata().map(|md| md.len()).unwrap_or(0);
                size(d.join(&f.name)) + size(d.join(format!("{}.part", f.name)))
            })
            .sum();
        m.download_bytes().saturating_sub(have) + m.file_bytes() + (1 << 30)
    }

    /// The biggest model this computer's memory runs well (the list is biggest first) that's here already or that
    /// the disk has room to get ready; the smallest, if none.
    pub fn pick<'a>(&self, models: &'a [Model], memory: u64, free: Option<u64>) -> &'a Model {
        let fit: Vec<&Model> = models.iter().filter(|m| runs_in(m, memory)).collect();
        let fit = if fit.is_empty() { vec![models.last().unwrap()] } else { fit };
        let Some(free) = free else { return fit[0] };
        fit.iter().find(|m| self.ready(m).is_some() || self.needed(m) + ROOM_LEFT <= free).copied().unwrap_or(fit[fit.len() - 1])
    }
}

/// A model file's settings (its "config" entry), read from the file's start and its table, without mapping it (on
/// Windows a mapping would keep it open, and it couldn't be replaced): None if it isn't a model file.
pub fn settings_of(path: &Path) -> Option<String> {
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).ok()?;
    if &head[..8] != model::file::MAGIC {
        return None;
    }
    let len = u32::from_le_bytes(head[8..12].try_into().unwrap()) as usize;
    if len > 64 << 20 {
        return None;
    }
    let mut table = vec![0u8; len];
    f.read_exact(&mut table).ok()?;
    let table = String::from_utf8(table).ok()?;
    // "config blob - OFFSET SIZE", its offset from the data's start (the next multiple of 64 after the table).
    let line = table.lines().find(|l| l.split(' ').next() == Some("config"))?;
    let parts: Vec<&str> = line.split(' ').collect();
    let (offset, size): (u64, usize) = (parts.get(3)?.parse().ok()?, parts.get(4)?.parse().ok()?);
    if size > 1 << 20 {
        return None;
    }
    f.seek(SeekFrom::Start((12 + len).div_ceil(64) as u64 * 64 + offset)).ok()?;
    let mut text = vec![0u8; size];
    f.read_exact(&mut text).ok()?;
    String::from_utf8(text).ok()
}

fn quoted(s: &str) -> String {
    let mut out = String::new();
    json::quote(s, &mut out);
    out
}

struct Args {
    dir: PathBuf,
    model: String,
    catalogue: Option<PathBuf>,
    threads: Option<usize>,
}

fn args(command: &str, a: Vec<String>) -> Result<Args, String> {
    let (mut dir, mut model, mut catalogue, mut threads) = (None, "auto".to_string(), None, None);
    let mut i = 0;
    while i < a.len() {
        let value = a.get(i + 1).cloned().unwrap_or_default();
        match a[i].as_str() {
            "--model" => model = value,
            "--catalogue" => catalogue = Some(PathBuf::from(value)),
            "--threads" => threads = value.parse().ok(),
            p if dir.is_none() && !p.starts_with("--") => {
                dir = Some(PathBuf::from(p));
                i += 1;
                continue;
            }
            other => return Err(format!("{command}: {other}?")),
        }
        i += 2;
    }
    Ok(Args { dir: dir.ok_or(format!("{command}: which folder?"))?, model, catalogue, threads })
}

fn load(a: &Args) -> Result<Vec<Model>, String> {
    let text = match &a.catalogue {
        Some(p) => std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?,
        None => CATALOGUE.to_string(),
    };
    let models = catalogue(&text)?;
    if models.is_empty() {
        return Err("the catalogue has no models".into());
    }
    Ok(models)
}

/// `models DIR`: which models there are, which are here, and which this computer would get.
pub fn models_main(a: Vec<String>) -> i32 {
    let run = || -> Result<String, String> {
        let a = args("models", a)?;
        let models = load(&a)?;
        let store = Store { dir: a.dir.clone() };
        let _ = std::fs::create_dir_all(&store.dir);
        // (A file made again, finished, takes its place now: nothing serves the old one while the app asks this.)
        for m in &models {
            store.put_in_place(m);
        }
        let (memory, free) = (sys::memory_bytes(), sys::free_bytes(&store.dir));
        let pick = store.pick(&models, memory, free);
        let list: Vec<String> = models
            .iter()
            .map(|m| {
                format!(
                    "{{\"name\":{},\"label\":{},\"memory_gb\":{},\"bits\":{},\"download\":{},\"file\":{},\"ready\":{},\"outdated\":{},\"uncensored\":{},\"runs\":{},\"needed\":{}}}",
                    quoted(&m.name), quoted(&m.label), m.memory_gb, m.bits, m.download_bytes(), m.file_bytes(),
                    store.ready(m).is_some(), store.outdated(m), store.uncensored(m), runs_in(m, memory), store.needed(m)
                )
            })
            .collect();
        Ok(format!(
            "{{\"memory\":{memory},\"free\":{},\"pick\":{},\"models\":[{}]}}",
            free.map_or("null".to_string(), |f| f.to_string()), quoted(&pick.name), list.join(",")
        ))
    };
    match run() {
        Ok(s) => {
            println!("{s}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// `prepare DIR`: the model chosen (or the one this computer would get) got ready.
pub fn prepare_main(a: Vec<String>) -> i32 {
    let run = || -> Result<PathBuf, String> {
        let a = args("prepare", a)?;
        let models = load(&a)?;
        let store = Store { dir: a.dir.clone() };
        std::fs::create_dir_all(&store.dir).map_err(|e| format!("{}: {e}", store.dir.display()))?;
        let m = if a.model == "auto" || a.model.is_empty() {
            store.pick(&models, sys::memory_bytes(), sys::free_bytes(&store.dir))
        } else {
            models.iter().find(|m| m.name == a.model).ok_or(format!("there's no model called {}", a.model))?
        };
        prepare(&store, m, a.threads)
    };
    match run() {
        Ok(p) => {
            println!("{}", p.display());
            0
        }
        Err(e) => {
            eprintln!("failed {e}");
            1
        }
    }
}

fn prepare(store: &Store, m: &Model, threads: Option<usize>) -> Result<PathBuf, String> {
    // (One made again before, finished: in its place now, if it can be; else it waits for the old one to be let go,
    // and isn't made again meanwhile.)
    let (made, _) = store.made_again(m);
    if !store.put_in_place(m) && made.is_file() {
        eprintln!("ready {}", made.display());
        return Ok(made);
    }
    // One made before the engine kept the part that sees is made again; the one here answers meanwhile.
    let again = store.outdated(m);
    if let Some(p) = store.ready(m).filter(|_| !again) {
        if !store.uncensored(m) {
            uncensor(&p, &store.refusal_path(m), threads)?;
        }
        eprintln!("ready {}", p.display());
        return Ok(p);
    }
    let need = store.needed(m);
    if let Some(free) = sys::free_bytes(&store.dir)
        && free < need
    {
        let what = if again { "to be made again so it can see pictures (the one here answers meanwhile)" } else { "to get ready" };
        return Err(format!(
            "{} needs {:.1} GB of free space {what}, and there's {:.1} GB",
            m.label, need as f64 / 1e9, free as f64 / 1e9
        ));
    }
    let d = store.downloads(m);
    std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    let total = m.download_bytes();
    let mut done = 0u64;
    let mut told = Instant::now();
    for f in &m.files {
        download(m, f, &d, total, &mut done, &mut told)?;
    }
    eprintln!("downloading {total} {total}");
    eprintln!("converting");
    let out = store.path(m);
    let tmp = store.dir.join(format!("{}.ccm.part", m.name));
    crate::convert::run(&d, &tmp, m.bits)?;
    if again {
        // Finished beside the old one, then in its place if it can be (else once nothing serves the old one).
        let (file, refusal) = store.made_again(m);
        let _ = std::fs::remove_file(&refusal);
        uncensor(&tmp, &refusal, threads)?;
        std::fs::rename(&tmp, &file).map_err(|e| format!("{}: {e}", file.display()))?;
        let _ = std::fs::remove_dir_all(&d);
        let done = if store.put_in_place(m) { out } else { file };
        eprintln!("ready {}", done.display());
        return Ok(done);
    }
    uncensor(&tmp, &store.refusal_path(m), threads)?;
    std::fs::rename(&tmp, &out).map_err(|e| format!("{}: {e}", out.display()))?;
    let _ = std::fs::remove_dir_all(&d);
    eprintln!("ready {}", out.display());
    Ok(out)
}

/// Its refusals found, kept in `out` (by this program, as its own process: the model file it maps is let go when
/// it ends).
fn uncensor(file: &Path, out: &Path, threads: Option<usize>) -> Result<(), String> {
    eprintln!("uncensoring");
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(me);
    cmd.arg("find-refusal").arg(file).arg("--out").arg(out);
    if crate::WITH_STDIN.load(std::sync::atomic::Ordering::Relaxed) {
        cmd.arg("--exit-with-stdin");        // (its standard input is this one's: it ends with it)
    }
    if let Some(n) = threads {
        cmd.arg("--threads").arg(n.to_string());
    }
    let status = cmd.stdout(std::process::Stdio::null()).status().map_err(|e| e.to_string())?;
    if !status.success() {
        // (It answers as it is, then; this is tried again the next time.)
        eprintln!("uncensoring didn't finish: it answers as it was made, for now");
    }
    Ok(())
}

fn download(m: &Model, f: &FileSpec, d: &Path, total: u64, done: &mut u64, told: &mut Instant) -> Result<(), String> {
    let dest = d.join(&f.name);
    if dest.metadata().map(|md| md.len()).ok() == Some(f.size) {
        *done += f.size;        // (checked when it finished downloading)
        return Ok(());
    }
    let part = d.join(format!("{}.part", f.name));
    let url = format!("{}/{}/resolve/{}/{}", hub().trim_end_matches('/'), m.repo, m.revision, f.name);
    let mut tries = 0;
    loop {
        let mut have = part.metadata().map(|md| md.len()).unwrap_or(0);
        if have > f.size {
            let _ = std::fs::remove_file(&part);
            have = 0;
        }
        // What's there already, counted into the hash.
        let mut h = crypto::Sha256::new();
        if have > 0 {
            let mut r = std::fs::File::open(&part).map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; 1 << 20];
            loop {
                let n = r.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
        }
        let base = *done;
        let mut wrote = 0u64;
        let mut out = std::fs::OpenOptions::new().create(true).append(true).open(&part).map_err(|e| e.to_string())?;
        let mut restart = false;
        let got = if have < f.size {
            fetch::get(&url, have, &mut sys::random, &mut |p| match p {
                fetch::Part::Head { status, length } => {
                    if have > 0 && status == 200 {
                        restart = true;        // (the server sent it all: from the start, then)
                        return Ok(false);
                    }
                    if status != 200 && status != 206 {
                        return Err(format!("{}: the server said {status}", f.name));
                    }
                    if length.is_some_and(|n| have + n != f.size) {
                        return Err(format!("{}: the server's is a different size from the one pinned", f.name));
                    }
                    Ok(true)
                }
                fetch::Part::Body(b) => {
                    out.write_all(b).map_err(|e| format!("{}: {e}", part.display()))?;
                    h.update(b);
                    wrote += b.len() as u64;
                    if told.elapsed() > Duration::from_secs(1) {
                        *told = Instant::now();
                        eprintln!("downloading {} {total}", base + have + wrote);
                    }
                    Ok(have + wrote <= f.size)
                }
            })
        } else {
            Ok(())
        };
        drop(out);
        if restart {
            let _ = std::fs::remove_file(&part);
            continue;
        }
        match got {
            Ok(()) if have + wrote == f.size => {
                let mut digest = String::new();
                for b in h.finish().iter() {
                    digest.push_str(&format!("{b:02x}"));
                }
                if digest != f.sha256 {
                    let _ = std::fs::remove_file(&part);
                    return Err(format!("{} came down different from what Chat Computer pinned (its SHA-256): not used", f.name));
                }
                std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
                *done += f.size;
                return Ok(());
            }
            Ok(()) if have + wrote > f.size => {
                let _ = std::fs::remove_file(&part);
                return Err(format!("{} came down bigger than it is", f.name));
            }
            other => {
                tries += 1;
                let why = other.err().unwrap_or_else(|| "it stopped short".into());
                if tries > TRIES {
                    return Err(format!("downloading {} failed: {why}", f.name));
                }
                eprintln!("download of {} broke off ({why}): again in {} s", f.name, 2 * tries);
                std::thread::sleep(Duration::from_secs(2 * tries as u64));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    #[test]
    fn the_catalogue_reads_and_the_biggest_that_runs_is_picked() {
        let models = catalogue(CATALOGUE).unwrap();
        let names: Vec<&str> = models.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["qwen3.8-27b", "qwen3.6-35b-a3b", "qwen3.5-9b", "qwen3.5-4b", "qwen3.5-2b", "qwen3.5-0.8b"]);
        let dir = std::env::temp_dir().join(format!("cc-pick-{}", std::process::id()));
        let store = Store { dir: dir.clone() };
        for (gb, want) in [(128, "qwen3.8-27b"), (47, "qwen3.8-27b"), (46, "qwen3.6-35b-a3b"), (31, "qwen3.6-35b-a3b"),
                           (16, "qwen3.5-9b"), (12, "qwen3.5-4b"), (8, "qwen3.5-4b"), (6, "qwen3.5-2b"), (3, "qwen3.5-2b"),
                           (2, "qwen3.5-0.8b")] {
            assert_eq!(store.pick(&models, gb * GB, None).name, want, "{gb} GB");
        }
        // Without room for the 27B (about 72 GB to get ready), 48 GB of memory gets the 9B with 40 GB free.
        assert_eq!(store.pick(&models, 48 * GB, Some(40_000_000_000)).name, "qwen3.5-9b");
        // One that's here already is kept, however full the disk is.
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("qwen3.6-35b-a3b.ccm"), b"x").unwrap();
        assert_eq!(store.pick(&models, 48 * GB, Some(1_000_000_000)).name, "qwen3.6-35b-a3b");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A model file with these settings (and a little else in it, before them).
    fn file_with(path: &Path, settings: &str) {
        let mut w = model::file::Writer::default();
        w.add("tokenizer.tokens", model::quant::Dtype::Blob, &[], b"x");
        w.add("config", model::quant::Dtype::Blob, &[], settings.as_bytes());
        let mut bytes = w.head();
        bytes.extend_from_slice(&w.data);
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_file_made_before_the_part_that_sees_was_kept_is_made_again() {
        let models = catalogue(CATALOGUE).unwrap();
        assert!(models.iter().all(|m| m.sees), "Qwen's models here all see");
        let m = models.iter().find(|m| m.name == "qwen3.5-0.8b").unwrap();
        let dir = std::env::temp_dir().join(format!("cc-again-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store { dir: dir.clone() };
        // Not here: nothing to make again.
        assert!(!store.outdated(m));
        // Made before (no converter said it made it, and no vision part): made again.
        file_with(&store.path(m), "arch=qwen3_5\nhidden=8\n");
        assert_eq!(settings_of(&store.path(m)).as_deref(), Some("arch=qwen3_5\nhidden=8\n"));
        assert!(store.outdated(m));
        // With the vision part, or made by a converter that says so (with it or not): not.
        file_with(&store.path(m), "arch=qwen3_5\nvision.depth=2\n");
        assert!(!store.outdated(m));
        file_with(&store.path(m), &format!("converter={}\narch=qwen3_5\n", model::convert::CONVERTER));
        assert!(!store.outdated(m));
        // Nor one of a model the catalogue doesn't say sees.
        let plain = r#"{"models":[{"name":"qwen3.5-0.8b","label":"M","repo":"Q/M","revision":"r","memory_gb":0,"bits":8,"files":[]}]}"#;
        let blind = &catalogue(plain).unwrap()[0];
        file_with(&store.path(blind), "arch=qwen3_5\n");
        assert!(!blind.sees && !store.outdated(blind) && store.outdated(m));
        // Made again, it takes the old one's place, with its refusal direction.
        std::fs::write(store.refusal_path(m), b"old").unwrap();
        let (file, refusal) = store.made_again(m);
        file_with(&file, "converter=2\narch=qwen3_5\nvision.depth=2\n");
        std::fs::write(&refusal, b"new").unwrap();
        assert!(store.put_in_place(m));
        assert!(!file.exists() && !refusal.exists() && !store.outdated(m));
        assert_eq!(std::fs::read(store.refusal_path(m)).unwrap(), b"new");
        // Without its own refusal direction, the old one's goes (it isn't this file's: it's looked for again).
        file_with(&file, "converter=2\n");
        assert!(store.put_in_place(m) && !store.refusal_path(m).exists() && !store.uncensored(m));
        // Nothing made again: nothing to put in place.
        assert!(!store.put_in_place(m));
        // Not a model file: not known to be outdated.
        std::fs::write(store.path(m), b"not a model file").unwrap();
        assert!(settings_of(&store.path(m)).is_none() && !store.outdated(m));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_catalogue_that_names_files_outside_their_folder_is_refused() {
        let bad = r#"{"models":[{"name":"m","label":"M","repo":"Q/M","revision":"r","memory_gb":0,"bits":8,
                     "files":[{"name":"../x","size":1,"sha256":"0000000000000000000000000000000000000000000000000000000000000000"}]}]}"#;
        assert!(catalogue(bad).unwrap_err().contains("isn't plain"));
        assert!(catalogue(&bad.replace("../x", "x").replace("\"m\"", "\"../m\"")).is_err());
    }
}

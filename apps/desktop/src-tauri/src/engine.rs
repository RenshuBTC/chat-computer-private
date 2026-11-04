//! The model, on this computer: Chat Computer's own engine (os/model's chat-computer-model, which comes with the
//! app, beside its program), with no account and nothing sent anywhere. At the start the app asks it which model
//! this computer runs well (`models`); serves the one that's here (`serve`, on 127.0.0.1, OpenAI's chat completions
//! API, for the app's page and Chat Computer's website only); and, while the one chosen isn't here yet, has the
//! engine get it ready (`prepare`: downloaded over Chat Computer's own HTTPS, checked, converted, its refusals taken
//! out), at a lower priority, telling the page how it goes; then serves that one. A model file made before the
//! engine kept the part of the model that sees (`models` says it's "outdated") is made again the same way, the old
//! one answering meanwhile; once the new one's finished, serving stops for a moment (on Windows a file in use can't
//! be replaced), `models` puts it in the old one's place, and it's served.
//!
//! The engine's processes end with the app (--exit-with-stdin: the app holds their standard input). Should the one
//! serving stop by itself (out of memory, say), it's started again, telling the page.

use serde::Serialize;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The port the website looks for the model on (and the app's page, unless another program has it).
pub const PORT: u16 = 8765;

/// How it's going, for the page.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Status {
    /// starting | preparing | ready | failed | none (no engine for this computer)
    pub state: String,
    /// The model that answers now ("" while none does), and the one being got ready.
    pub label: String,
    pub preparing: String,
    /// Whether the one being got ready is the one that answers, made again (so it can see pictures).
    pub updating: bool,
    /// downloading | converting | uncensoring (failed: getting it ready stopped, though another answers), and how
    /// far: bytes, or steps.
    pub stage: String,
    pub done: u64,
    pub total: u64,
    pub port: u16,
    pub detail: String,
}

#[derive(Default)]
pub struct Engine {
    pub status: Mutex<Status>,
    /// The serving process and its standard input (it ends when that closes), and the preparing one's.
    serving: Mutex<Option<(Child, Option<ChildStdin>)>>,
    preparing: Mutex<Option<(Child, Option<ChildStdin>)>>,
    /// The file served (to start it again, should it stop by itself); one start at a time; the app closing.
    served: Mutex<Option<PathBuf>>,
    starting: Mutex<()>,
    closing: AtomicBool,
}

/// The engine's program, beside the app's (where Tauri puts what the app brings: its externalBin).
pub fn program() -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    let p = me.parent()?.join(format!("chat-computer-model{}", std::env::consts::EXE_SUFFIX));
    p.is_file().then_some(p)
}

/// A command for the engine: no window of its own on Windows, ending with the app.
fn command(program: &Path, args: &[&str], lower: bool) -> Command {
    let mut c = Command::new(program);
    c.args(args).arg("--exit-with-stdin").stdin(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        c.creation_flags(CREATE_NO_WINDOW | if lower { BELOW_NORMAL_PRIORITY_CLASS } else { 0 });
    }
    #[cfg(unix)]
    if lower && Path::new("/usr/bin/nice").is_file() {
        let mut n = Command::new("/usr/bin/nice");
        n.arg("-n").arg("10").arg(program).args(args).arg("--exit-with-stdin").stdin(Stdio::piped());
        return n;
    }
    let _ = lower;
    c
}

/// What `models` says: the models, which are here, and which this computer would get.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct Models {
    pub pick: String,
    pub models: Vec<ModelInfo>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub label: String,
    pub ready: bool,
    /// Here, but made before the engine kept the part that sees: it's made again (an engine from before says nothing).
    #[serde(default)]
    pub outdated: bool,
    pub runs: bool,
}

impl Models {
    /// The model that answers now: the one this computer gets, once it's here; else the biggest one here that it
    /// runs well (or, if none does, the smallest here).
    pub fn serving(&self) -> Option<&ModelInfo> {
        let here: Vec<&ModelInfo> = self.models.iter().filter(|m| m.ready).collect();
        here.iter().find(|m| m.name == self.pick).or_else(|| here.iter().find(|m| m.runs)).or(here.last()).copied()
    }

    pub fn picked(&self) -> Option<&ModelInfo> {
        self.models.iter().find(|m| m.name == self.pick)
    }

    /// The one chosen, if it's to be got ready: not here yet, or here but to be made again.
    pub fn to_prepare(&self) -> Option<&ModelInfo> {
        self.picked().filter(|p| !p.ready || p.outdated)
    }
}

/// A line of `prepare`'s, read into the status: its stage, and how far it's got.
pub fn progress(s: &mut Status, line: &str) {
    let line = line.trim();
    let mut words = line.split_whitespace();
    match words.next() {
        Some("downloading") => {
            s.stage = "downloading".into();
            s.done = words.next().and_then(|w| w.parse().ok()).unwrap_or(s.done);
            s.total = words.next().and_then(|w| w.parse().ok()).unwrap_or(s.total);
        }
        Some(stage @ ("converting" | "uncensoring")) => {
            s.stage = stage.into();
            (s.done, s.total) = (0, 0);
        }
        Some(w) if w.contains('/') => {
            if let Some((d, t)) = w.split_once('/') {
                if let (Ok(d), Ok(t)) = (d.parse(), t.parse()) {
                    (s.done, s.total) = (d, t);
                }
            }
        }
        Some("failed") => s.detail = line.trim_start_matches("failed").trim().to_string(),
        _ => {}
    }
}

/// Whether something answers as the engine does at 127.0.0.1:port (its /health).
fn healthy(port: u16) -> bool {
    let Ok(mut s) = std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let req = format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out.starts_with("HTTP/1.1 200") && out.contains("\"ok\":true")
}

fn free_port() -> u16 {
    (PORT..PORT + 10).find(|p| std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok()).unwrap_or(PORT)
}

impl Engine {
    fn set(&self, tell: &dyn Fn(&Status), f: impl FnOnce(&mut Status)) {
        let now = {
            let mut s = self.status.lock().unwrap_or_else(|p| p.into_inner());
            f(&mut s);
            s.clone()
        };
        tell(&now);
    }

    /// Serves `file` (stopping what served before), and waits until it answers.
    fn serve(&self, program: &Path, dir: &Path, file: &Path, origins: &[String]) -> Result<u16, String> {
        let _one = self.starting.lock().unwrap_or_else(|p| p.into_inner());
        if self.closing.load(Ordering::Relaxed) {
            return Err("the app is closing".into());
        }
        if let Some((mut c, _)) = self.serving.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let port = free_port();
        let (file_s, cache, port_s) = (file.to_string_lossy().into_owned(), dir.join("cache").to_string_lossy().into_owned(), port.to_string());
        let mut args = vec!["serve", &file_s, "--port", &port_s, "--cache", &cache];
        for o in origins {
            args.push("--allow-origin");
            args.push(o);
        }
        let log = std::fs::File::create(dir.join("model.log")).ok();
        let mut cmd = command(program, &args, false);
        cmd.stdout(Stdio::null()).stderr(log.map(Stdio::from).unwrap_or(Stdio::null()));
        let mut child = cmd.spawn().map_err(|e| format!("the model's engine didn't start: {e}"))?;
        let stdin = child.stdin.take();
        let started = Instant::now();
        loop {
            if healthy(port) {
                break;
            }
            if let Ok(Some(code)) = child.try_wait() {
                return Err(format!("the model's engine stopped as it started ({code})"));
            }
            if started.elapsed() > Duration::from_secs(120) {
                let _ = child.kill();
                return Err("the model's engine didn't start in time".into());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        *self.serving.lock().unwrap_or_else(|p| p.into_inner()) = Some((child, stdin));
        *self.served.lock().unwrap_or_else(|p| p.into_inner()) = Some(file.to_path_buf());
        Ok(port)
    }

    /// Stops serving: the model file is let go (so a file made again can take its place).
    fn stop_serving(&self) {
        let _one = self.starting.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((mut c, stdin)) = self.serving.lock().unwrap_or_else(|p| p.into_inner()).take() {
            drop(stdin);
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// Should the engine serving stop by itself (not stopped by the app), it's started again on the same file: up to
    /// 3 times.
    fn watch(&self, program: &Path, dir: &Path, origins: &[String], tell: &dyn Fn(&Status)) {
        let mut again = 0;
        while !self.closing.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(2));
            let ended = {
                let mut slot = self.serving.lock().unwrap_or_else(|p| p.into_inner());
                let gone = matches!(slot.as_mut().map(|(c, _)| c.try_wait()), Some(Ok(Some(_))));
                if gone { slot.take() } else { None }
            };
            let file = self.served.lock().unwrap_or_else(|p| p.into_inner()).clone();
            let (Some(_), Some(file)) = (ended, file) else { continue };
            if self.closing.load(Ordering::Relaxed) {
                return;
            }
            if again == 3 {
                self.set(tell, |s| {
                    s.state = "failed".into();
                    s.detail = "the model's engine kept stopping".into();
                });
                return;
            }
            again += 1;
            self.set(tell, |s| s.state = "starting".into());
            match self.serve(program, dir, &file, origins) {
                Ok(port) => self.set(tell, |s| {
                    s.state = "ready".into();
                    s.port = port;
                }),
                Err(e) => self.set(tell, |s| {
                    s.state = "failed".into();
                    s.detail = e;
                }),
            }
        }
    }

    /// Everything, from the start: what's here served, the model chosen got ready, then served.
    pub fn run(self: &Arc<Self>, dir: PathBuf, catalogue: Option<PathBuf>, origins: Vec<String>,
               tell: Arc<dyn Fn(&Status) + Send + Sync>) {
        let Some(program) = program() else {
            self.set(&*tell, |s| {
                s.state = "none".into();
                s.detail = "the model's engine isn't with this app".into();
            });
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        let d = dir.to_string_lossy().into_owned();
        // (Only a build for testing names another catalogue.)
        let extra: Vec<String> = catalogue.map(|c| vec!["--catalogue".into(), c.to_string_lossy().into_owned()]).unwrap_or_default();
        let models = || -> Result<Models, String> {
            let mut a = vec!["models".to_string(), d.clone()];
            a.extend(extra.iter().cloned());
            let a: Vec<&str> = a.iter().map(String::as_str).collect();
            let mut child = command(&program, &a, false).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
                .map_err(|e| e.to_string())?;
            let stdin = child.stdin.take();             // (held until it's answered: closed, it would end at once)
            let out = child.wait_with_output().map_err(|e| e.to_string())?;
            drop(stdin);
            serde_json::from_slice(&out.stdout).map_err(|e| format!("the engine's list of models: {e}"))
        };
        let serve_best = |m: &Models| {
            if let Some(best) = m.serving() {
                let file = dir.join(format!("{}.ccm", best.name));
                match self.serve(&program, &dir, &file, &origins) {
                    Ok(port) => self.set(&*tell, |s| {
                        s.state = "ready".into();
                        s.label = best.label.clone();
                        s.port = port;
                    }),
                    Err(e) => self.set(&*tell, |s| {
                        s.state = "failed".into();
                        s.detail = e;
                    }),
                }
            }
        };
        let m = match models() {
            Ok(m) => m,
            Err(e) => {
                self.set(&*tell, |s| {
                    s.state = "failed".into();
                    s.detail = e;
                });
                return;
            }
        };
        serve_best(&m);
        {
            let (me, program, dir, origins, tell) = (self.clone(), program.clone(), dir.clone(), origins.clone(), tell.clone());
            std::thread::spawn(move || me.watch(&program, &dir, &origins, &*tell));
        }
        let Some(pick) = m.to_prepare().cloned() else { return };
        // The one chosen, got ready (or made again, the one here answering meanwhile).
        let again = pick.ready;
        self.set(&*tell, |s| {
            if s.state != "ready" {
                s.state = "preparing".into();
            }
            s.preparing = pick.label.clone();
            s.updating = again;
            s.stage = "downloading".into();
        });
        let mut a = vec!["prepare".to_string(), d.clone(), "--model".into(), pick.name.clone()];
        a.extend(extra.iter().cloned());
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        let mut cmd = command(&program, &a, true);
        cmd.stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.set(&*tell, |s| s.detail = e.to_string());
                return;
            }
        };
        let err = child.stderr.take();
        let stdin = child.stdin.take();
        *self.preparing.lock().unwrap_or_else(|p| p.into_inner()) = Some((child, stdin));
        let mut told = Instant::now() - Duration::from_secs(10);
        if let Some(err) = err {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                let now = {
                    let mut s = self.status.lock().unwrap_or_else(|p| p.into_inner());
                    progress(&mut s, &line);
                    s.clone()
                };
                if told.elapsed() > Duration::from_secs(1) || !line.contains(char::is_numeric) {
                    told = Instant::now();
                    tell(&now);
                }
            }
        }
        let ok = self.preparing.lock().unwrap_or_else(|p| p.into_inner()).take()
            .and_then(|(mut c, _)| c.wait().ok()).is_some_and(|code| code.success());
        if ok && again && !self.closing.load(Ordering::Relaxed) {
            // (The old file let go, so the new one can take its place: `models` puts it there.)
            self.set(&*tell, |s| s.state = "starting".into());
            self.stop_serving();
        }
        let now = if ok { models().ok() } else { None };
        if let Some(m) = now {
            self.set(&*tell, |s| {
                s.preparing.clear();
                s.updating = false;
            });
            serve_best(&m);
        } else {
            self.set(&*tell, |s| {
                if s.state != "ready" {
                    s.state = "failed".into();
                }
                s.stage = "failed".into();
                if s.detail.is_empty() {
                    s.detail = "getting the model ready stopped".into();
                }
            });
        }
    }

    /// The app is closing: the engine's processes end.
    pub fn stop(&self) {
        self.closing.store(true, Ordering::Relaxed);
        for slot in [&self.serving, &self.preparing] {
            if let Some((mut c, stdin)) = slot.lock().unwrap_or_else(|p| p.into_inner()).take() {
                drop(stdin);
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_s_lines_say_how_it_goes() {
        let mut s = Status::default();
        progress(&mut s, "downloading 1200 9000");
        assert_eq!((s.stage.as_str(), s.done, s.total), ("downloading", 1200, 9000));
        progress(&mut s, "converting");
        progress(&mut s, "50/324 layers.11.k_norm (4 s)");
        assert_eq!((s.stage.as_str(), s.done, s.total), ("converting", 50, 324));
        progress(&mut s, "uncensoring");
        progress(&mut s, "8/440 reading the requests");
        assert_eq!((s.stage.as_str(), s.done, s.total), ("uncensoring", 8, 440));
        progress(&mut s, "failed Qwen3.5 9B needs 12.0 GB of free space to get ready, and there's 3.1 GB");
        assert!(s.detail.starts_with("Qwen3.5 9B needs"));
    }

    #[test]
    fn the_model_here_answers_while_the_one_chosen_comes() {
        let m = |name: &str, ready, runs| ModelInfo { name: name.into(), label: name.into(), ready, outdated: false, runs };
        let list = Models { pick: "9b".into(), models: vec![m("27b", false, false), m("9b", false, true), m("4b", true, true), m("0.8b", true, true)] };
        assert_eq!(list.serving().unwrap().name, "4b");
        assert_eq!(list.picked().unwrap().name, "9b");
        let ready = Models { pick: "9b".into(), models: vec![m("9b", true, true), m("4b", true, true)] };
        assert_eq!(ready.serving().unwrap().name, "9b");
        let none = Models { pick: "9b".into(), models: vec![m("9b", false, true)] };
        assert!(none.serving().is_none());
        // The one chosen is got ready while it isn't here, and not once it is.
        assert_eq!(list.to_prepare().unwrap().name, "9b");
        assert!(ready.to_prepare().is_none());
    }

    #[test]
    fn a_model_made_before_it_could_see_is_made_again_while_it_answers() {
        // What `models` says of one here made before the engine kept the part that sees ("outdated"); an engine
        // from before says nothing of it.
        let said: Models = serde_json::from_str(r#"{"pick":"4b","models":[
            {"name":"4b","label":"Qwen3.5 4B","ready":true,"outdated":true,"uncensored":true,"runs":true},
            {"name":"2b","label":"Qwen3.5 2B","ready":true,"runs":true}]}"#).unwrap();
        assert!(said.models[0].outdated && !said.models[1].outdated);
        // It answers meanwhile, and it's the one got ready (made again).
        assert_eq!(said.serving().unwrap().name, "4b");
        assert_eq!(said.to_prepare().unwrap().name, "4b");
        let done: Models = serde_json::from_str(r#"{"pick":"4b","models":[{"name":"4b","label":"Qwen3.5 4B","ready":true,"outdated":false,"runs":true}]}"#).unwrap();
        assert!(done.to_prepare().is_none());
    }
}

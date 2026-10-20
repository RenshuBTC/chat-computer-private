//! Chat Computer for Windows and Mac: the website's own page (chatcomputer.ai, bundled into the app, so no code
//! from elsewhere ever runs with the app's powers), in a window of its own, plus what a website can't do:
//!
//! - Any site opens right in the middle, in a real browser view laid over the frame the page draws for it (the
//!   "hole"), as on a Chat Computer: sites that won't show inside another page (most of them) open here too.
//! - "/" finds files on this computer by name, and opens the one chosen with its own app (programs and scripts
//!   aren't opened from here).
//! - The greeting has the name this computer knows you by.
//!
//! Only the app's own page (the webview called "main") can ask for these: the sites shown in the middle can't
//! (capabilities/, and each command checks who's asking).
#![cfg_attr(test, allow(dead_code))]

mod files;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod loopback;

use std::sync::Mutex;

use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl};

/// The app's own page, and the view of a site laid over it.
const MAIN: &str = "main";
const SITE: &str = "site";
/// On a Mac, the view of a site says it's Safari (it is Safari's engine): some sites turn away a browser they don't
/// know, and the same is done on a Chat Computer.
#[cfg(target_os = "macos")]
const SAFARI_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) \
                         Version/26.0 Safari/605.1.15";

/// Where the app's own page comes from: its origin, to tell it from anything else.
struct Home(Url);

/// The files the last searches found: the only ones the page can have opened.
#[derive(Default)]
struct Found(Mutex<files::Seen>);

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Only the app's own page may ask (not a site in the middle, whatever it does).
fn from_main(webview: &Webview) -> Result<(), String> {
    if webview.label() == MAIN {
        Ok(())
    } else {
        Err("Only Chat Computer's own page can ask for that.".into())
    }
}

/// A web address to show: http or https only.
fn web_url(s: &str) -> Result<Url, String> {
    let u = Url::parse(s.trim()).map_err(|_| "That isn't a web address.".to_string())?;
    match u.scheme() {
        "http" | "https" => Ok(u),
        _ => Err("Only web addresses (http or https) open here.".into()),
    }
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme() && a.host_str() == b.host_str() && a.port_or_known_default() == b.port_or_known_default()
}

/// Something for the app's own page to know (host.js, window.chatcomputerApp): a JSON message.
fn tell_main(app: &AppHandle, what: serde_json::Value) {
    if let Some(main) = app.get_webview(MAIN) {
        let _ = main.eval(format!("window.chatcomputerApp && window.chatcomputerApp({what})"));
    }
}

fn place(site: &Webview, x: f64, y: f64, w: f64, h: f64) -> Result<(), String> {
    site.set_position(LogicalPosition::new(x.max(0.0), y.max(0.0))).map_err(err)?;
    site.set_size(LogicalSize::new(w.max(1.0), h.max(1.0))).map_err(err)
}

/// A site in the middle, at the frame the page drew for it (x, y, w, h: CSS pixels of the page, which are the
/// window's logical pixels). The same view each time, so sign-ins and the way back stay.
#[tauri::command]
fn show_site(app: AppHandle, webview: Webview, url: String, x: f64, y: f64, w: f64, h: f64) -> Result<(), String> {
    from_main(&webview)?;
    let url = web_url(&url)?;
    if let Some(site) = app.get_webview(SITE) {
        site.navigate(url).map_err(err)?;
        place(&site, x, y, w, h)?;
        return site.show().map_err(err);
    }
    let (titled, loaded, saved, popup) = (app.clone(), app.clone(), app.clone(), app.clone());
    #[allow(unused_mut)]
    let mut builder = WebviewBuilder::new(SITE, WebviewUrl::External(url))
        // Only web pages (and their blank and blob pages); a mailto: or an app's own link isn't followed here.
        .on_navigation(|u| matches!(u.scheme(), "http" | "https" | "about" | "blob"))
        // A link that would open a new window or tab: here, in its place, as on a Chat Computer.
        .on_new_window(move |u, _features| {
            if matches!(u.scheme(), "http" | "https") {
                if let Some(site) = popup.get_webview(SITE) {
                    let _ = site.navigate(u);
                }
            }
            NewWindowResponse::Deny
        })
        .on_document_title_changed(move |_, title| tell_main(&titled, serde_json::json!({ "title": title })))
        .on_page_load(move |_, load| {
            if let PageLoadEvent::Finished = load.event() {
                tell_main(&loaded, serde_json::json!({ "url": load.url().as_str() }));
            }
        })
        // Downloads go to the Downloads folder, as a browser's do; the page says so in Actions.
        .on_download(move |_, event| {
            if let DownloadEvent::Finished { url, path, success } = event {
                let name = path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| url.path_segments().and_then(|mut s| s.next_back()).unwrap_or("a file").to_string());
                tell_main(&saved, serde_json::json!({ "downloaded": name, "ok": success }));
            }
            true
        });
    #[cfg(target_os = "macos")]
    {
        builder = builder.user_agent(SAFARI_UA);
    }
    webview
        .window()
        .add_child(builder, LogicalPosition::new(x.max(0.0), y.max(0.0)), LogicalSize::new(w.max(1.0), h.max(1.0)))
        .map_err(err)?;
    Ok(())
}

/// The frame moved or changed size (the window did, or the sidebars came in).
#[tauri::command]
fn place_site(app: AppHandle, webview: Webview, x: f64, y: f64, w: f64, h: f64) -> Result<(), String> {
    from_main(&webview)?;
    match app.get_webview(SITE) {
        Some(site) => place(&site, x, y, w, h),
        None => Ok(()),
    }
}

/// Out of sight (the conversation has the middle, or a sheet is over it): still there, still playing.
#[tauri::command]
fn hide_site(app: AppHandle, webview: Webview) -> Result<(), String> {
    from_main(&webview)?;
    match app.get_webview(SITE) {
        Some(site) => site.hide().map_err(err),
        None => Ok(()),
    }
}

/// Back in sight, where it was.
#[tauri::command]
fn unhide_site(app: AppHandle, webview: Webview) -> Result<(), String> {
    from_main(&webview)?;
    match app.get_webview(SITE) {
        Some(site) => site.show().map_err(err),
        None => Ok(()),
    }
}

/// Closed: gone, and what played stops.
#[tauri::command]
fn close_site(app: AppHandle, webview: Webview) -> Result<(), String> {
    from_main(&webview)?;
    match app.get_webview(SITE) {
        Some(site) => site.close().map_err(err),
        None => Ok(()),
    }
}

/// What's playing in the middle (a video, music): played, paused, muted or not, its volume (0 to 100); and the
/// site scrolled (Page Up and Down, and the arrow keys, Home and End while the message box is empty). A fixed
/// script, run in the site's page: nothing the page sends is run there.
#[tauri::command]
fn site_media(app: AppHandle, webview: Webview, action: String, value: Option<f64>) -> Result<(), String> {
    from_main(&webview)?;
    let js = site_script(&action, value)?;
    if let Some(site) = app.get_webview(SITE) {
        site.eval(js).map_err(err)?;
    }
    Ok(())
}

fn site_script(action: &str, value: Option<f64>) -> Result<String, String> {
    let scroll = |by: &str| format!("window.scrollBy({{ top: {by}, behavior: 'smooth' }});");
    let media = match action {
        "play" => "m.play()".to_string(),
        "pause" => "m.pause()".to_string(),
        "mute" => "m.muted = true".to_string(),
        "unmute" => "m.muted = false".to_string(),
        "volume" => format!("m.volume = {}", value.unwrap_or(100.0).clamp(0.0, 100.0) / 100.0),
        "scroll_page_up" => return Ok(scroll("-0.85 * innerHeight")),
        "scroll_page_down" => return Ok(scroll("0.85 * innerHeight")),
        "scroll_up" => return Ok(scroll("-0.15 * innerHeight")),
        "scroll_down" => return Ok(scroll("0.15 * innerHeight")),
        "scroll_top" => return Ok("window.scrollTo({ top: 0, behavior: 'smooth' });".into()),
        "scroll_bottom" => {
            return Ok("window.scrollTo({ top: document.documentElement.scrollHeight, behavior: 'smooth' });".into())
        }
        _ => return Err("That isn't something a video, music or a page does.".into()),
    };
    Ok(format!("document.querySelectorAll('video, audio').forEach(function (m) {{ try {{ {media}; }} catch (e) {{}} }});"))
}

/// The home folder, and the usual folders in it: Desktop, Documents, Downloads, Pictures, Music and Videos (where
/// the system keeps them: OneDrive's, when it backs them up), and OneDrive's own.
fn folders(app: &AppHandle) -> (Option<std::path::PathBuf>, Vec<std::path::PathBuf>) {
    let p = app.path();
    let mut roots: Vec<std::path::PathBuf> =
        [p.desktop_dir(), p.document_dir(), p.download_dir(), p.picture_dir(), p.audio_dir(), p.video_dir()]
            .into_iter()
            .flatten()
            .collect();
    for var in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
        if let Some(d) = std::env::var_os(var) {
            roots.push(d.into());
        }
    }
    (p.home_dir().ok(), roots)
}

/// "/": the files on this computer whose names have all the words, best first (at most 8).
#[tauri::command]
async fn search_files(webview: Webview, found: tauri::State<'_, Found>, query: String) -> Result<Vec<files::Hit>, String> {
    from_main(&webview)?;
    let (home, roots) = folders(webview.app_handle());
    let hits = tauri::async_runtime::spawn_blocking(move || files::search(home.as_deref(), &roots, &query, 8))
        .await
        .map_err(err)?;
    found.0.lock().map_err(err)?.add(hits.iter().map(|h| h.path.clone()));
    Ok(hits)
}

/// One of them, with its own app (Preview, Word, Photos...); a folder in Finder or File Explorer. Only what a
/// search found, and never a program or a script.
#[tauri::command]
fn open_file(webview: Webview, found: tauri::State<'_, Found>, path: String) -> Result<String, String> {
    from_main(&webview)?;
    let p = std::path::PathBuf::from(&path);
    if !found.0.lock().map_err(err)?.has(&p) {
        return Err("Only a file found with \u{201c}/\u{201d} opens from here.".into());
    }
    if !files::openable(&p) {
        return Err("Programs and scripts aren't opened from here: open it yourself if you're sure.".into());
    }
    open::that_detached(&p).map_err(err)?;
    Ok(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(path))
}

/// Who this computer says you are (your account's full name), for the greeting; and which computer it is.
#[tauri::command]
fn about(webview: Webview) -> Result<serde_json::Value, String> {
    from_main(&webview)?;
    let (real, user) = (whoami::realname(), whoami::username());
    let name = if real.trim().is_empty() || real.trim().eq_ignore_ascii_case(user.trim()) { String::new() } else { real };
    Ok(serde_json::json!({
        "name": name.trim(),
        "system": if cfg!(target_os = "macos") { "mac" } else if cfg!(windows) { "windows" } else { "linux" },
    }))
}

/// The address of the app's own page: Tauri's own scheme; on a Mac, a server only this computer can reach, at
/// 127.0.0.1 (a Mac's web view doesn't treat Tauri's scheme as secure, and without that there's no WebGPU for
/// Gemma, no microphone and no browser cache). The same port each time where it's free, so what the page keeps
/// (your name, Claude's key, Gemma) is there next time.
fn home_url(app: &tauri::App) -> Result<WebviewUrl, Box<dyn std::error::Error>> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        if cfg!(target_os = "macos") || std::env::var_os("CHATCOMPUTER_LOOPBACK").is_some() {
            let port = loopback::serve(app.asset_resolver())?;
            return Ok(WebviewUrl::External(format!("http://127.0.0.1:{port}/index.html").parse()?));
        }
    }
    let _ = app;
    Ok(WebviewUrl::App("index.html".into()))
}

pub fn run() {
    tauri::Builder::default()
        .manage(Found::default())
        .invoke_handler(tauri::generate_handler![
            show_site, place_site, hide_site, unhide_site, close_site, site_media, search_files, open_file, about
        ])
        .setup(|app| {
            let url = home_url(app)?;
            let window = tauri::window::WindowBuilder::new(app, "chat-computer")
                .title("Chat Computer")
                .inner_size(1280.0, 820.0)
                .min_inner_size(420.0, 560.0)
                .center()
                .build()?;
            let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
            let home = match &url {
                WebviewUrl::External(u) => u.clone(),
                _ => {
                    if cfg!(windows) { "http://tauri.localhost/" } else { "tauri://localhost/" }.parse()?
                }
            };
            app.manage(Home(home.clone()));
            let (back, opener, guard) = (home.clone(), app.handle().clone(), app.handle().clone());
            let opening = Mutex::new(std::env::var("CHATCOMPUTER_OPEN").ok().filter(|u| web_url(u).is_ok()));
            let main = WebviewBuilder::new(MAIN, url)
                .auto_resize()
                // The app's own page stays: a link followed opens in the middle instead (the page itself does that;
                // this catches the rest). Frames in it (a map, a page of its own) may load what they like.
                .on_navigation(move |u| {
                    if same_origin(u, &back) || matches!(u.scheme(), "about" | "data" | "blob") {
                        return true;
                    }
                    // On Windows only the page itself asks here; elsewhere its frames do too.
                    if cfg!(windows) && matches!(u.scheme(), "http" | "https") {
                        tell_main(&opener, serde_json::json!({ "open": u.as_str() }));
                        return false;
                    }
                    matches!(u.scheme(), "http" | "https")
                })
                .on_page_load(move |wv, load| {
                    // A site to open at the start (CHATCOMPUTER_OPEN, as the app's tests on GitHub's machines do).
                    if let PageLoadEvent::Finished = load.event() {
                        if let Some(first) = opening.lock().ok().and_then(|mut o| o.take()) {
                            tell_main(&guard, serde_json::json!({ "open": first }));
                        }
                    }
                    // Elsewhere, the page itself leaving: back to it, and the address opened in the middle.
                    if let PageLoadEvent::Started = load.event() {
                        let home = &guard.state::<Home>().0;
                        if !same_origin(load.url(), home) && matches!(load.url().scheme(), "http" | "https") {
                            let away = load.url().to_string();
                            let _ = wv.navigate(home.join("index.html").unwrap_or_else(|_| home.clone()));
                            let app = guard.clone();
                            std::thread::spawn(move || {
                                std::thread::sleep(std::time::Duration::from_millis(1500));
                                tell_main(&app, serde_json::json!({ "open": away }));
                            });
                        }
                    }
                })
                // A link meant for a new window: in the middle.
                .on_new_window({
                    let app = app.handle().clone();
                    move |u, _| {
                        if matches!(u.scheme(), "http" | "https") {
                            tell_main(&app, serde_json::json!({ "open": u.as_str() }));
                        }
                        NewWindowResponse::Deny
                    }
                });
            window.add_child(main, LogicalPosition::new(0.0, 0.0), size)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Chat Computer couldn't start");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_addresses_open_in_the_middle() {
        assert!(web_url("https://sg.yahoo.com/").is_ok());
        assert!(web_url(" http://example.com/a?b=c ").is_ok());
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "chrome://settings", "tauri://localhost/", "nonsense"] {
            assert!(web_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_fixed_scripts_run_in_a_site() {
        assert_eq!(site_script("volume", Some(250.0)).unwrap(),
                   "document.querySelectorAll('video, audio').forEach(function (m) { try { m.volume = 1; } catch (e) {} });");
        assert!(site_script("volume", Some(40.0)).unwrap().contains("m.volume = 0.4;"));
        assert!(site_script("scroll_page_down", None).unwrap().starts_with("window.scrollBy({ top: 0.85 * innerHeight"));
        assert!(site_script("alert(1)", None).is_err());
    }

    #[test]
    fn the_app_s_own_page_is_told_from_others() {
        let home: Url = "http://127.0.0.1:42851/index.html".parse().unwrap();
        assert!(same_origin(&"http://127.0.0.1:42851/demo/host.js".parse().unwrap(), &home));
        assert!(!same_origin(&"http://127.0.0.1:42852/".parse().unwrap(), &home));
        assert!(!same_origin(&"https://127.0.0.1:42851/".parse().unwrap(), &home));
        assert!(!same_origin(&"http://evil.example/".parse().unwrap(), &home));
    }
}

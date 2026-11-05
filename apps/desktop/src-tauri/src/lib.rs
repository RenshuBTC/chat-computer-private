//! Chat Computer for Windows and Mac: the website's own page (chatcomputer.ai, bundled into the app, so no code
//! from elsewhere ever runs with the app's powers), in a window of its own, plus what a website can't do:
//!
//! - Any site opens right in the middle, in a real browser view laid over the frame the page draws for it (the
//!   "hole"), as on a Chat Computer: sites that won't show inside another page (most of them) open here too.
//! - "/" finds files on this computer by name, and opens the one chosen with its own app (programs and scripts
//!   aren't opened from here).
//! - The greeting has the name this computer knows you by.
//! - Messages from your own Gmail, Outlook, Telegram, WhatsApp, WeChat and Discord, each signed in on its own page in
//!   a view of its own, put together in Messages (messages.rs).
//! - The model that answers, on this computer: Chat Computer's own engine, which comes with the app, gets the model
//!   this computer runs well ready and serves it to the page (engine.rs). No account, and nothing sent anywhere.
//! - Or, if you sign in with ChatGPT (the spark beside +), your ChatGPT plan answers: the app signs in on OpenAI's
//!   page and keeps the tokens, and asks OpenAI for the page (chatgpt.rs).
//! - How you have Chat Computer, asked the first time it starts (ways.rs, and its page, start/): as this app, or as
//!   its own system (Chat Computer Setup, which comes with the app).
//!
//! Only the app's own page (the webview called "main") can ask for these: the sites shown in the middle can't
//! (capabilities/, and each command checks who's asking). A connected service's own pages may hand the app what's
//! read there, and nothing else.
#![cfg_attr(test, allow(dead_code))]

mod chatgpt;
mod engine;
mod files;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod loopback;
mod messages;
mod ways;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl};

/// The app's own page, and the view of a site laid over it (in the browser, each tab's: "tab-" and its number).
const MAIN: &str = "main";
const SITE: &str = "site";
/// The app's window.
const WINDOW: &str = "chat-computer";
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

/// Which view is in the middle: the site's, a tab's, a service's, or none.
#[derive(Default)]
struct Middle(Mutex<String>);

/// How you have Chat Computer (ways.rs): the way chosen, once one has been.
#[derive(Default)]
struct Chosen(Mutex<Option<ways::Way>>);

/// The model's engine, started once the way is the app (a model of gigabytes, got ready for this computer: not for
/// the own system, which doesn't need the app's).
#[derive(Default)]
struct EngineStart(Mutex<Option<Box<dyn FnOnce() + Send>>>);

fn start_engine(app: &AppHandle) {
    let start = app.state::<EngineStart>().0.lock().ok().and_then(|mut s| s.take());
    if let Some(start) = start {
        start();
    }
}

/// A tab's view, by its number (the page numbers them: 1 to 500).
fn middle(app: &AppHandle) -> Option<Webview> {
    let label = app.state::<Middle>().0.lock().ok()?.clone();
    app.get_webview(&label)
}

fn set_middle(app: &AppHandle, label: &str) {
    if let Ok(mut m) = app.state::<Middle>().0.lock() {
        *m = label.to_string();
    }
}

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

/// A service's view out of sight: beside the window, as big as it, still shown (a page that's hidden stops drawing
/// its list, and some stop keeping it up to date), so it stays signed in and its reader keeps reading. (On Linux,
/// where the app is built only for its tests, GTK lays the views out and has no room beside the window: hidden.)
fn aside(view: &Webview) -> Result<(), String> {
    if cfg!(target_os = "linux") {
        return view.hide().map_err(err);
    }
    let (w, h) = match view.window().inner_size().and_then(|s| Ok(s.to_logical::<f64>(view.window().scale_factor()?))) {
        Ok(s) => (s.width.max(800.0), s.height.max(600.0)),
        Err(_) => (1200.0, 800.0),
    };
    view.set_position(LogicalPosition::new(-(w + 64.0), 0.0)).map_err(err)?;
    view.set_size(LogicalSize::new(w, h)).map_err(err)?;
    view.show().map_err(err)
}

/// What's in the middle goes, for `label` to take its place: a service's view is put aside (still signed in and
/// read); the site's closes, as a site does when another opens in its place (what played there stops).
fn clear_middle(app: &AppHandle, label: &str) {
    if let Some(view) = middle(app).filter(|v| v.label() != label) {
        if messages::of_view(view.label()).is_some() {
            let _ = aside(&view);
        } else {
            let _ = view.close();
        }
    }
}

/// A view of the web (the site's, or a service's): only web pages; a link for a new window opens in the middle
/// instead; its title and where it is, told to the app's page (with the view's name: only the one in the middle
/// is shown); downloads to the Downloads folder.
fn web_view(app: &AppHandle, label: &str, url: Url) -> WebviewBuilder<tauri::Wry> {
    let (titled, loaded, saved, popup) = (app.clone(), app.clone(), app.clone(), app.clone());
    let (name, name2) = (label.to_string(), label.to_string());
    #[allow(unused_mut)]
    let mut builder = WebviewBuilder::new(label, WebviewUrl::External(url))
        // Only web pages (and their blank and blob pages); a mailto: or an app's own link isn't followed here.
        .on_navigation(|u| matches!(u.scheme(), "http" | "https" | "about" | "blob"))
        // A link that would open a new window or tab: in the middle, in the site's view, as on a Chat Computer.
        .on_new_window(move |u, _features| {
            if matches!(u.scheme(), "http" | "https") {
                tell_main(&popup, serde_json::json!({ "open": u.as_str() }));
            }
            NewWindowResponse::Deny
        })
        .on_document_title_changed(move |_, title| tell_main(&titled, serde_json::json!({ "title": title, "view": name })))
        .on_page_load(move |_, load| {
            if let PageLoadEvent::Finished = load.event() {
                tell_main(&loaded, serde_json::json!({ "url": load.url().as_str(), "view": name2 }));
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
    builder
}

/// A site in the middle, at the frame the page drew for it (x, y, w, h: CSS pixels of the page, which are the
/// window's logical pixels). The same view each time, so sign-ins and the way back stay.
#[tauri::command]
fn show_site(app: AppHandle, webview: Webview, url: String, x: f64, y: f64, w: f64, h: f64) -> Result<(), String> {
    from_main(&webview)?;
    let url = web_url(&url)?;
    clear_middle(&app, SITE);
    set_middle(&app, SITE);
    if let Some(site) = app.get_webview(SITE) {
        site.navigate(url).map_err(err)?;
        place(&site, x, y, w, h)?;
        return site.show().map_err(err);
    }
    let builder = web_view(&app, SITE, url);
    webview
        .window()
        .add_child(builder, LogicalPosition::new(x.max(0.0), y.max(0.0)), LogicalSize::new(w.max(1.0), h.max(1.0)))
        .map_err(err)?;
    Ok(())
}

/// The frame moved or changed size (the window did, or the sidebars came in): what's in the middle moves with it.
#[tauri::command]
fn place_site(app: AppHandle, webview: Webview, x: f64, y: f64, w: f64, h: f64) -> Result<(), String> {
    from_main(&webview)?;
    match middle(&app) {
        Some(view) => place(&view, x, y, w, h),
        None => Ok(()),
    }
}

/// Out of sight (the conversation has the middle, or a sheet is over it): still there, still playing.
#[tauri::command]
fn hide_site(app: AppHandle, webview: Webview) -> Result<(), String> {
    from_main(&webview)?;
    match middle(&app) {
        Some(view) => view.hide().map_err(err),
        None => Ok(()),
    }
}

/// Back in sight, where it was.
#[tauri::command]
fn unhide_site(app: AppHandle, webview: Webview) -> Result<(), String> {
    from_main(&webview)?;
    match middle(&app) {
        Some(view) => view.show().map_err(err),
        None => Ok(()),
    }
}

/// Closed: the site's gone, and what played stops; a service's is put aside (it stays signed in, and read).
#[tauri::command]
fn close_site(app: AppHandle, webview: Webview) -> Result<(), String> {
    from_main(&webview)?;
    let view = middle(&app);
    set_middle(&app, "");
    match view {
        Some(view) if messages::of_view(view.label()).is_some() => aside(&view),
        Some(view) => view.close().map_err(err),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------------------- how you have it (ways.rs)
/// How you have Chat Computer now, and what's here for each way (for its page, start/).
#[tauri::command]
async fn way_status(app: AppHandle, webview: Webview) -> Result<serde_json::Value, String> {
    from_main(&webview)?;
    let data = app.path().app_local_data_dir().map_err(err)?;
    let way = *app.state::<Chosen>().0.lock().map_err(err)?;
    tauri::async_runtime::spawn_blocking(move || {
        serde_json::json!({
            "way": way.map(ways::Way::name),
            "setup": ways::setup_dir(&data).to_string_lossy(),
            "os": ways::os_check(),
        })
    })
    .await
    .map_err(err)
}

/// A way chosen, and done: kept, and what it needs (see ways.rs). Chosen again, it's done again (Chat Computer Setup
/// started again).
#[tauri::command]
async fn choose_way(app: AppHandle, webview: Webview, way: String) -> Result<serde_json::Value, String> {
    from_main(&webview)?;
    let way = ways::Way::parse(&way).ok_or("That isn't a way to have Chat Computer.")?;
    let data = app.path().app_local_data_dir().map_err(err)?;
    let config = app.path().app_config_dir().map_err(err)?;
    let setup = app.path().resource_dir().map_err(err)?.join("setup");
    let mut done = tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let mut done = serde_json::json!({ "way": way.name() });
        match way {
            ways::Way::App => {}
            ways::Way::Os => {
                let check = ways::os_check();
                if !check.can {
                    return Err(check.why);
                }
                let to = ways::setup_dir(&data);
                ways::put_setup(&setup, &to).map_err(|e| format!("Chat Computer Setup couldn't be put in its folder ({e})."))?;
                ways::run_setup(&to)?;
                done["folder"] = to.to_string_lossy().into();
            }
        }
        Ok(done)
    })
    .await
    .map_err(err)??;
    ways::keep(&config, way).map_err(|e| format!("The way chosen couldn't be kept ({e})."))?;
    *app.state::<Chosen>().0.lock().map_err(err)? = Some(way);
    if way.in_app() {
        start_engine(&app);
    }
    done["system"] = about_system().into();
    Ok(done)
}

/// Chat Computer Setup's folder, shown in File Explorer or Finder.
#[tauri::command]
fn show_folder(app: AppHandle, webview: Webview, which: String) -> Result<(), String> {
    from_main(&webview)?;
    let data = app.path().app_local_data_dir().map_err(err)?;
    let dir = match which.as_str() {
        "setup" => ways::setup_dir(&data),
        _ => return Err("That isn't one of Chat Computer's folders.".into()),
    };
    if !dir.is_dir() {
        return Err("That folder isn't there yet.".into());
    }
    ways::show_folder(&dir)
}

fn about_system() -> &'static str {
    if cfg!(target_os = "macos") { "mac" } else if cfg!(windows) { "windows" } else { "linux" }
}

/// What's playing in the middle (a video, music): played, paused, muted or not, its volume (0 to 100); and the
/// site scrolled (Page Up and Down, and the arrow keys, Home and End while the message box is empty). A fixed
/// script, run in the site's page: nothing the page sends is run there.
#[tauri::command]
fn site_media(app: AppHandle, webview: Webview, action: String, value: Option<f64>) -> Result<(), String> {
    from_main(&webview)?;
    let js = site_script(&action, value)?;
    if let Some(view) = middle(&app) {
        view.eval(js).map_err(err)?;
    }
    Ok(())
}

/// Where a service's view opens: its own page (or, only in a build for testing, with CHATCOMPUTER_SERVICES_AT, a
/// page of the tests' own on this computer, named after the service).
fn service_url(s: &messages::Service) -> Result<Url, String> {
    if let Some(base) = test_services() {
        return base.join(&format!("{}.html", s.id)).map_err(err);
    }
    s.url.parse().map_err(err)
}

/// The tests' own pages for the services: only in a debug build, and only on this computer.
fn test_services() -> Option<Url> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let base: Url = std::env::var("CHATCOMPUTER_SERVICES_AT").ok()?.parse().ok()?;
    (base.scheme() == "http" && matches!(base.host_str(), Some("127.0.0.1" | "localhost"))).then_some(base)
}

/// A service connected (Gmail, Outlook, Telegram, WhatsApp, WeChat, Discord): its own view, made if it isn't there
/// yet. With `show`, in the middle at the frame (x, y, w, h), to sign in or to read there; otherwise put aside,
/// where its reader reads it for Messages.
#[tauri::command]
fn connect_service(app: AppHandle, webview: Webview, service: String, show: bool, x: f64, y: f64, w: f64, h: f64)
    -> Result<(), String> {
    from_main(&webview)?;
    let s = messages::by_id(&service).ok_or("That isn't a service Chat Computer connects.")?;
    let label = messages::view_label(s);
    let view = match app.get_webview(&label) {
        Some(v) => v,
        None => {
            let builder = web_view(&app, &label, service_url(s)?).initialization_script(messages::READER);
            webview
                .window()
                .add_child(builder, LogicalPosition::new(x.max(0.0), y.max(0.0)), LogicalSize::new(w.max(1.0), h.max(1.0)))
                .map_err(err)?
        }
    };
    if show {
        clear_middle(&app, &label);
        set_middle(&app, &label);
        place(&view, x, y, w, h)?;
        view.show().map_err(err)
    } else if middle(&app).is_some_and(|m| m.label() == label) {
        Ok(())
    } else {
        aside(&view)
    }
}

/// A service no longer connected: its view closes (its sign-in stays with the app's web views, as a browser's
/// would: sign out on its page first to end it).
#[tauri::command]
fn forget_service(app: AppHandle, webview: Webview, service: String) -> Result<(), String> {
    from_main(&webview)?;
    let s = messages::by_id(&service).ok_or("That isn't a service Chat Computer connects.")?;
    if middle(&app).is_some_and(|m| m.label() == messages::view_label(s)) {
        set_middle(&app, "");
    }
    match app.get_webview(&messages::view_label(s)) {
        Some(v) => v.close().map_err(err),
        None => Ok(()),
    }
}

/// One of a service's conversations, opened in its view (a fixed script: only the conversation's name is in it).
#[tauri::command]
fn open_service_item(app: AppHandle, webview: Webview, service: String, item: String) -> Result<(), String> {
    from_main(&webview)?;
    let s = messages::by_id(&service).ok_or("That isn't a service Chat Computer connects.")?;
    let js = messages::open_script(&item).ok_or("That isn't one of its conversations.")?;
    match app.get_webview(&messages::view_label(s)) {
        Some(v) => v.eval(js).map_err(err),
        None => Err(format!("{} isn't connected.", s.label)),
    }
}

/// How the model is: answering (on which port of this computer), being got ready (how far), or not here.
#[tauri::command]
fn model_status(webview: Webview, engine: tauri::State<'_, Arc<engine::Engine>>) -> Result<engine::Status, String> {
    from_main(&webview)?;
    let s = engine.status.lock().map_err(err)?.clone();
    Ok(s)
}

/// Where the models go: the app's own folder (or, only in a build for testing, CHATCOMPUTER_MODELS_DIR), and the
/// catalogue the engine has built in (or, only for testing, CHATCOMPUTER_ENGINE_CATALOGUE's).
fn models_dir(app: &tauri::App) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if cfg!(debug_assertions) {
        if let Some(d) = std::env::var_os("CHATCOMPUTER_MODELS_DIR") {
            return Ok(PathBuf::from(d));
        }
    }
    Ok(app.path().app_local_data_dir()?.join("models"))
}

fn test_catalogue() -> Option<PathBuf> {
    std::env::var_os("CHATCOMPUTER_ENGINE_CATALOGUE").filter(|_| cfg!(debug_assertions)).map(PathBuf::from)
}

/// What a service's reader read on its page, for Messages: only from a service's own view, on its own pages.
#[tauri::command]
fn messages_found(app: AppHandle, webview: Webview, items: Vec<messages::Item>, unread: Option<u32>) -> Result<(), String> {
    let s = messages::of_view(webview.label()).ok_or("Only a connected service's own view hands over its messages.")?;
    let url = webview.url().map_err(err)?;
    let test = test_services().is_some_and(|b| same_origin(&url, &b) && url.path() == format!("/{}.html", s.id));
    if !messages::is_own_page(s, &url) && !test {
        return Err("Only the service's own pages hand over its messages.".into());
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    tell_main(&app, serde_json::json!({ "messages": {
        "service": s.id, "label": s.label, "items": messages::clean(items, now), "unread": unread.map(|n| n.min(100_000)),
    } }));
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

// ---------------------------------------------------------------------------- your ChatGPT plan (chatgpt.rs)
fn chatgpt_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|e| format!("The app's folder: {e}"))
}

/// Whether the app is signed in with ChatGPT, and as whom.
#[tauri::command]
fn chatgpt_status(app: AppHandle, webview: Webview) -> Result<chatgpt::Status, String> {
    from_main(&webview)?;
    Ok(chatgpt::status(&chatgpt_dir(&app)?))
}

/// Signs in with ChatGPT: OpenAI's page in the person's own browser, and back to the app; done when it's back.
#[tauri::command]
async fn chatgpt_sign_in(app: AppHandle, webview: Webview) -> Result<chatgpt::Status, String> {
    from_main(&webview)?;
    let dir = chatgpt_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        chatgpt::sign_in(&dir, &|url: &str| {
            // (Only OpenAI's own sign-in page is opened from here.)
            if !url.starts_with("https://auth.openai.com/") {
                return Err("That isn't ChatGPT's sign-in.".into());
            }
            open::that_detached(url).map_err(|e| format!("Your browser couldn't be opened ({e})."))
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn chatgpt_sign_out(app: AppHandle, webview: Webview) -> Result<chatgpt::Status, String> {
    from_main(&webview)?;
    let dir = chatgpt_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || chatgpt::sign_out(&dir)).await.map_err(|e| e.to_string())
}

/// Asks ChatGPT on your plan for the page ("responses": an answer, streamed to `events` as it comes; "models": the
/// models your plan has); returns OpenAI's status.
#[tauri::command]
async fn chatgpt_ask(app: AppHandle, webview: Webview, path: String, body: Option<String>, events: tauri::ipc::Channel<String>)
    -> Result<u16, String> {
    from_main(&webview)?;
    let dir = chatgpt_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        chatgpt::ask(&dir, &path, body.as_deref(), &mut |t: &str| {
            let _ = events.send(t.to_string());
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Who this computer says you are (your account's full name), for the greeting; which computer it is; and how you
/// have Chat Computer (the app, or your browser: then sites open in tabs).
#[tauri::command]
fn about(app: AppHandle, webview: Webview) -> Result<serde_json::Value, String> {
    from_main(&webview)?;
    let (real, user) = (whoami::realname(), whoami::username());
    let name = if real.trim().is_empty() || real.trim().eq_ignore_ascii_case(user.trim()) { String::new() } else { real };
    let way = *app.state::<Chosen>().0.lock().map_err(err)?;
    Ok(serde_json::json!({
        "name": name.trim(),
        "system": about_system(),
        "way": way.map(ways::Way::name),
    }))
}

/// The address of the app's own page (`page`: the conversation, index.html, or the one that asks how you want Chat
/// Computer, start/index.html): Tauri's own scheme; on a Mac, a server only this computer can reach, at 127.0.0.1 (a
/// Mac's web view doesn't treat Tauri's scheme as secure, and without that there's no WebGPU for Heart, no
/// microphone and no browser cache). The same port each time where it's free, so what the page keeps (your name,
/// your Actions, Heart) is there next time.
fn home_url(app: &tauri::App, page: &str) -> Result<WebviewUrl, Box<dyn std::error::Error>> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        if cfg!(target_os = "macos") || std::env::var_os("CHATCOMPUTER_LOOPBACK").is_some() {
            let port = loopback::serve(app.asset_resolver())?;
            return Ok(WebviewUrl::External(format!("http://127.0.0.1:{port}/{page}").parse()?));
        }
    }
    let _ = app;
    Ok(WebviewUrl::App(page.into()))
}

/// The page the app opens on: the conversation, when the way is the app; otherwise the one that asks how you want
/// Chat Computer (the first time), or shows the own system's steps, to change it there.
fn first_page(way: Option<ways::Way>) -> &'static str {
    if way.is_some_and(ways::Way::in_app) { "index.html" } else { "start/index.html" }
}

/// The screen's dark (theme.css's ground): the window and the page are that while the page loads, never white.
const SCREEN: tauri::window::Color = tauri::window::Color(0x15, 0x15, 0x15, 255);

pub fn run() {
    let engine = Arc::new(engine::Engine::default());
    let (starting, stopping) = (engine.clone(), engine.clone());
    tauri::Builder::default()
        .manage(Found::default())
        .manage(Middle::default())
        .manage(Chosen::default())
        .manage(EngineStart::default())
        .manage(engine)
        .invoke_handler(tauri::generate_handler![
            show_site, place_site, hide_site, unhide_site, close_site, site_media, search_files, open_file, about,
            connect_service, forget_service, open_service_item, model_status, messages_found, chatgpt_status,
            chatgpt_sign_in, chatgpt_sign_out, chatgpt_ask, way_status, choose_way, show_folder
        ])
        .setup(move |app| {
            // (A build for testing, pointed at the tests' own pages for the services: those may hand over what's
            // read there too.)
            if let Some(base) = test_services() {
                let mut cap = tauri::ipc::CapabilityBuilder::new("messages-test")
                    .remote(format!("{}*", base.as_str()))
                    .local(false)
                    .permission("allow-messages-found");
                for s in &messages::SERVICES {
                    cap = cap.webview(messages::view_label(s));
                }
                app.add_capability(cap)?;
            }
            // How you have Chat Computer: as chosen before; or, where the app was used before there was a choice
            // (its models' folder is there), the app, as it was; or not yet chosen (its page asks).
            let config = app.path().app_config_dir()?;
            let mut way = ways::kept(&config);
            if way.is_none() && app.path().app_local_data_dir()?.join("models").is_dir() {
                way = Some(ways::Way::App);
                let _ = ways::keep(&config, ways::Way::App);
            }
            *app.state::<Chosen>().0.lock().map_err(|e| e.to_string())? = way;
            let url = home_url(app, first_page(way))?;
            // The whole screen to start with, as a Chat Computer has (and room for both sidebars).
            let window = tauri::window::WindowBuilder::new(app, WINDOW)
                .title("Chat Computer")
                .background_color(SCREEN)
                .inner_size(1280.0, 820.0)
                .min_inner_size(420.0, 560.0)
                .center()
                .maximized(true)
                .build()?;
            let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
            let home = match &url {
                WebviewUrl::External(u) => u.join("/")?,
                _ => {
                    if cfg!(windows) { "http://tauri.localhost/" } else { "tauri://localhost/" }.parse()?
                }
            };
            app.manage(Home(home.clone()));
            // The model, on this computer: served (and got ready, where it isn't yet), for the page, which hears how
            // it goes (chatcomputerApp({model})). The page's own origin may use it (on a Mac, the app's server's).
            // Started now when the way is the app; otherwise once it's chosen.
            let dir = models_dir(app)?;
            let origins = if home.scheme() == "http" { vec![home.origin().ascii_serialization()] } else { Vec::new() };
            let told = app.handle().clone();
            let tell: Arc<dyn Fn(&engine::Status) + Send + Sync> =
                Arc::new(move |s| tell_main(&told, serde_json::json!({ "model": s })));
            *app.state::<EngineStart>().0.lock().map_err(|e| e.to_string())? = Some(Box::new(move || {
                std::thread::spawn(move || starting.run(dir, test_catalogue(), origins, tell));
            }));
            if way.is_some_and(ways::Way::in_app) {
                start_engine(app.handle());
            }
            let (back, opener, guard) = (home.clone(), app.handle().clone(), app.handle().clone());
            let main = WebviewBuilder::new(MAIN, url)
                .background_color(SCREEN)
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
                    if let PageLoadEvent::Started = load.event() {
                        let home = &guard.state::<Home>().0;
                        // Elsewhere, the page itself leaving: back to it, and the address opened in the middle.
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
        .build(tauri::generate_context!())
        .expect("Chat Computer couldn't start")
        .run(move |app, event| {
            let _ = app;
            // (The model's engine ends with the app.)
            if let tauri::RunEvent::Exit = event {
                stopping.stop();
            }
        });
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
    fn the_app_opens_on_the_conversation_once_it_s_the_app() {
        assert_eq!(first_page(None), "start/index.html");
        assert_eq!(first_page(Some(ways::Way::Os)), "start/index.html");
        assert_eq!(first_page(Some(ways::Way::App)), "index.html");
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

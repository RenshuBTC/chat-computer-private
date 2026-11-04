//! How you have Chat Computer: the first thing the app asks, on a page of its own (start/, bundled with the app), and
//! what it then does. There are four ways, and the one chosen is kept in the app's own folder (way.json), to be
//! changed there again later ("/" in Chat Computer lists it; the app opens on that page while the way is one that
//! isn't the app):
//!
//! - Chrome extension: the extension (apps/chrome, which comes with the app) is put in a folder of the app's own that
//!   stays, Chrome is opened (at chatcomputer.ai/get/chrome.html, the same steps, with the folder), and the page shows
//!   the steps Chrome asks for (Developer mode, Load unpacked, that folder: its path is copied). Chrome only installs
//!   extensions from its Web Store by itself, and doesn't let another program open its extensions page (it drops a
//!   chrome:// address given on its command line), so those steps are the person's.
//! - Browser: the app as a web browser (the conversation, and sites in tabs above it: host.js), registered as one
//!   with the system so links from other apps open in it (on Windows, for this account: StartMenuInternet,
//!   Capabilities, RegisteredApplications and a URL ProgID; on a Mac, Info.plist's URL types), with an offer to be
//!   the default browser (Windows doesn't let an app make itself that: Settings opens at it; a Mac asks itself).
//! - App: the app as it was, a window of its own.
//! - Its own system: Chat Computer Setup (which comes with the app: setup/) is copied to a folder of its own and run
//!   from there: on a PC, "Chat Computer Setup.cmd", which asks Windows for permission and shows its plan before
//!   changing anything; on a Mac with M1, M2 or M3, mac/setup.sh, in Terminal.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Way {
    Chrome,
    Browser,
    App,
    Os,
}

impl Way {
    pub fn parse(s: &str) -> Option<Way> {
        match s {
            "chrome" => Some(Way::Chrome),
            "browser" => Some(Way::Browser),
            "app" => Some(Way::App),
            "os" => Some(Way::Os),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Way::Chrome => "chrome",
            Way::Browser => "browser",
            Way::App => "app",
            Way::Os => "os",
        }
    }

    /// Whether the app opens on the conversation (as the app, or as a browser), rather than on the page that asks.
    pub fn in_app(self) -> bool {
        matches!(self, Way::App | Way::Browser)
    }
}

/// The way kept in the app's folder, if one was chosen.
pub fn kept(dir: &Path) -> Option<Way> {
    let text = std::fs::read_to_string(dir.join("way.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    Way::parse(v.get("way")?.as_str()?)
}

pub fn keep(dir: &Path, way: Way) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let new = dir.join("way.json.new");
    std::fs::write(&new, serde_json::json!({ "way": way.name() }).to_string())?;
    std::fs::rename(new, dir.join("way.json"))
}

// ---------------------------------------------------------------------------------------- the Chrome extension
/// The extension's files, as apps/chrome has them (scripts/build_apps.py zips the same ones for chatcomputer.ai/get).
pub const EXTENSION: [(&str, &str); 5] = [
    ("manifest.json", include_str!("../../../chrome/manifest.json")),
    ("background.js", include_str!("../../../chrome/background.js")),
    ("panel.html", include_str!("../../../chrome/panel.html")),
    ("panel.css", include_str!("../../../chrome/panel.css")),
    ("panel.js", include_str!("../../../chrome/panel.js")),
];

/// Where the extension stays: a folder of the app's own (Chrome loads an unpacked extension from where it is, each
/// time it starts, so the folder mustn't move).
pub fn extension_dir(app_dir: &Path) -> PathBuf {
    app_dir.join("Chrome extension")
}

/// The extension's files put in its folder (as this version of the app has them).
pub fn put_extension(app_dir: &Path) -> std::io::Result<PathBuf> {
    let to = extension_dir(app_dir);
    std::fs::create_dir_all(&to)?;
    for (name, text) in EXTENSION {
        std::fs::write(to.join(name), text)?;
    }
    Ok(to)
}

/// Where Chrome is on this computer, if it's there.
pub fn chrome() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        // Where Windows says it is (App Paths), for this account or for everyone; then where it installs itself.
        for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            let key = RegKey::predef(root).open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe");
            if let Ok(p) = key.and_then(|k| k.get_value::<String, _>("")) {
                let p = PathBuf::from(p.trim().trim_matches('"'));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|base| PathBuf::from(base).join(r"Google\Chrome\Application\chrome.exe"))
            .find(|p| p.is_file())
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Applications"));
        [Some(PathBuf::from("/Applications")), home]
            .into_iter()
            .flatten()
            .map(|d| d.join("Google Chrome.app"))
            .find(|p| p.is_dir())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .flat_map(|d| ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"].map(|n| d.join(n)))
            .find(|p| p.is_file())
    }
}

/// The page Chrome is opened at: the steps it asks for (chatcomputer.ai/get/chrome.html), with the extension's folder
/// after #folder= (the part of an address after # stays in the browser: it isn't sent to the site).
pub fn chrome_steps(folder: &Path) -> String {
    let mut path = String::new();
    for b in folder.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            path.push(b as char);
        } else {
            path.push_str(&format!("%{b:02X}"));
        }
    }
    format!("https://chatcomputer.ai/get/chrome.html#folder={path}")
}

/// Chrome, opened at a web page (in a window of its own: it doesn't let another program open chrome://extensions,
/// whatever it's asked).
pub fn open_chrome(chrome: &Path, page: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut c = {
        let mut c = Command::new("/usr/bin/open");
        c.arg("-a").arg(chrome).arg(page);
        c
    };
    #[cfg(not(target_os = "macos"))]
    let mut c = {
        let mut c = Command::new(chrome);
        c.arg("--new-window").arg(page);
        c
    };
    c.spawn().map(|_| ())
}

/// A folder shown in File Explorer or Finder (the extension's, to drag onto Chrome's page or see where it is).
pub fn show_folder(dir: &Path) -> Result<(), String> {
    open::that_detached(dir).map_err(|e| format!("The folder couldn't be shown ({e})."))
}

// ---------------------------------------------------------------------------------------- the app as a browser
/// Its name in Windows' lists of browsers (Default apps, the Start menu's Internet entry, RegisteredApplications).
#[cfg_attr(not(windows), allow(dead_code))]
pub const BROWSER_NAME: &str = "Chat Computer";
/// The kind of thing the web addresses it opens are, to Windows.
#[cfg_attr(not(windows), allow(dead_code))]
pub const PROGID: &str = "ChatComputerURL";

/// What registers the app as a web browser for this Windows account (all under HKEY_CURRENT_USER, so no
/// administrator is needed): (key, value's name ("" for the key's own), value). `exe` is the app's program.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn browser_keys(exe: &str) -> Vec<(String, String, String)> {
    let client = format!(r"Software\Clients\StartMenuInternet\{BROWSER_NAME}");
    let caps = format!(r"{client}\Capabilities");
    let class = format!(r"Software\Classes\{PROGID}");
    let icon = format!("{exe},0");
    let k = |key: &str, name: &str, value: &str| (key.to_string(), name.to_string(), value.to_string());
    vec![
        k(&client, "", BROWSER_NAME),
        k(&format!(r"{client}\DefaultIcon"), "", &icon),
        k(&format!(r"{client}\shell\open\command"), "", &format!("\"{exe}\"")),
        k(&caps, "ApplicationName", BROWSER_NAME),
        k(&caps, "ApplicationDescription", "Chat Computer as your web browser: the sites you open, in tabs, and a conversation that does things for you."),
        k(&caps, "ApplicationIcon", &icon),
        k(&format!(r"{caps}\StartMenu"), "StartMenuInternet", BROWSER_NAME),
        k(&format!(r"{caps}\URLAssociations"), "http", PROGID),
        k(&format!(r"{caps}\URLAssociations"), "https", PROGID),
        k(r"Software\RegisteredApplications", BROWSER_NAME, &caps),
        k(&class, "", "Chat Computer web page"),
        k(&format!(r"{class}\DefaultIcon"), "", &icon),
        k(&format!(r"{class}\Application"), "ApplicationName", BROWSER_NAME),
        k(&format!(r"{class}\Application"), "ApplicationIcon", &icon),
        k(&format!(r"{class}\Application"), "ApplicationCompany", "Chat Computer"),
        // A link from another app: the app started with it (or, when it's running, the running one told: lib.rs).
        k(&format!(r"{class}\shell\open\command"), "", &format!("\"{exe}\" --url \"%1\"")),
    ]
}

/// The keys browser_keys writes, to take out again (the uninstaller takes them out too: nsis/hooks.nsh).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn browser_trees() -> [String; 2] {
    [format!(r"Software\Clients\StartMenuInternet\{BROWSER_NAME}"), format!(r"Software\Classes\{PROGID}")]
}

#[cfg(windows)]
mod shell {
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHChangeNotify(event: i32, flags: u32, item1: *const core::ffi::c_void, item2: *const core::ffi::c_void);
    }
    /// Windows told that what opens what has changed (Settings' Default apps reads the browsers again).
    pub fn associations_changed() {
        const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, 0, std::ptr::null(), std::ptr::null()) }
    }
}

/// The app registered as a web browser (Windows, for this account; elsewhere there's nothing to do: a Mac reads the
/// app's Info.plist).
pub fn register_browser() -> Result<(), String> {
    #[cfg(windows)]
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for (key, name, value) in browser_keys(&exe.to_string_lossy()) {
            let (k, _) = hkcu.create_subkey(&key).map_err(|e| format!("Windows didn't let the app list itself as a browser ({e})."))?;
            k.set_value(&name, &value).map_err(|e| format!("Windows didn't let the app list itself as a browser ({e})."))?;
        }
        shell::associations_changed();
    }
    Ok(())
}

/// Taken out of Windows' lists of browsers again (the way changed to another).
pub fn unregister_browser() {
    #[cfg(windows)]
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let mut changed = false;
        for tree in browser_trees() {
            changed |= hkcu.delete_subkey_all(&tree).is_ok();
        }
        if let Ok(apps) = hkcu.open_subkey_with_flags(r"Software\RegisteredApplications", winreg::enums::KEY_SET_VALUE) {
            changed |= apps.delete_value(BROWSER_NAME).is_ok();
        }
        if changed {
            shell::associations_changed();
        }
    }
}

/// Whether the app opens web links now (the default browser), where that can be told.
pub fn is_default_browser(bundle_id: &str) -> Option<bool> {
    #[cfg(windows)]
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let _ = bundle_id;
        let choice = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice")
            .and_then(|k| k.get_value::<String, _>("ProgId"));
        Some(choice.is_ok_and(|p| p == PROGID))
    }
    #[cfg(target_os = "macos")]
    {
        Some(mac::default_for("https").is_some_and(|b| b.eq_ignore_ascii_case(bundle_id)))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = bundle_id;
        None
    }
}

/// Asked to be the default browser: on Windows, Settings' Default apps opened at Chat Computer (Windows doesn't let an
/// app do it itself: the person chooses there); on a Mac, macOS asks the person itself.
pub fn ask_default_browser(bundle_id: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = bundle_id;
        open::that_detached("ms-settings:defaultapps?registeredAppUser=Chat%20Computer")
            .map_err(|e| format!("Settings couldn't be opened ({e})."))
    }
    #[cfg(target_os = "macos")]
    {
        mac::set_default(bundle_id)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = bundle_id;
        Err("On this computer, choose the default browser in its own settings.".into())
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{CStr, CString, c_char, c_void};
    type CFStringRef = *const c_void;
    const UTF8: u32 = 0x0800_0100;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(alloc: *const c_void, s: *const c_char, encoding: u32) -> CFStringRef;
        fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFRelease(cf: *const c_void);
    }
    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSSetDefaultHandlerForURLScheme(scheme: CFStringRef, bundle: CFStringRef) -> i32;
        fn LSCopyDefaultHandlerForURLScheme(scheme: CFStringRef) -> CFStringRef;
    }
    fn cf(s: &str) -> CFStringRef {
        let c = CString::new(s).unwrap_or_default();
        unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) }
    }
    /// The app as the handler of http and https: macOS then asks the person whether to change the default browser.
    pub fn set_default(bundle_id: &str) -> Result<(), String> {
        for scheme in ["http", "https"] {
            let (s, b) = (cf(scheme), cf(bundle_id));
            let status = unsafe { LSSetDefaultHandlerForURLScheme(s, b) };
            unsafe {
                CFRelease(s);
                CFRelease(b);
            }
            if status != 0 {
                return Err(format!("macOS didn't make it the default browser (error {status})."));
            }
        }
        Ok(())
    }
    pub fn default_for(scheme: &str) -> Option<String> {
        let s = cf(scheme);
        let h = unsafe { LSCopyDefaultHandlerForURLScheme(s) };
        unsafe { CFRelease(s) };
        if h.is_null() {
            return None;
        }
        let mut buf = [0 as c_char; 512];
        let ok = unsafe { CFStringGetCString(h, buf.as_mut_ptr(), buf.len() as isize, UTF8) };
        unsafe { CFRelease(h) };
        (ok != 0).then(|| unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned())
    }
}

/// The web addresses among a program's arguments (a link from another app: Windows starts the app with it, after
/// --url), each at most once, in order. Nothing else is taken from them.
pub fn urls_in(args: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in args {
        let a = a.trim();
        let Ok(u) = tauri::Url::parse(a) else { continue };
        if matches!(u.scheme(), "http" | "https") && u.host_str().is_some() && !out.iter().any(|o| o == u.as_str()) {
            out.push(u.to_string());
        }
    }
    out
}

// ---------------------------------------------------------------------------------------- its own system
/// Whether this computer can have Chat Computer's own system, and if not, why (in the words the page shows).
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct OsCheck {
    pub can: bool,
    pub why: String,
    /// What the computer is: "windows", "mac" or "linux"; on a Mac, its chip too.
    pub system: String,
    pub chip: String,
}

/// A Mac's chip, as its own setup (mac/setup.sh) judges it: Chat Computer's own system starts only on Apple silicon
/// M1, M2 and M3 (not M3 Ultra): it's built on Fedora Asahi Remix, and Asahi Linux doesn't support newer chips yet.
pub fn mac_chip(brand: &str, arm64: bool) -> Result<(), String> {
    let b = brand.trim();
    let is = |chip: &str| b == chip || b.starts_with(&format!("{chip} "));
    if !arm64 {
        return Err("This Mac has an Intel processor: Chat Computer's own system starts only on Macs with Apple silicon \
                    (M1, M2 or M3)."
            .into());
    }
    if is("Apple M3 Ultra") {
        return Err(format!("This Mac has an {b}: Chat Computer's own system works on M1, M2 and M3 Macs, but not yet on \
                            the M3 Ultra (Asahi Linux, which it's built on, doesn't support it yet)."));
    }
    if is("Apple M1") || is("Apple M2") || is("Apple M3") {
        return Ok(());
    }
    let chip = if b.is_empty() { "Apple chip Chat Computer doesn't know" } else { b };
    Err(format!("This Mac has an {chip}: Chat Computer's own system starts only on Macs with M1, M2 or M3 (Asahi Linux, \
                 which it's built on, doesn't support newer chips yet)."))
}

fn sysctl(name: &str) -> String {
    Command::new("/usr/sbin/sysctl")
        .args(["-n", name])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

pub fn os_check() -> OsCheck {
    if cfg!(windows) {
        // (Its setup checks the rest, and says what it finds before changing anything: UEFI, the room, the internet.)
        return OsCheck { can: true, why: String::new(), system: "windows".into(), chip: String::new() };
    }
    if cfg!(target_os = "macos") {
        let chip = sysctl("machdep.cpu.brand_string");
        let why = mac_chip(&chip, sysctl("hw.optional.arm64") == "1").err().unwrap_or_default();
        return OsCheck { can: why.is_empty(), why, system: "mac".into(), chip };
    }
    OsCheck {
        can: false,
        why: "Chat Computer's own system installs from Windows or from macOS: run this app there.".into(),
        system: "linux".into(),
        chip: String::new(),
    }
}

/// Where Chat Computer Setup is put to run from: a folder of the app's own, which stays (on a PC, the installed
/// Chat Computer looks there for its updates too).
pub fn setup_dir(app_dir: &Path) -> PathBuf {
    app_dir.join("Chat Computer Setup")
}

/// Chat Computer Setup, as the app brings it (its resources' setup/), copied to its folder (files from before that
/// aren't in this version are left; the same ones are replaced).
pub fn put_setup(from: &Path, to: &Path) -> std::io::Result<()> {
    if !from.join("README.md").is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "Chat Computer Setup isn't with this app."));
    }
    copy_tree(from, to)
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let dest = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

/// What starts Chat Computer Setup on a Mac: opened by Terminal (a file this app writes, so macOS doesn't hold it back
/// as something downloaded).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const MAC_LAUNCHER: &str = "#!/bin/bash\n\
# Chat Computer Setup for Mac, started by the Chat Computer app (its own system, next to macOS).\n\
cd \"$(dirname \"$0\")\" || exit 1\n\
exec bash mac/setup.sh\n";

/// Chat Computer Setup started from its folder: on a PC its .cmd (it asks Windows for permission itself, and shows its
/// plan before changing anything); on a Mac, mac/setup.sh in Terminal.
pub fn run_setup(dir: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let cmd = dir.join("windows").join("Chat Computer Setup.cmd");
        if !cmd.is_file() {
            return Err("Chat Computer Setup isn't in its folder.".into());
        }
        open::that_detached(&cmd).map_err(|e| format!("Chat Computer Setup couldn't start ({e})."))
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::PermissionsExt;
        if !dir.join("mac").join("setup.sh").is_file() {
            return Err("Chat Computer Setup isn't in its folder.".into());
        }
        let launcher = dir.join("Chat Computer Setup.command");
        std::fs::write(&launcher, MAC_LAUNCHER).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        Command::new("/usr/bin/open")
            .args(["-a", "Terminal"])
            .arg(&launcher)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Terminal couldn't be opened ({e})."))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = dir;
        Err(os_check().why)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_way_is_kept_and_read_back() {
        let dir = std::env::temp_dir().join(format!("cc-ways-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(kept(&dir), None);
        for way in [Way::Chrome, Way::Browser, Way::App, Way::Os] {
            keep(&dir, way).unwrap();
            assert_eq!(kept(&dir), Some(way));
            assert_eq!(Way::parse(way.name()), Some(way));
        }
        std::fs::write(dir.join("way.json"), "{\"way\":\"desktop\"}").unwrap();
        assert_eq!(kept(&dir), None);
        assert_eq!(Way::parse("Browser"), None);
        assert!(Way::App.in_app() && Way::Browser.in_app() && !Way::Chrome.in_app() && !Way::Os.in_app());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_extension_is_put_where_it_stays() {
        let dir = std::env::temp_dir().join(format!("cc-ext-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let to = put_extension(&dir).unwrap();
        assert_eq!(to, dir.join("Chrome extension"));
        let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(to.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["name"], "Chat Computer");
        assert_eq!(manifest["side_panel"]["default_path"], "panel.html");
        for (name, text) in EXTENSION {
            assert_eq!(std::fs::read_to_string(to.join(name)).unwrap(), text);
        }
        put_extension(&dir).unwrap();               // (again: the same folder, the same files)
        assert_eq!(std::fs::read_dir(&to).unwrap().count(), EXTENSION.len());
        assert_eq!(chrome_steps(Path::new(r"C:\Users\Ren Shu\AppData\Local\ai.chatcomputer.desktop\Chrome extension")),
                   "https://chatcomputer.ai/get/chrome.html#folder=C%3A%5CUsers%5CRen%20Shu%5CAppData%5CLocal%5Cai.chatcomputer.desktop%5CChrome%20extension");
        assert_eq!(chrome_steps(Path::new("/Users/zoë/Library/x")),
                   "https://chatcomputer.ai/get/chrome.html#folder=%2FUsers%2Fzo%C3%AB%2FLibrary%2Fx");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_browser_is_registered_for_this_account_only() {
        let keys = browser_keys(r"C:\Users\Ren\AppData\Local\Chat Computer\chat-computer.exe");
        let get = |key: &str, name: &str| keys.iter().find(|(k, n, _)| k == key && n == name).map(|(_, _, v)| v.as_str());
        let client = r"Software\Clients\StartMenuInternet\Chat Computer";
        assert_eq!(get(client, ""), Some("Chat Computer"));
        assert_eq!(get(&format!(r"{client}\Capabilities\URLAssociations"), "http"), Some("ChatComputerURL"));
        assert_eq!(get(&format!(r"{client}\Capabilities\URLAssociations"), "https"), Some("ChatComputerURL"));
        assert_eq!(get(&format!(r"{client}\Capabilities\StartMenu"), "StartMenuInternet"), Some("Chat Computer"));
        assert_eq!(get(r"Software\RegisteredApplications", "Chat Computer"), Some(r"Software\Clients\StartMenuInternet\Chat Computer\Capabilities"));
        assert_eq!(get(r"Software\Classes\ChatComputerURL\shell\open\command", ""),
                   Some(r#""C:\Users\Ren\AppData\Local\Chat Computer\chat-computer.exe" --url "%1""#));
        assert_eq!(get(&format!(r"{client}\shell\open\command"), ""), Some(r#""C:\Users\Ren\AppData\Local\Chat Computer\chat-computer.exe""#));
        // Every key is the account's own (written under HKEY_CURRENT_USER), and in one of the trees taken out again.
        for (k, _, _) in &keys {
            assert!(k.starts_with(r"Software\"), "{k}");
            assert!(k == r"Software\RegisteredApplications" || browser_trees().iter().any(|t| k.starts_with(t.as_str())), "{k}");
        }
    }

    #[test]
    fn only_web_addresses_come_from_the_command_line() {
        let args: Vec<String> = ["chat-computer.exe", "--url", "https://www.bbc.co.uk/news", "file:///C:/Windows/win.ini",
                                 "javascript:alert(1)", "https://www.bbc.co.uk/news", " http://example.com/a?b=c ", "--flag", "notes.md"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(urls_in(&args), vec!["https://www.bbc.co.uk/news".to_string(), "http://example.com/a?b=c".to_string()]);
        assert!(urls_in(&["https://".to_string(), "http:".to_string(), "--url".to_string()]).is_empty());
    }

    #[test]
    fn only_macs_with_m1_m2_or_m3_have_the_own_system() {
        for ok in ["Apple M1", "Apple M1 Pro", "Apple M2 Max", "Apple M2 Ultra", "Apple M3", "Apple M3 Pro", "Apple M3 Max"] {
            assert_eq!(mac_chip(ok, true), Ok(()), "{ok}");
        }
        assert!(mac_chip("Apple M3 Ultra", true).unwrap_err().contains("not yet on the M3 Ultra"));
        assert!(mac_chip("Apple M4 Pro", true).unwrap_err().contains("This Mac has an Apple M4 Pro"));
        assert!(mac_chip("Apple M10", true).is_err());
        assert!(mac_chip("Intel(R) Core(TM) i7-9750H", false).unwrap_err().contains("Intel processor"));
        assert!(mac_chip("", true).unwrap_err().contains("Apple chip Chat Computer doesn't know"));
    }

    #[test]
    fn setup_is_copied_whole_to_its_folder() {
        let base = std::env::temp_dir().join(format!("cc-setup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (from, to) = (base.join("from"), base.join("to"));
        assert!(put_setup(&from, &to).is_err());                      // (not with this app: nothing to copy)
        std::fs::create_dir_all(from.join("windows/lib")).unwrap();
        std::fs::write(from.join("README.md"), "Chat Computer").unwrap();
        std::fs::write(from.join("windows/Chat Computer Setup.cmd"), "@echo off\r\n").unwrap();
        std::fs::write(from.join("windows/lib/Flow.ps1"), "# flow").unwrap();
        put_setup(&from, &to).unwrap();
        assert_eq!(std::fs::read(to.join("windows/Chat Computer Setup.cmd")).unwrap(), b"@echo off\r\n");
        assert_eq!(std::fs::read_to_string(to.join("windows/lib/Flow.ps1")).unwrap(), "# flow");
        assert_eq!(setup_dir(&base), base.join("Chat Computer Setup"));
        assert!(MAC_LAUNCHER.starts_with("#!/bin/bash\n") && MAC_LAUNCHER.ends_with("exec bash mac/setup.sh\n"));
        let _ = std::fs::remove_dir_all(&base);
    }
}

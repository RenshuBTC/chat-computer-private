//! How you have Chat Computer: the first thing the app asks, on a page of its own (start/, bundled with the app), and
//! what it then does. There are two ways, and the one chosen is kept in the app's own folder (way.json), to be changed
//! there again later ("/" in Chat Computer lists it; the app opens on that page while the way is its own system):
//!
//! - App: Chat Computer in a window of its own.
//! - Its own system: on a PC, Chat Computer's own operating system (os/), put next to Windows by "Chat Computer Own
//!   System.cmd" (which comes with the app: setup/), which asks Windows for permission and once whether to go ahead;
//!   on a Mac with M1, M2 or M3, mac/setup.sh, in Terminal.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Way {
    App,
    Os,
}

impl Way {
    pub fn parse(s: &str) -> Option<Way> {
        match s {
            "app" => Some(Way::App),
            "os" => Some(Way::Os),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Way::App => "app",
            Way::Os => "os",
        }
    }

    /// Whether the app opens on the conversation (as the app), rather than on the page that asks.
    pub fn in_app(self) -> bool {
        matches!(self, Way::App)
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

/// A folder shown in File Explorer or Finder (Chat Computer Setup's).
pub fn show_folder(dir: &Path) -> Result<(), String> {
    open::that_detached(dir).map_err(|e| format!("The folder couldn't be shown ({e})."))
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
        // (Its setup checks the rest, and says what it finds before changing anything: UEFI, the start-up partition.)
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

/// Chat Computer Setup started from its folder: on a PC, Chat Computer's own system's .cmd (it asks Windows for
/// permission itself, then once whether to go ahead); on a Mac, mac/setup.sh in Terminal.
pub fn run_setup(dir: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let cmd = dir.join("windows").join("Chat Computer Own System.cmd");
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
        for way in [Way::App, Way::Os] {
            keep(&dir, way).unwrap();
            assert_eq!(kept(&dir), Some(way));
            assert_eq!(Way::parse(way.name()), Some(way));
        }
        std::fs::write(dir.join("way.json"), "{\"way\":\"desktop\"}").unwrap();
        assert_eq!(kept(&dir), None);
        assert_eq!(Way::parse("App"), None);
        assert_eq!(Way::parse("browser"), None);
        assert!(Way::App.in_app() && !Way::Os.in_app());
        let _ = std::fs::remove_dir_all(&dir);
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

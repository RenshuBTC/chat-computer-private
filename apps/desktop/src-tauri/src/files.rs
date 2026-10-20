//! "/" in the message box: this computer's files, found by name as the words after it are typed (as on a Chat
//! Computer, where an index does it: here the usual folders are looked through each time, quickly). Programs'
//! own folders, hidden ones and the bin are passed over. And which of them the app will open: documents, pictures,
//! music, videos and folders, never programs or scripts.
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::Serialize;

/// A file found: its name, where it is ("~/Documents/Taxes"), what kind of thing it is, and when it last changed
/// (seconds since 1970; the page says it as a person would).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Hit {
    pub name: String,
    pub path: String,
    #[serde(rename = "where")]
    pub place: String,
    pub kind: &'static str,
    pub ts: f64,
}

/// As chatcomputer/files_index.py has them.
const KINDS: &[(&str, &[&str])] = &[
    ("document", &["pdf", "doc", "docx", "odt", "rtf", "txt", "md", "pages", "tex"]),
    ("spreadsheet", &["xls", "xlsx", "xlsm", "csv", "tsv", "ods", "numbers"]),
    ("presentation", &["ppt", "pptx", "odp", "key"]),
    ("image", &["jpg", "jpeg", "png", "gif", "webp", "heic", "bmp", "tif", "tiff", "svg", "raw", "cr2", "nef", "dng"]),
    ("video", &["mp4", "mov", "mkv", "avi", "webm", "m4v", "wmv", "flv"]),
    ("audio", &["mp3", "m4a", "wav", "flac", "aac", "ogg", "opus", "wma"]),
    ("archive", &["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz"]),
    ("code", &["py", "js", "ts", "tsx", "jsx", "html", "css", "json", "java", "c", "cpp", "h", "rs", "go", "ipynb", "sh",
               "ps1", "sql", "rb", "php", "yaml", "yml", "toml", "xml"]),
    ("ebook", &["epub", "mobi", "azw3"]),
    ("program", &["exe", "msi", "appimage", "deb", "dmg", "pkg"]),
];

/// What opens from here, with its own app: things to read, look at or listen to. Not programs, scripts, shortcuts
/// or documents with macros (xlsm, docm), which can run code.
const OPENABLE: &[&str] = &[
    "pdf", "doc", "docx", "odt", "rtf", "txt", "md", "pages", "tex", "xls", "xlsx", "csv", "tsv", "ods", "numbers", "ppt",
    "pptx", "odp", "key", "jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp", "tif", "tiff", "svg", "raw", "cr2",
    "nef", "dng", "mp4", "mov", "mkv", "avi", "webm", "m4v", "wmv", "mp3", "m4a", "wav", "flac", "aac", "ogg", "opus",
    "wma", "zip", "epub", "json", "xml", "log",
];

/// Folders only programs keep things in, wherever they are; and ones that are really one thing (an app, a Photos
/// library), not looked into.
const SKIP: &[&str] = &[
    "node_modules", "__pycache__", "site-packages", "venv", "$RECYCLE.BIN", "System Volume Information", "AppData",
    "Application Data", "Local Settings", "OneDriveTemp", "Library", "Temp", "tmp",
];
const ONE_THING: &[&str] = &["app", "photoslibrary", "musiclibrary", "tvlibrary", "imovielibrary", "fcpbundle", "bundle",
                             "framework", "plugin", "pkg"];

const MAX_DEPTH: usize = 7;
const MAX_LOOKED_AT: usize = 80_000;
const MAX_TIME: Duration = Duration::from_millis(1500);

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn kind_of(p: &Path, dir: bool) -> &'static str {
    if dir {
        return "folder";
    }
    let e = ext(p);
    KINDS.iter().find(|(_, exts)| exts.contains(&e.as_str())).map(|(k, _)| *k).unwrap_or("other")
}

pub fn openable(p: &Path) -> bool {
    p.is_dir() && !ONE_THING.contains(&ext(p).as_str()) || p.is_file() && OPENABLE.contains(&ext(p).as_str())
}

/// Where a folder is, as a person would say it: from the home folder ("~/Documents/Taxes").
fn friendly(dir: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| dir.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => dir.display().to_string(),
    }
}

/// The files and folders under `roots` whose names have every word (any case), best first: a name that starts
/// with what was typed, then the most recently changed. No words: the most recently changed.
pub fn search(home: Option<&Path>, roots: &[PathBuf], query: &str, limit: usize) -> Vec<Hit> {
    let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
    let typed = query.trim().to_lowercase();
    let (started, mut looked, mut seen) = (Instant::now(), 0usize, HashSet::new());
    let mut hits: Vec<(bool, Hit)> = Vec::new();
    let mut queue: VecDeque<(PathBuf, usize)> = roots.iter().map(|r| (r.clone(), 0)).collect();
    while let Some((dir, depth)) = queue.pop_front() {
        if looked >= MAX_LOOKED_AT || started.elapsed() > MAX_TIME {
            break;
        }
        let Ok(canon) = dir.canonicalize() else { continue };
        if !seen.insert(canon) {
            continue; // (a folder reached twice: OneDrive's Documents, a link)
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            looked += 1;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.starts_with('~') || name.eq_ignore_ascii_case("desktop.ini") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let path = entry.path();
            let is_dir = meta.is_dir();
            let lower = name.to_lowercase();
            if words.iter().all(|w| lower.contains(w.as_str())) {
                let ts = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs_f64()).unwrap_or(0.0);
                hits.push((
                    !typed.is_empty() && lower.starts_with(&typed),
                    Hit { name, path: path.display().to_string(), place: friendly(&dir, home), kind: kind_of(&path, is_dir), ts },
                ));
            }
            if is_dir && depth + 1 < MAX_DEPTH && !SKIP.iter().any(|s| s.eq_ignore_ascii_case(&lower))
                && !ONE_THING.contains(&ext(&path).as_str()) && !meta.file_type().is_symlink()
            {
                queue.push_back((path, depth + 1));
            }
        }
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.ts.partial_cmp(&a.1.ts).unwrap_or(std::cmp::Ordering::Equal)));
    hits.into_iter().map(|(_, h)| h).take(limit).collect()
}

/// The files found lately (the newest few hundred): the only ones the page can have opened.
#[derive(Default)]
pub struct Seen {
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl Seen {
    pub fn add(&mut self, paths: impl Iterator<Item = String>) {
        for p in paths {
            if self.set.insert(p.clone()) {
                self.order.push_back(p);
            }
        }
        while self.order.len() > 400 {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
    }

    pub fn has(&self, p: &Path) -> bool {
        self.set.contains(&p.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tree() -> (tempdir::Dir, Vec<PathBuf>) {
        let home = tempdir::Dir::new();
        for f in ["Documents/Taxes/Taxes 2025.pdf", "Documents/Resume.docx", "Documents/Work/budget.xlsx",
                  "Downloads/setup.exe", "Downloads/notes on taxes.txt", "Documents/node_modules/taxes.js",
                  "Documents/.hidden/taxes.pdf", "Pictures/Bali/Beach.jpg", "Documents/AppData/taxes.pdf"] {
            let p = home.0.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"x").unwrap();
        }
        let roots = ["Desktop", "Documents", "Downloads", "Pictures"].iter().map(|r| home.0.join(r)).collect();
        (home, roots)
    }

    #[test]
    fn finds_by_name_in_the_usual_folders_but_not_programs_folders() {
        let (home, roots) = tree();
        let names: Vec<String> = search(Some(&home.0), &roots, "taxes", 8).into_iter().map(|h| h.name).collect();
        // Every one with the word, the ones starting with it first; none inside node_modules, AppData or a hidden folder.
        assert_eq!(names.len(), 3, "{names:?}");
        assert!(names[..2].contains(&"Taxes 2025.pdf".to_string()) && names[..2].contains(&"Taxes".to_string()));
        assert_eq!(names[2], "notes on taxes.txt");
        let two = search(Some(&home.0), &roots, "taxes 2025", 8);
        assert_eq!(two.len(), 1);
        assert_eq!((two[0].kind, two[0].place.as_str()), ("document", format!("~{0}Documents{0}Taxes", std::path::MAIN_SEPARATOR).as_str()));
        let beach = search(Some(&home.0), &roots, "BEACH", 8);
        assert_eq!((beach[0].name.as_str(), beach[0].kind), ("Beach.jpg", "image"));
        assert_eq!(search(Some(&home.0), &roots, "", 3).len(), 3);
        drop(home);
    }

    #[test]
    fn opens_documents_and_folders_but_not_programs() {
        let (home, _) = tree();
        assert!(openable(&home.0.join("Documents/Taxes/Taxes 2025.pdf")));
        assert!(openable(&home.0.join("Documents/Taxes")));
        assert!(!openable(&home.0.join("Downloads/setup.exe")));
        assert!(!openable(&home.0.join("Documents/node_modules/taxes.js")));
        assert!(!openable(&home.0.join("Documents/missing.pdf")));
        let mut seen = Seen::default();
        seen.add(["/a/b.pdf".to_string()].into_iter());
        assert!(seen.has(Path::new("/a/b.pdf")) && !seen.has(Path::new("/a/c.pdf")));
        seen.add((0..500).map(|i| format!("/x/{i}")));
        assert!(!seen.has(Path::new("/a/b.pdf")) && seen.has(Path::new("/x/499")));
    }

    /// A folder of its own for each test, gone after.
    mod tempdir {
        pub struct Dir(pub std::path::PathBuf);
        impl Dir {
            pub fn new() -> Self {
                use std::sync::atomic::{AtomicUsize, Ordering};
                static N: AtomicUsize = AtomicUsize::new(0);
                let p = std::env::temp_dir().join(format!("cc-files-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
                std::fs::create_dir_all(&p).unwrap();
                Dir(p)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}

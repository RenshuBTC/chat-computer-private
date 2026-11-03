//! Messages from your own accounts: Gmail, Outlook, Telegram, WhatsApp, WeChat and Discord, each read from the
//! service's own web page, where you sign in yourself (in a view of its own in the app: the sign-in stays with it,
//! as a browser's does), and put together in Messages, newest first.
//!
//! Each service you connect has its own view, kept open out of sight (unless it's in the middle) so it stays signed
//! in and up to date. A fixed script in it (reader.js) reads its list of conversations every few seconds and hands
//! it to the app: `messages_found` is the one thing the services' pages may ask the app for (capabilities/
//! messages.json), and the app passes what's read to its own page and nowhere else. Tapping a message opens its
//! service in the middle, at that conversation.

use serde::{Deserialize, Serialize};

pub struct Service {
    pub id: &'static str,
    pub label: &'static str,
    /// Where it opens: its own sign-in or its inbox, once signed in.
    pub url: &'static str,
    /// The sites its pages are on (its sign-in pages may be elsewhere: Google's, Microsoft's).
    pub hosts: &'static [&'static str],
}

pub const SERVICES: [Service; 6] = [
    Service { id: "gmail", label: "Gmail", url: "https://mail.google.com/mail/u/0/#inbox", hosts: &["mail.google.com"] },
    Service {
        id: "outlook",
        label: "Outlook",
        url: "https://outlook.live.com/mail/0/",
        hosts: &["outlook.live.com", "outlook.office.com", "outlook.office365.com"],
    },
    Service { id: "telegram", label: "Telegram", url: "https://web.telegram.org/a/", hosts: &["web.telegram.org"] },
    Service { id: "whatsapp", label: "WhatsApp", url: "https://web.whatsapp.com/", hosts: &["web.whatsapp.com"] },
    Service {
        id: "wechat",
        label: "WeChat",
        url: "https://web.wechat.com/",
        hosts: &["web.wechat.com", "wx.qq.com", "wx2.qq.com", "wx8.qq.com", "web1.wechat.com", "web2.wechat.com"],
    },
    Service { id: "discord", label: "Discord", url: "https://discord.com/channels/@me", hosts: &["discord.com"] },
];

/// Each service's view is called "msg-" and its id.
pub const PREFIX: &str = "msg-";

pub fn by_id(id: &str) -> Option<&'static Service> {
    SERVICES.iter().find(|s| s.id == id)
}

pub fn view_label(s: &Service) -> String {
    format!("{PREFIX}{}", s.id)
}

/// The service a view is for, by its name.
pub fn of_view(label: &str) -> Option<&'static Service> {
    label.strip_prefix(PREFIX).and_then(by_id)
}

/// Whether a page is the service's own (where its list of conversations is read).
pub fn is_own_page(s: &Service, url: &tauri::Url) -> bool {
    url.scheme() == "https" && url.host_str().is_some_and(|h| s.hosts.contains(&h))
}

/// A conversation as the reader found it in the service's list.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Item {
    /// The reader's own name for it (to open it again): letters, digits and a few marks.
    #[serde(default)]
    pub id: String,
    /// Who it's from (or the group's name).
    #[serde(default)]
    pub who: String,
    /// What it says: the subject and its start, or the last message.
    #[serde(default)]
    pub text: String,
    /// When, as the service shows it, and as seconds since 1970 where the reader could tell.
    #[serde(default)]
    pub when: String,
    #[serde(default)]
    pub ts: Option<f64>,
    #[serde(default)]
    pub unread: bool,
}

/// At most this many from each service, and each part at most so long.
pub const MOST: usize = 60;
const WHO: usize = 80;
const TEXT: usize = 300;
const WHEN: usize = 40;
const ID: usize = 64;

fn clip(s: &str, n: usize) -> String {
    let flat: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(n).collect()
}

/// What a service's page handed over, made safe to pass on: at most MOST, each part trimmed and cut to length, the
/// names only of letters, digits and "-_:.", the times sensible (from 1990 to a day ahead of now).
pub fn clean(items: Vec<Item>, now: f64) -> Vec<Item> {
    items
        .into_iter()
        .take(MOST)
        .map(|i| Item {
            id: i.id.chars().filter(|c| c.is_ascii_alphanumeric() || "-_:.".contains(*c)).take(ID).collect(),
            who: clip(&i.who, WHO),
            text: clip(&i.text, TEXT),
            when: clip(&i.when, WHEN),
            ts: i.ts.filter(|t| t.is_finite() && *t > 631_152_000.0 && *t < now + 86_400.0),
            unread: i.unread,
        })
        .filter(|i| !i.who.is_empty() || !i.text.is_empty())
        .collect()
}

/// The script that opens one of them in its service's view (a fixed script: only the item's cleaned name is in it).
pub fn open_script(id: &str) -> Option<String> {
    let id: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || "-_:.".contains(*c)).take(ID).collect();
    (!id.is_empty()).then(|| format!("window.__chatcomputerOpen && window.__chatcomputerOpen({id:?});"))
}

/// The reader, put in each service's view (its pages' main frame only).
pub const READER: &str = include_str!("reader.js");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_service_has_its_own_view_and_pages() {
        assert_eq!(SERVICES.iter().map(|s| s.id).collect::<Vec<_>>(), ["gmail", "outlook", "telegram", "whatsapp", "wechat", "discord"]);
        let gmail = by_id("gmail").unwrap();
        assert_eq!(view_label(gmail), "msg-gmail");
        assert_eq!(of_view("msg-whatsapp").unwrap().label, "WhatsApp");
        assert!(of_view("site").is_none() && of_view("msg-").is_none() && of_view("main").is_none());
        assert!(is_own_page(gmail, &"https://mail.google.com/mail/u/0/#inbox".parse().unwrap()));
        assert!(!is_own_page(gmail, &"https://accounts.google.com/signin".parse().unwrap()));
        assert!(!is_own_page(gmail, &"http://mail.google.com/".parse().unwrap()));
        assert!(!is_own_page(gmail, &"https://mail.google.com.evil.example/".parse().unwrap()));
        for s in &SERVICES {
            let u: tauri::Url = s.url.parse().unwrap();
            assert!(is_own_page(s, &u), "{}", s.id);
        }
    }

    #[test]
    fn what_a_page_hands_over_is_made_safe() {
        let now = 1_793_000_000.0;
        let long = "x".repeat(1000);
        let items = vec![
            Item { id: "t:12<script>".into(), who: " Jane\u{0}  Lim ".into(), text: long.clone(), when: "10:42".into(), ts: Some(now - 60.0), unread: true },
            Item { id: "a".into(), who: "".into(), text: "".into(), ..Item::default() },
            Item { id: "b".into(), who: "Bob".into(), text: "hi".into(), ts: Some(f64::NAN), ..Item::default() },
            Item { id: "c".into(), who: "Carol".into(), ts: Some(now + 10.0 * 86_400.0), ..Item::default() },
        ];
        let out = clean(items, now);
        assert_eq!(out.len(), 3);
        assert_eq!((out[0].id.as_str(), out[0].who.as_str(), out[0].text.len(), out[0].ts), ("t:12script", "Jane Lim", TEXT, Some(now - 60.0)));
        assert_eq!((out[1].ts, out[2].ts), (None, None));
        assert_eq!(clean(vec![Item { who: "x".into(), ..Item::default() }; 100], now).len(), MOST);
    }

    #[test]
    fn opening_one_is_a_fixed_script() {
        assert_eq!(open_script("t:12").unwrap(), "window.__chatcomputerOpen && window.__chatcomputerOpen(\"t:12\");");
        assert_eq!(open_script("\");alert(1);//").unwrap(), "window.__chatcomputerOpen && window.__chatcomputerOpen(\"alert1\");");
        assert!(open_script("<>").is_none());
    }
}

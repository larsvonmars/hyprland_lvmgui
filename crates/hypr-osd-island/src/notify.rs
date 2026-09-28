//! The notification hub: what it is doing, what the notifications *say*, and the
//! buttons that drive it.
//!
//! The hub is read in two halves, because `swaync` publishes them separately.
//!
//! **What it is doing** - how many are waiting, do-not-disturb, whether the
//! control centre is up - comes from `swaync-client -swb`, the interface the
//! daemon publishes for a status bar: it subscribes and then prints one JSON
//! line per change, which is exactly the shape [`hypr_osd_core::follow`] exists
//! for. So the state is event driven and needs no polling: a notification
//! arriving shows up in the panel immediately. The daemon's own vocabulary
//! (`notification`/`none`, `dnd-`, `inhibited-`, `cc-open`) is translated into
//! four facts here, once, so the view never has to know how swaync spells
//! things.
//!
//! **What the notifications say** - the application, the summary, the icon -
//! swaync does not publish at all: it has no verb for the list, and its own
//! D-Bus objects expose nothing but GTK's own interfaces (checked against
//! `org.erikreider.swaync` and `org.erikreider.swaync.cc`: no actions, no
//! properties). So the words are read off the bus instead, where the daemon's
//! incoming `Notify` calls and its `NotificationClosed` signals are ordinary
//! messages. `busctl monitor` prints every message as one JSON line, which is
//! the same shape again - and `serde_json` was already a dependency for the line
//! above.
//!
//! Monitoring is a privileged operation on some buses. Where it is refused,
//! `busctl` exits at once (the follower restarts a tool that has exited, one
//! second later), the history stays empty and the tile is what it was before any
//! of this existed: the count, the dnd switch and the buttons. That is why the
//! two halves are kept apart - the state is worth having on its own.

use std::time::{Duration, Instant};

use hypr_osd_core::follow::Follow;
use serde_json::Value;

/// What the notification daemon is doing right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Notifications {
    /// How many notifications are waiting.
    pub count: i32,
    /// Do-not-disturb is on.
    pub dnd: bool,
    /// Notifications are being suppressed by something else (a screen share, a
    /// presentation) rather than by the user.
    pub inhibited: bool,
    /// swaync's own control centre is on screen.
    pub centre_open: bool,
}

/// Follow the daemon: `on_change` runs on the main loop for every change,
/// including the state it reports when it attaches.
pub fn follow(on_change: impl Fn(Notifications) + 'static) -> Follow {
    Follow::start("swaync-client", &["-swb"], move |line| match parse(line) {
        Some(notifications) => on_change(notifications),
        // A line nobody can read is dropped rather than guessed at; the previous
        // state stays on screen until the next one arrives.
        None => eprintln!("hypr-osd-island: unreadable swaync line: {line}"),
    })
}

/// One line of `swaync-client -swb`.
///
/// ```json
/// {"text": 3, "alt": "dnd-notification", "class": ["dnd", "cc-open"], "tooltip": "…"}
/// ```
///
/// `alt` carries two flags and the presence of a count: it is
/// `[dnd-][inhibited-]` followed by `notification` or `none`. The `class` list
/// carries the same flags again plus `cc-open`, which is why it is the one
/// consulted for the control centre.
fn parse(line: &str) -> Option<Notifications> {
    let value: Value = serde_json::from_str(line).ok()?;
    let alt = value.get("alt").and_then(Value::as_str).unwrap_or_default();
    let classes: Vec<&str> = value
        .get("class")
        .and_then(Value::as_array)
        .map(|classes| classes.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    Some(Notifications {
        count: count_of(value.get("text")),
        dnd: alt.starts_with("dnd") || classes.contains(&"dnd"),
        inhibited: alt.contains("inhibited") || classes.contains(&"inhibited"),
        centre_open: classes.contains(&"cc-open"),
    })
}

/// The count arrives as a string ("3") in some versions and a number in others,
/// and as an empty string when there is nothing waiting.
fn count_of(text: Option<&Value>) -> i32 {
    match text {
        Some(Value::Number(number)) => number.as_i64().unwrap_or(0).max(0) as i32,
        Some(Value::String(text)) => text.trim().parse().unwrap_or(0).max(0),
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// What the notifications say
// ---------------------------------------------------------------------------

/// The interface every message in this half belongs to.
const NOTIFICATIONS: &str = "org.freedesktop.Notifications";

/// What the bus monitor is run with: the session bus, one JSON line per message,
/// and the traffic of the notification daemon - which is where both the `Notify`
/// calls coming in and the daemon's own `NotificationClosed` signals are seen.
/// (`--json=` reached `busctl monitor` in systemd 257; this desktop runs 262.)
const MONITOR: &[&str] = &[
    "--user",
    "--json=short",
    "monitor",
    "org.freedesktop.Notifications",
];

/// How many notifications are remembered. A few more than the panel draws, so
/// that dismissing one promotes the next rather than leaving a gap.
const HISTORY_CAP: usize = 8;

/// How many `Notify` calls may be waiting for the id their reply carries. The
/// wait is one round trip long, so this is only a guard against a reply that
/// never came (a missed line would otherwise leave an entry behind for good).
const WAITING_CAP: usize = 16;

/// One notification, as the bus carried it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// What the sending application calls itself.
    pub app: String,
    /// The one line a row draws: the summary, or the body when the sender left
    /// the summary empty.
    pub title: String,
    /// What came after the summary, as sent. Nothing is drawn from it - it is
    /// what the row's tooltip shows, so a title that had to be ellipsised is
    /// still readable in full.
    pub detail: String,
    /// The best picture hint the sender gave: an icon name, or the path of a
    /// file when the sender attached a picture. Empty when there was none, and
    /// the row stands on the application's first letter then.
    pub icon: String,
    /// 0 low, 1 normal, 2 critical. A critical notification is one swaync keeps
    /// on screen until it is dealt with, so the row wears the warning colour.
    pub urgency: u8,
    /// When it arrived, for the age at the end of the row.
    pub at: Instant,
    /// The daemon's id, which is what a close quotes. 0 until the reply to the
    /// `Notify` call comes back - the row is put up from the call itself,
    /// because waiting for the reply would hold it back by a round trip.
    pub id: u32,
    /// This module's own number for the same notification. The reply carries a
    /// cookie rather than an id, so the call has to be remembered by something
    /// that is already known while it is being made.
    serial: u32,
}

impl Item {
    /// How long ago this arrived, the shortest way of saying it.
    pub fn age(&self, now: Instant) -> String {
        age_text(now.saturating_duration_since(self.at))
    }

    /// What the row's tooltip says: everything the sender wrote, since the row
    /// itself has room for one line.
    pub fn tooltip(&self) -> String {
        if self.detail.is_empty() {
            self.title.clone()
        } else {
            format!("{}\n{}", self.title, self.detail)
        }
    }
}

/// `now`, `5 min`, `2 h`, `3 d` - the same steps swaync's own panel uses.
fn age_text(age: Duration) -> String {
    let seconds = age.as_secs();
    match seconds {
        0..=59 => "now".to_string(),
        60..=3599 => format!("{} min", seconds / 60),
        3600..=86_399 => format!("{} h", seconds / 3600),
        _ => format!("{} d", seconds / 86_400),
    }
}

/// The notifications seen so far, newest first.
///
/// Only what the bus carried while this process was watching: swaync keeps the
/// notifications themselves, but publishes none of them, so a panel that starts
/// after they arrived cannot know about them. What it does know is when one goes
/// away - the daemon says so - so a dismissed notification does not linger here.
#[derive(Debug, Default)]
pub struct History {
    items: Vec<Item>,
    /// The `Notify` calls whose id has not come back yet: who called, with which
    /// cookie, and which entry the call added. Without this the id would be
    /// unknowable - and the id is what a close quotes, so a notification that
    /// was dismissed could never be taken off the list.
    waiting: Vec<Waiting>,
    serial: u32,
}

#[derive(Debug)]
struct Waiting {
    /// The caller's unique bus name, and the cookie it put on the call.
    sender: String,
    cookie: u32,
    serial: u32,
}

impl History {
    pub fn new() -> Self {
        History::default()
    }

    /// The notifications to draw, newest first.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Read one line of the bus monitor, and answer whether the *picture*
    /// changed - a row added, replaced or removed. A line that only fills in an
    /// id (the reply to a `Notify`) changes nothing on screen, and says so.
    pub fn apply(&mut self, line: &str) -> bool {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            // A line nobody can read is dropped rather than guessed at, exactly
            // like the swaync line above: the list stays as it was.
            return false;
        };
        match message.get("type").and_then(Value::as_str) {
            Some("method_call") if is_notifications(&message, "Notify") => self.arrived(&message),
            Some("method_call") if is_notifications(&message, "CloseNotification") => {
                self.closed(first_id(&message))
            }
            Some("signal") if is_notifications(&message, "NotificationClosed") => {
                self.closed(first_id(&message))
            }
            Some("method_return") => {
                self.reply(&message);
                false
            }
            _ => false,
        }
    }

    /// A `Notify` call: one more notification, unless it replaces one that is
    /// already on the list.
    fn arrived(&mut self, message: &Value) -> bool {
        let Some(data) = message.pointer("/payload/data").and_then(Value::as_array) else {
            return false;
        };
        // The call's arguments, in the order the freedesktop spec puts them:
        // app_name, replaces_id, app_icon, summary, body, actions, hints, timeout.
        let text = |index: usize| data.get(index).and_then(Value::as_str).unwrap_or_default();
        let hints = data.get(6);
        let app = text(0).to_string();
        let replaces = data.get(1).and_then(Value::as_u64).unwrap_or(0) as u32;
        // The summary is the row's line; the body is kept as sent, for the
        // tooltip. With no summary the body has to be the line itself - plenty
        // of senders write the whole message in one or the other.
        let summary = collapse(text(3));
        let body = text(4).trim().to_string();
        // With no summary the body has to be the line itself - plenty of senders
        // write the whole message into one field or the other.
        let has_summary = !summary.is_empty();

        self.serial = self.serial.wrapping_add(1);
        let mut item = Item {
            app,
            title: if has_summary {
                summary
            } else {
                collapse(text(4))
            },
            detail: if has_summary { body } else { String::new() },
            icon: icon_hint(text(2), hints),
            urgency: urgency(hints),
            at: Instant::now(),
            id: 0,
            serial: self.serial,
        };

        // A replacement takes the place of the notification it updates, and keeps
        // the id the daemon already gave it - so no reply has to be waited for.
        if replaces != 0 {
            if let Some(existing) = self.items.iter_mut().find(|item| item.id == replaces) {
                item.serial = existing.serial;
                item.id = replaces;
                *existing = item;
                return true;
            }
        }

        let (Some(sender), Some(cookie)) = (
            message.get("sender").and_then(Value::as_str),
            message.get("cookie").and_then(Value::as_u64),
        ) else {
            return false;
        };
        if self.waiting.len() >= WAITING_CAP {
            self.waiting.remove(0);
        }
        self.waiting.push(Waiting {
            sender: sender.to_string(),
            cookie: cookie as u32,
            serial: item.serial,
        });
        self.items.insert(0, item);
        self.items.truncate(HISTORY_CAP);
        true
    }

    /// The answer to a `Notify` call, which is where the id comes from.
    fn reply(&mut self, message: &Value) {
        let cookie = message.get("reply_cookie").and_then(Value::as_u64);
        let caller = message.get("destination").and_then(Value::as_str);
        let id = message.pointer("/payload/data/0").and_then(Value::as_u64);
        let (Some(cookie), Some(caller), Some(id)) = (cookie, caller, id) else {
            return;
        };
        // The cookie alone is not enough to pair on: two callers can be using the
        // same number at the same time. The caller is the other half of the key.
        let Some(position) = self
            .waiting
            .iter()
            .position(|waiting| waiting.sender == caller && waiting.cookie == cookie as u32)
        else {
            return;
        };
        let waiting = self.waiting.remove(position);
        if let Some(item) = self
            .items
            .iter_mut()
            .find(|item| item.serial == waiting.serial)
        {
            item.id = id as u32;
        }
    }

    /// A notification the daemon has let go of - dismissed, replaced, or expired
    /// on its timeout.
    fn closed(&mut self, id: u32) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.id != id);
        self.items.len() != before
    }
}

/// Follow the bus: one JSON line per D-Bus message, which is what
/// [`History::apply`] reads. `busctl` is systemd's own bus client - already
/// installed wherever there is a session bus to monitor.
pub fn follow_history(on_line: impl Fn(&str) + 'static) -> Follow {
    Follow::start("busctl", MONITOR, on_line)
}

/// Whether this message is `member` of the notification interface.
fn is_notifications(message: &Value, member: &str) -> bool {
    message.get("interface").and_then(Value::as_str) == Some(NOTIFICATIONS)
        && message.get("member").and_then(Value::as_str) == Some(member)
}

/// The first argument of a message that carries an id.
fn first_id(message: &Value) -> u32 {
    message
        .pointer("/payload/data/0")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32
}

/// A hint out of a `Notify` call's hint dictionary, if the sender set it.
fn hint<'a>(hints: Option<&'a Value>, name: &str) -> Option<&'a str> {
    let value = hints?.get(name)?;
    // `busctl` renders a variant as `{"type": "s", "data": "…"}`; a hand-written
    // line may carry the bare value instead.
    value
        .get("data")
        .and_then(Value::as_str)
        .or_else(|| value.as_str())
}

/// The best picture hint a `Notify` call carries, most specific first: the image
/// the sender attached, the application's own icon, or the desktop entry to look
/// one up with. Empty when the sender said nothing, and the row falls back to the
/// application's first letter.
///
/// The raw-pixel hints (`image-data`, `icon_data`) are deliberately not read:
/// they are ARGB buffers, and reading them would mean decoding and scaling a
/// bitmap here instead of letting the icon theme answer.
fn icon_hint(app_icon: &str, hints: Option<&Value>) -> String {
    for name in ["image-path", "image_path", "app-icon"] {
        if let Some(value) = hint(hints, name).filter(|value| !value.is_empty()) {
            return value.to_string();
        }
    }
    if !app_icon.is_empty() {
        return app_icon.to_string();
    }
    // `desktop-entry` is a file name, and the icon is named after the entry.
    match hint(hints, "desktop-entry").filter(|value| !value.is_empty()) {
        Some(entry) => entry.strip_suffix(".desktop").unwrap_or(entry).to_string(),
        None => String::new(),
    }
}

/// How urgent the sender says this is. Saying nothing means normal.
fn urgency(hints: Option<&Value>) -> u8 {
    let Some(value) = hints.and_then(|hints| hints.get("urgency")) else {
        return 1;
    };
    let data = value.get("data").unwrap_or(value);
    // The hint is a byte by spec, but a string is seen in the wild.
    data.as_u64()
        .or_else(|| data.as_str().and_then(|text| text.trim().parse().ok()))
        .unwrap_or(1)
        .min(2) as u8
}

/// One line of text out of a title that may carry its own line breaks: a row
/// draws exactly one line, and a newline in a label is not one line.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// What the hub's buttons do
// ---------------------------------------------------------------------------

/// The five things the hub can ask the daemon for. Kept as an enum so the
/// commands live next to their doc comments rather than in five string literals
/// in the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Show or hide the control centre.
    ToggleCentre,
    /// Turn do-not-disturb on or off.
    ToggleDnd,
    /// Hide the notifications that are showing, without dismissing them.
    HideShown,
    /// Dismiss everything.
    DismissAll,
}

impl Command {
    /// The `swaync-client` arguments for this command. `-sw` makes the client
    /// wait for the change to land before it exits, so the state that comes back
    /// through the follower is the state the button caused.
    pub fn args(self) -> &'static [&'static str] {
        match self {
            Command::ToggleCentre => &["-t", "-sw"],
            Command::ToggleDnd => &["-d", "-sw"],
            Command::HideShown => &["--hide-all"],
            Command::DismissAll => &["-C"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_line_with_notifications_waiting() {
        let line =
            r#"{"text": 3, "alt": "notification", "class": ["notification"], "tooltip": "x"}"#;
        assert_eq!(
            parse(line),
            Some(Notifications {
                count: 3,
                dnd: false,
                inhibited: false,
                centre_open: false,
            })
        );
    }

    #[test]
    fn reads_do_not_disturb_and_the_open_centre() {
        let line = r#"{"text": "", "alt": "dnd-none", "class": ["dnd", "cc-open"]}"#;
        let notifications = parse(line).expect("a readable line");
        assert!(notifications.dnd);
        assert!(notifications.centre_open);
        assert_eq!(notifications.count, 0);
    }

    #[test]
    fn reads_inhibition() {
        let line = r#"{"text": "1", "alt": "inhibited-notification", "class": []}"#;
        let notifications = parse(line).expect("a readable line");
        assert!(notifications.inhibited);
        assert!(!notifications.dnd);
        // The count is a string in this version of the client.
        assert_eq!(notifications.count, 1);
    }

    #[test]
    fn a_line_it_cannot_read_is_not_a_state() {
        assert_eq!(parse("not json at all"), None);
        // No count, no flags: still a state (nothing waiting, nothing on).
        assert_eq!(parse("{}"), Some(Notifications::default()));
    }

    #[test]
    fn a_nonsense_count_is_no_count() {
        assert_eq!(count_of(Some(&serde_json::json!("many"))), 0);
        assert_eq!(count_of(Some(&serde_json::json!(-4))), 0);
        assert_eq!(count_of(None), 0);
        assert_eq!(count_of(Some(&serde_json::json!(7))), 7);
    }

    // ---- what the notifications say -----------------------------------------
    //
    // These three fixtures are lines this machine's bus really carried, captured
    // with `busctl --user --json=short monitor org.freedesktop.Notifications`
    // (one line each here, where this file's width would have wrapped them).

    /// `notify-send -a TestApp -i firefox -u critical "Test summary here" "Test body text"`
    const NOTIFY: &str = r#"{"type":"method_call","endian":"l","flags":0,"version":1,"cookie":9,"timestamp-realtime":1790619394932397,"sender":":1.11630","destination":":1.6144","path":"/org/freedesktop/Notifications","interface":"org.freedesktop.Notifications","member":"Notify","payload":{"type":"susssasa{sv}i","data":["TestApp",0,"","Test summary here","Test body text",[],{"image-path":{"type":"s","data":"firefox"},"urgency":{"type":"y","data":2},"sender-pid":{"type":"x","data":340356}},-1]}}"#;
    /// The answer to it, which is the only place the daemon's id appears.
    const REPLY: &str = r#"{"type":"method_return","endian":"l","flags":1,"version":1,"cookie":4294967295,"reply_cookie":9,"timestamp-realtime":1790619394934051,"sender":":1.6144","destination":":1.11630","payload":{"type":"u","data":[18]}}"#;
    /// What the daemon says when that notification goes away again.
    const CLOSED: &str = r#"{"type":"signal","endian":"l","flags":1,"version":1,"cookie":167,"timestamp-realtime":1790619395440953,"sender":":1.6144","path":"/org/freedesktop/Notifications","interface":"org.freedesktop.Notifications","member":"NotificationClosed","payload":{"type":"uu","data":[18,2]}}"#;

    /// A `Notify` call of the same shape, for the tests that need more than one.
    fn notify(app: &str, replaces: u32, summary: &str, body: &str) -> String {
        format!(
            r#"{{"type":"method_call","cookie":1,"sender":":1.5","destination":":1.6","interface":"org.freedesktop.Notifications","member":"Notify","payload":{{"type":"susssasa{{sv}}i","data":["{app}",{replaces},"",{summary:?},{body:?},[],{{}},-1]}}}}"#
        )
    }

    #[test]
    fn a_notify_call_becomes_a_row() {
        let mut history = History::new();
        assert!(history.apply(NOTIFY));
        let item = &history.items()[0];
        assert_eq!(item.app, "TestApp");
        assert_eq!(item.title, "Test summary here");
        assert_eq!(item.detail, "Test body text");
        assert_eq!(item.icon, "firefox");
        assert_eq!(item.urgency, 2);
        assert_eq!(item.tooltip(), "Test summary here\nTest body text");
        // The id is not known until the daemon answers the call.
        assert_eq!(item.id, 0);
    }

    #[test]
    fn the_id_arrives_with_the_reply_and_a_close_removes_the_row() {
        let mut history = History::new();
        history.apply(NOTIFY);
        // The reply changes no picture: the row is already up.
        assert!(!history.apply(REPLY));
        assert_eq!(history.items()[0].id, 18);
        // ...and the close is what takes it away again.
        assert!(history.apply(CLOSED));
        assert!(history.items().is_empty());
    }

    #[test]
    fn a_close_for_something_this_panel_never_saw_is_not_a_change() {
        let mut history = History::new();
        history.apply(NOTIFY);
        assert!(!history.apply(&CLOSED.replace("[18,2]", "[99,1]")));
        assert_eq!(history.items().len(), 1);
    }

    #[test]
    fn an_update_takes_the_place_of_the_notification_it_replaces() {
        let mut history = History::new();
        history.apply(NOTIFY);
        history.apply(REPLY);
        // A `replaces_id` of 18 is the daemon's way of saying "that one, again" -
        // a download's progress, a chat message that was edited.
        history.apply(&notify("TestApp", 18, "Updated summary", ""));
        assert_eq!(history.items().len(), 1);
        assert_eq!(history.items()[0].title, "Updated summary");
        assert_eq!(history.items()[0].id, 18);
    }

    #[test]
    fn only_the_newest_few_are_remembered() {
        let mut history = History::new();
        for number in 0..HISTORY_CAP + 2 {
            history.apply(&notify("App", 0, &format!("number {number}"), ""));
        }
        assert_eq!(history.items().len(), HISTORY_CAP);
        // Newest first: the row drawn at the top is the last one to arrive.
        assert_eq!(
            history.items()[0].title,
            format!("number {}", HISTORY_CAP + 1)
        );
    }

    #[test]
    fn the_body_is_the_line_when_there_is_no_summary() {
        let mut history = History::new();
        history.apply(&notify("App", 0, "", "Only the body"));
        let item = &history.items()[0];
        assert_eq!(item.title, "Only the body");
        // Nothing is repeated in the tooltip then.
        assert!(item.detail.is_empty());
        assert_eq!(item.tooltip(), "Only the body");
    }

    #[test]
    fn a_title_is_one_line_however_the_sender_wrote_it() {
        let mut history = History::new();
        history.apply(&notify("App", 0, "  a\n  long\n\ttitle  ", "body"));
        assert_eq!(history.items()[0].title, "a long title");
    }

    #[test]
    fn the_icon_is_the_best_hint_the_sender_gave() {
        let hints = |json: &str| serde_json::from_str::<Value>(json).ok();
        // The attached image wins over the application's own icon...
        assert_eq!(
            icon_hint(
                "fallback",
                hints(r#"{"image-path":{"type":"s","data":"/tmp/art.png"},"app-icon":{"type":"s","data":"mail"}}"#).as_ref()
            ),
            "/tmp/art.png"
        );
        // ...then comes the `app_icon` argument...
        assert_eq!(icon_hint("firefox", hints("{}").as_ref()), "firefox");
        // ...and the desktop entry is the last hint before giving up, which means
        // shedding the file name's suffix so the icon theme can answer.
        assert_eq!(
            icon_hint(
                "",
                hints(r#"{"desktop-entry":{"type":"s","data":"org.gnome.Maps.desktop"}}"#).as_ref()
            ),
            "org.gnome.Maps"
        );
        // Nothing at all: the row stands on the application's first letter.
        assert_eq!(icon_hint("", None), "");
        assert_eq!(icon_hint("", hints("{}").as_ref()), "");
    }

    #[test]
    fn urgency_is_a_number_either_way_and_normal_when_absent() {
        assert_eq!(urgency(None), 1);
        assert_eq!(urgency(Some(&serde_json::json!({}))), 1);
        assert_eq!(
            urgency(Some(
                &serde_json::json!({"urgency": {"type": "y", "data": 2}})
            )),
            2
        );
        // Some senders write it as a string; it is a byte by spec, not by nature.
        assert_eq!(
            urgency(Some(
                &serde_json::json!({"urgency": {"type": "s", "data": "0"}})
            )),
            0
        );
    }

    #[test]
    fn an_age_reads_the_way_the_panel_says_it() {
        assert_eq!(age_text(Duration::from_secs(0)), "now");
        assert_eq!(age_text(Duration::from_secs(59)), "now");
        assert_eq!(age_text(Duration::from_secs(60)), "1 min");
        assert_eq!(age_text(Duration::from_secs(59 * 60)), "59 min");
        assert_eq!(age_text(Duration::from_secs(3600)), "1 h");
        assert_eq!(age_text(Duration::from_secs(86_400)), "1 d");
    }

    #[test]
    fn a_line_this_module_cannot_read_changes_nothing() {
        let mut history = History::new();
        assert!(!history.apply("not json at all"));
        assert!(!history.apply("{}"));
        // Most of what the monitor carries is other people's traffic.
        assert!(!history.apply(
            r#"{"type":"method_call","interface":"org.freedesktop.DBus","member":"AddMatch","cookie":4,"sender":":1.9","payload":{"type":"s","data":["x"]}}"#
        ));
        assert!(history.items().is_empty());
    }
}

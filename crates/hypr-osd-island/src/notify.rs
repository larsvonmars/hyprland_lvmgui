//! The notification hub's feed, and the buttons that drive it.
//!
//! `swaync` is the notification daemon on this desktop, and `swaync-client
//! -swb` is the interface it publishes for a status bar: it subscribes and then
//! prints one JSON line per change, which is exactly the shape
//! [`hypr_osd_core::follow`] exists for. So the hub is event driven and needs no
//! polling: a notification arriving shows up in the panel immediately.
//!
//! The daemon's own vocabulary (`notification`/`none`, `dnd-`, `inhibited-`,
//! `cc-open`) is translated here into four facts, once, so the view never has to
//! know how swaync spells things.

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
}

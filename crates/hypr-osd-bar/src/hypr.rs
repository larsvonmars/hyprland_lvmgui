//! What Hyprland knows, kept up to date by its event stream.
//!
//! The workspace row and the window title are the two parts of the bar that have
//! to change *while* you are doing something - a swipe should light the next
//! workspace up as it lands, not a second later. So this does not poll: it
//! subscribes to Hyprland's event socket ([`hypripc::Events`]) and re-reads the
//! state when something relevant happened. The reads go over Hyprland's command
//! socket, which is a socket round-trip rather than a process.
//!
//! Two details worth knowing:
//!
//! * Events arrive in bursts (a workspace switch emits three or four), so reads
//!   are coalesced: the first event schedules a refresh, the rest join it.
//! * There is a slow safety poll as well, in the bar's tick. An event socket that
//!   quietly died would otherwise leave the row frozen for the session, which
//!   looks like a bug in the bar rather than in the connection.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use serde_json::Value;

use hypr_osd_core::hypripc::{self, Events};

/// How long to wait after the first event before reading, so a burst of them
/// costs one round-trip.
const COALESCE: Duration = Duration::from_millis(30);

/// One workspace, as the row draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    pub id: i32,
    /// What Hyprland calls it - its id as text, unless it was renamed.
    pub name: String,
    pub windows: i32,
    /// The workspace on the monitor you are using: the one that gets the accent
    /// fill in the row.
    pub active: bool,
    /// The workspace showing on *another* monitor. It is on screen, so it is not
    /// "empty", but it is not where you are either.
    pub visible: bool,
}

/// Everything the left half of the bar shows.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub workspaces: Vec<Workspace>,
    /// The focused window's title, empty when there is none or it has none.
    pub title: String,
    /// The focused window's class, for the tooltip.
    pub class: String,
}

/// Read the current state. `persistent` is how many numbered workspaces the row
/// keeps visible even when they do not exist yet - the same idea as the five
/// fixed pills of the bar this replaces: a workspace you can switch to should be
/// clickable before you have ever been there.
pub fn read(persistent: i32) -> Snapshot {
    let workspaces = hypripc::json("j/workspaces");
    let monitors = hypripc::json("j/monitors");
    let active = hypripc::json("j/activewindow");
    build(
        workspaces.as_ref(),
        monitors.as_ref(),
        active.as_ref(),
        persistent,
    )
}

/// The half of [`read`] with decisions in it, so it can be tested against
/// answers exactly as Hyprland writes them.
fn build(
    workspaces: Option<&Value>,
    monitors: Option<&Value>,
    active_window: Option<&Value>,
    persistent: i32,
) -> Snapshot {
    let (active_id, visible_ids) = monitor_state(monitors);

    // The row is the numbered workspaces the bar keeps room for, plus any
    // workspace that really exists - so a workspace named `special` or numbered
    // past the persistent ones still gets a pill once it holds something.
    let existing = workspaces
        .and_then(Value::as_array)
        .map(|list| list.as_slice())
        .unwrap_or_default();

    let mut ids: Vec<i32> = (1..=persistent.max(0)).collect();
    for workspace in existing {
        if let Some(id) = workspace.get("id").and_then(Value::as_i64) {
            let id = i32::try_from(id).unwrap_or(0);
            if id > 0 && !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids.sort_unstable();

    let workspaces = ids
        .into_iter()
        .map(|id| {
            let found = existing.iter().find(|workspace| {
                workspace.get("id").and_then(Value::as_i64) == Some(i64::from(id))
            });
            Workspace {
                id,
                name: found
                    .map(|workspace| text(workspace, "name"))
                    .unwrap_or_else(|| id.to_string()),
                windows: found
                    .and_then(|workspace| workspace.get("windows"))
                    .and_then(Value::as_i64)
                    .and_then(|count| i32::try_from(count).ok())
                    .unwrap_or(0),
                active: active_id == Some(id),
                visible: visible_ids.contains(&id),
            }
        })
        .collect();

    let window = active_window.unwrap_or(&Value::Null);
    Snapshot {
        workspaces,
        title: text(window, "title"),
        class: text(window, "class"),
    }
}

/// Which workspace is focused, and which ones are showing on the other
/// monitors. `id` is negative for special workspaces, which are left out.
fn monitor_state(monitors: Option<&Value>) -> (Option<i32>, Vec<i32>) {
    let mut focused = None;
    let mut visible = Vec::new();
    let Some(monitors) = monitors.and_then(Value::as_array) else {
        return (focused, visible);
    };
    for monitor in monitors {
        let Some(id) = monitor
            .get("activeWorkspace")
            .and_then(|workspace| workspace.get("id"))
            .and_then(Value::as_i64)
            .and_then(|id| i32::try_from(id).ok())
            .filter(|id| *id > 0)
        else {
            continue;
        };
        if monitor
            .get("focused")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            focused = Some(id);
        } else {
            visible.push(id);
        }
    }
    (focused, visible)
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The events that can change [`Snapshot`]. `windowtitlev2` is the one that
/// carries a *title change* of the window you are already in - without it the
/// title would only catch up when you switched windows.
const RELEVANT: &[&str] = &[
    "workspace",
    "createworkspace",
    "destroyworkspace",
    "moveworkspace",
    "renameworkspace",
    "activespecial",
    "openwindow",
    "closewindow",
    "movewindow",
    "activewindow",
    "windowtitle",
    "windowtitlev2",
    "focusedmon",
    "monitor",
    "fullscreen",
];

/// Call `on_change` when the bar's left half may be out of date.
///
/// Keep the returned handle: dropping it stops the subscription.
pub fn watch(on_change: impl Fn() + 'static) -> Events {
    // One refresh per burst. `scheduled` is the "a timer is already running"
    // flag, and it is cleared *inside* the timer, so an event that arrives while
    // the refresh runs schedules the next one rather than being lost.
    let scheduled = Rc::new(Cell::new(false));
    let on_change = Rc::new(on_change);
    Events::start(move |line| {
        let name = line.split(">>").next().unwrap_or_default();
        if !RELEVANT.contains(&name) || scheduled.replace(true) {
            return;
        }
        let scheduled = scheduled.clone();
        let on_change = on_change.clone();
        glib::timeout_add_local_once(COALESCE, move || {
            scheduled.set(false);
            on_change();
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspaces() -> Value {
        serde_json::json!([
            { "id": 1, "name": "1", "windows": 2 },
            { "id": 3, "name": "3", "windows": 0 },
            { "id": -99, "name": "special:magic", "windows": 1 },
        ])
    }

    fn monitors(focused_id: i64, other_id: i64) -> Value {
        serde_json::json!([
            { "name": "eDP-1", "focused": true, "activeWorkspace": { "id": focused_id } },
            { "name": "DP-1", "focused": false, "activeWorkspace": { "id": other_id } },
        ])
    }

    #[test]
    fn the_row_keeps_room_for_workspaces_that_do_not_exist_yet() {
        let snapshot = build(Some(&workspaces()), Some(&monitors(3, 1)), None, 5);
        let ids: Vec<i32> = snapshot.workspaces.iter().map(|w| w.id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);
        // Special workspaces are not places you keep windows, so they get no pill.
        assert!(snapshot.workspaces.iter().all(|w| w.id > 0));
    }

    #[test]
    fn the_focused_workspace_is_active_and_the_other_monitors_are_visible() {
        let snapshot = build(Some(&workspaces()), Some(&monitors(3, 1)), None, 3);
        let by_id = |id: i32| {
            snapshot
                .workspaces
                .iter()
                .find(|w| w.id == id)
                .expect("the pill exists")
        };
        assert!(by_id(3).active);
        assert!(!by_id(3).visible);
        // On screen, on the other monitor: visible, and not "empty".
        assert!(by_id(1).visible);
        assert!(!by_id(1).active);
        assert_eq!(by_id(1).windows, 2);
        // Neither: an empty workspace you can still switch to.
        assert!(!by_id(2).active && !by_id(2).visible);
        assert_eq!(by_id(2).windows, 0);
    }

    #[test]
    fn a_workspace_that_exists_past_the_persistent_ones_gets_a_pill() {
        let workspaces = serde_json::json!([{ "id": 9, "name": "9", "windows": 1 }]);
        let snapshot = build(Some(&workspaces), Some(&monitors(9, 1)), None, 2);
        let ids: Vec<i32> = snapshot.workspaces.iter().map(|w| w.id).collect();
        assert_eq!(ids, vec![1, 2, 9]);
    }

    #[test]
    fn a_renamed_workspace_keeps_its_name() {
        let workspaces = serde_json::json!([{ "id": 2, "name": "code", "windows": 3 }]);
        let snapshot = build(Some(&workspaces), None, None, 2);
        assert_eq!(snapshot.workspaces[1].name, "code");
    }

    #[test]
    fn the_title_comes_from_the_focused_window() {
        let window = serde_json::json!({ "title": "README.md", "class": "kitty" });
        let snapshot = build(Some(&workspaces()), Some(&monitors(1, 1)), Some(&window), 1);
        assert_eq!(snapshot.title, "README.md");
        assert_eq!(snapshot.class, "kitty");
    }

    #[test]
    fn no_window_is_no_title_rather_than_a_stale_one() {
        // Hyprland answers `{}` for `j/activewindow` on an empty workspace.
        let empty = serde_json::json!({});
        let snapshot = build(None, None, Some(&empty), 3);
        assert!(snapshot.title.is_empty());
        assert_eq!(snapshot.workspaces.len(), 3);
    }
}

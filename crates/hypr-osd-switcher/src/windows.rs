//! The window list, straight from Hyprland.
//!
//! `hyprctl -j clients` is the only source, and it happens to carry everything
//! this element needs: the class (to find an icon), the title (to label a tile),
//! the address (to focus the window on the way out) and `focusHistoryID` - which
//! is what makes this an alt-tab rather than a list of windows, because 0 is the
//! window you are in, 1 the one you were in before it, and so on.

use hypr_osd_core::hyprctl;
use serde_json::Value;

/// One switchable window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    /// `0x…`, which is what a dispatch wants back to focus it.
    pub address: String,
    /// The application's class, e.g. `firefox`. Empty for a window that never
    /// set one (rare, but possible under XWayland).
    pub class: String,
    /// What the window calls itself.
    pub title: String,
    /// The workspace it lives on, for the tooltip.
    pub workspace: String,
}

impl Window {
    /// What a tile is labelled with: the title, or the class when there is no
    /// title yet - a window that is still starting up has none, and a tile with
    /// no label at all reads as a broken card.
    pub fn label(&self) -> &str {
        if self.title.is_empty() {
            &self.class
        } else {
            &self.title
        }
    }

    /// The one-line form: the tooltip on a tile, and what `status` prints.
    pub fn describe(&self) -> String {
        format!(
            "{} · workspace {} · {}",
            self.label(),
            self.workspace,
            self.address
        )
    }
}

/// Every mapped window, most recently used first.
///
/// Windows on other workspaces - even on a special workspace - are included on
/// purpose: switching to one is supposed to bring it up, workspace and all.
pub fn list() -> Vec<Window> {
    let Some(clients) = hyprctl::json(&["clients"]) else {
        eprintln!("hypr-osd-switcher: hyprctl clients did not answer - nothing to switch to");
        return Vec::new();
    };
    let Some(clients) = clients.as_array() else {
        eprintln!("hypr-osd-switcher: hyprctl clients answered something unexpected");
        return Vec::new();
    };
    sorted(clients)
}

/// The clients in `clients`, most recently used first.
///
/// Split out from [`list`] because this is the part with decisions in it, and a
/// decision is worth a test that does not need a compositor.
fn sorted(clients: &[Value]) -> Vec<Window> {
    let mut windows: Vec<(i64, Window)> = clients.iter().filter_map(window_of).collect();
    // `focusHistoryID` is the whole point of using clients over anything else:
    // the window you used last comes first, whatever order the array was in.
    windows.sort_by_key(|(history, _)| *history);
    windows.into_iter().map(|(_, window)| window).collect()
}

fn window_of(client: &Value) -> Option<(i64, Window)> {
    // A window that is not on screen is not something to switch to. Windows on
    // other *workspaces* are mapped and stay in the list; a minimised, still
    // starting or otherwise absent one is not.
    if !client
        .get("mapped")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    // A window that has never been focused has no history id; it sorts last,
    // which is where a window you have not used yet belongs.
    let history = client
        .get("focusHistoryID")
        .and_then(Value::as_i64)
        .unwrap_or(i64::MAX);
    Some((
        history,
        Window {
            address: text(client, "address"),
            class: text(client, "class"),
            title: text(client, "title"),
            workspace: client
                .get("workspace")
                .map(|workspace| text(workspace, "name"))
                .unwrap_or_default(),
        },
    ))
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(class: &str, title: &str, history: Option<i64>, mapped: bool) -> Value {
        let mut json = serde_json::json!({
            "class": class,
            "title": title,
            "address": "0x559e6cdb4a30",
            "mapped": mapped,
            "workspace": { "name": "3" },
        });
        if let Some(history) = history {
            json["focusHistoryID"] = serde_json::json!(history);
        }
        json
    }

    #[test]
    fn the_window_used_last_comes_first() {
        let clients = [
            client("code-oss", "art.rs", Some(1), true),
            client("firefox", "docs", Some(0), true),
        ];
        let windows = sorted(&clients);
        assert_eq!(
            windows.iter().map(|w| w.class.as_str()).collect::<Vec<_>>(),
            ["firefox", "code-oss"]
        );
    }

    #[test]
    fn a_window_that_is_not_on_screen_is_not_offered() {
        let clients = [client("steam", "loading", Some(0), false)];
        assert!(sorted(&clients).is_empty());
    }

    #[test]
    fn a_window_without_a_title_is_labelled_by_its_class() {
        let clients = [client("kitty", "", Some(0), true)];
        assert_eq!(sorted(&clients)[0].label(), "kitty");
    }

    #[test]
    fn a_window_that_was_never_focused_sorts_last() {
        let clients = [
            client("mystery", "never used", None, true),
            client("firefox", "docs", Some(4), true),
        ];
        assert_eq!(sorted(&clients)[0].class, "firefox");
    }

    #[test]
    fn the_workspace_travels_with_the_window() {
        let clients = [client("firefox", "docs", Some(0), true)];
        assert_eq!(sorted(&clients)[0].workspace, "3");
    }
}

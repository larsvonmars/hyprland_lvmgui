//! What Hyprland knows about windows and workspaces.
//!
//! `hyprctl -j clients` is the single source, and it happens to carry everything
//! the two elements that draw windows need: the class (to find an icon), the
//! title (to label a tile), the address (to focus the window on the way out),
//! the size (to draw a tile in the window's own shape), the `stableId` (to
//! capture the window's picture, which is what the overview is made of) and
//! `focusHistoryID` - the field that turns a list of windows into an *alt-tab*
//! order, because 0 is the window you are in, 1 the one you were in before it,
//! and so on.
//!
//! Everything here is what Hyprland said, in Hyprland's terms: this module sorts
//! and filters, it does not decide what makes a good card. That is the element's
//! business.

use crate::hyprctl;
use serde_json::Value;

/// One window, as much as a card needs to know about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    /// `0x…`, which is what a dispatch wants back to focus it.
    pub address: String,
    /// The application's class, e.g. `firefox`. Empty for a window that never
    /// set one (rare, but possible under XWayland).
    pub class: String,
    /// What the window calls itself.
    pub title: String,
    /// The workspace it lives on, by name: `3`, or `special:magic`.
    pub workspace: String,
    /// The identifier the compositor gave this toplevel. It is what a per-window
    /// capture is asked for (`grim -T`, see the overview's `thumbs`), and it is
    /// empty when Hyprland did not answer with one - then there is nothing to
    /// capture by, and the tile has to stand on its icon.
    pub stable_id: String,
    /// The window's size in *layout* pixels, i.e. the size it has on screen
    /// before the monitor's scale factor: `1324x820` here is a `2648x1640`
    /// buffer on a 2x screen. `(0, 0)` when Hyprland reported none.
    pub size: (i32, i32),
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

    /// The window's shape, width ÷ height, so a tile can be drawn in the
    /// window's own proportions. `None` when there is no usable size to take it
    /// from - a window that is still starting up has not been sized yet.
    pub fn aspect(&self) -> Option<f64> {
        let (width, height) = self.size;
        (width > 0 && height > 0).then(|| f64::from(width) / f64::from(height))
    }
}

/// One workspace that exists right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    /// `1`, `2`, … Special workspaces carry negative ids and are left out of
    /// this list: a scratchpad is not one of the spaces you keep windows in.
    pub id: i32,
    /// What it is called, which for an ordinary workspace is its id as text.
    pub name: String,
    /// How many windows it holds - `0` for a workspace you have visited but
    /// emptied, which is a real space you can still switch to.
    pub windows: i32,
}

/// Every mapped window, most recently used first.
///
/// Windows on other workspaces are included on purpose: switching to one is
/// supposed to bring it up, workspace and all.
pub fn list() -> Vec<Window> {
    let Some(clients) = hyprctl::json(&["clients"]) else {
        eprintln!("hypr-osd: hyprctl clients did not answer");
        return Vec::new();
    };
    let Some(clients) = clients.as_array() else {
        eprintln!("hypr-osd: hyprctl clients answered something unexpected");
        return Vec::new();
    };
    sorted(clients)
}

/// Every workspace that exists, by number.
pub fn workspaces() -> Vec<Workspace> {
    let Some(list) = hyprctl::json(&["workspaces"]) else {
        eprintln!("hypr-osd: hyprctl workspaces did not answer");
        return Vec::new();
    };
    let Some(list) = list.as_array() else {
        eprintln!("hypr-osd: hyprctl workspaces answered something unexpected");
        return Vec::new();
    };
    sorted_workspaces(list)
}

/// The name of the workspace that is focused right now (`3`), i.e. the one whose
/// windows are on screen. `None` when hyprctl cannot be asked.
pub fn focused_workspace() -> Option<String> {
    let workspace = hyprctl::json(&["activeworkspace"])?;
    workspace.get("name")?.as_str().map(str::to_owned)
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
            stable_id: text(client, "stableId"),
            size: size(client),
        },
    ))
}

/// `size` arrives as `[width, height]`; anything else is no size at all.
fn size(client: &Value) -> (i32, i32) {
    let number = |index: usize| {
        client
            .get("size")
            .and_then(|size| size.get(index))
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(0)
    };
    (number(0), number(1))
}

/// The workspaces in `list`, by number. Split out for the same reason [`sorted`]
/// is: it is the part with a decision in it.
fn sorted_workspaces(list: &[Value]) -> Vec<Workspace> {
    let mut workspaces: Vec<Workspace> = list.iter().filter_map(workspace_of).collect();
    workspaces.sort_by_key(|workspace| workspace.id);
    workspaces
}

fn workspace_of(workspace: &Value) -> Option<Workspace> {
    let id = workspace.get("id").and_then(Value::as_i64)?;
    // Special workspaces are numbered negatively (`special:magic` is -99).
    let id = i32::try_from(id).ok().filter(|id| *id > 0)?;
    Some(Workspace {
        id,
        name: text(workspace, "name"),
        windows: workspace
            .get("windows")
            .and_then(Value::as_i64)
            .and_then(|count| i32::try_from(count).ok())
            .unwrap_or(0),
    })
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

    #[test]
    fn the_size_and_the_capture_id_travel_with_the_window() {
        let mut json = client("firefox", "docs", Some(0), true);
        json["size"] = serde_json::json!([1324, 820]);
        json["stableId"] = serde_json::json!("1800000b");

        let window = &sorted(&[json])[0];
        assert_eq!(window.size, (1324, 820));
        assert_eq!(window.stable_id, "1800000b");
        // The shape a tile is drawn in: the window's own, not a guess.
        assert!((window.aspect().unwrap() - 1324.0 / 820.0).abs() < 1e-9);
    }

    #[test]
    fn a_window_hyprland_gave_no_size_has_no_shape() {
        let window = &sorted(&[client("steam", "loading", Some(0), true)])[0];
        assert_eq!(window.size, (0, 0));
        assert_eq!(window.aspect(), None);
        assert_eq!(window.stable_id, "");
    }

    fn workspace(id: i64, name: &str, windows: i64) -> Value {
        serde_json::json!({ "id": id, "name": name, "windows": windows })
    }

    #[test]
    fn workspaces_are_ordered_by_number_and_counted() {
        let list = [workspace(3, "3", 2), workspace(1, "1", 0)];
        let workspaces = sorted_workspaces(&list);
        assert_eq!(workspaces.iter().map(|w| w.id).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(workspaces[0].name, "1");
        // An emptied workspace is still a space, and still switchable.
        assert_eq!(workspaces[0].windows, 0);
        assert_eq!(workspaces[1].windows, 2);
    }

    #[test]
    fn a_special_workspace_is_not_a_space_you_keep_windows_in() {
        let list = [workspace(-99, "special:magic", 1), workspace(2, "2", 1)];
        let workspaces = sorted_workspaces(&list);
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].id, 2);
    }
}

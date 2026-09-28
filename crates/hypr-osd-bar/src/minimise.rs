//! Putting a window away, and the tray row that holds it.
//!
//! Hyprland has no minimise. What it has is special workspaces - a scratchpad
//! you toggle onto the screen - and a window moved onto one that is *never*
//! toggled is exactly a minimised window: out of the way, still running, still
//! holding its place in the window list. So that is what this module does: it
//! moves the focused window onto the workspace named by [`PUT_AWAY`], and the bar
//! draws what is on there as icons in the tray's row, beside the application
//! indicators - the one place on this bar where "something that is not on screen"
//! belongs.
//!
//! Three things about the mechanism are not obvious, and each one cost a live
//! experiment in a running session:
//!
//! * **The window stays mapped.** Hyprland reports a hidden window as
//!   `mapped: true`, so `windows::list` cannot tell it apart from a window you can
//!   switch to - the workspace is the only witness. That is why the workspace is a
//!   named constant in the *shared* half: the switcher and the overview have to
//!   agree about it, and `windows::list` hands them only what they can switch to.
//! * **The workspace is *shown* on the way in.** Moving a window onto a special
//!   workspace makes Hyprland display it - that is what the gesture means for a
//!   scratchpad, where you move a window there to look at it - and while it is
//!   displayed the compositor keeps the window it just hid as the *focused* one.
//!   So the window comes straight back on screen and stays there until something
//!   else takes the focus: a minimise is two dispatches, not one, the second being
//!   "hide the workspace again".
//!
//!   What hiding it does to the focus was measured too, because it is not the same
//!   answer everywhere: it hands the focus back to the top window of the workspace
//!   the window came from, *if* that workspace still has one. On a workspace whose
//!   only window is the one being put away there is nothing to hand it to, and
//!   Hyprland goes on considering the hidden window the focused one - which sounds
//!   worse than it is (that workspace has nothing to type into, and the first click
//!   or key press a person makes goes somewhere else), but it is why the bar's
//!   title refuses to name a put-away window, and why nothing here tries to
//!   "restore the focus": `focus({ workspace = … })` on the workspace you are
//!   already on is a no-op, and focusing another one would drag the user's
//!   attention to another screen to fix a detail nobody can see.
//!
//!   The trap in measuring this: a monitor's `activeWorkspace` stays `1` while the
//!   special workspace is drawn *over* it, so an earlier round of tests - which
//!   only ever asked about `activeWorkspace` - reported "hidden" for a window that
//!   was plainly on screen. `specialWorkspace` is the field that means it.
//!
//! * **Hyprland forgets where the window came from.** The special workspace holds
//!   the window, not its history, so the workspace it goes back to is remembered
//!   here, in [`Memory`]. The *list* of icons is not: it is read from Hyprland
//!   every time. A bar that is restarted therefore still shows everything that is
//!   put away, and still offers it back - it has just forgotten where each one
//!   belongs, which the icon's tooltip admits.
//!
//! Deliberately not here: closing a put-away window. A tray icon that kills on a
//! right click is one gesture away from "bring it back", and a window nobody can
//! see is a bad place to lose one by accident.

use hypr_osd_core::hypripc;
use hypr_osd_core::windows::{self, Window, PUT_AWAY};
use serde_json::Value;

/// The events that can change what is put away: a window appearing, going away,
/// or moving between workspaces.
///
/// The list is short on purpose. Everything else the bar hears about (a focus
/// change, a new title) leaves the tray's icons exactly as they are, and
/// `j/clients` is the largest answer Hyprland gives - the one read worth not
/// repeating for nothing.
pub const CHANGES: &[&str] = &["openwindow", "closewindow", "movewindow", "movewindowv2"];

/// What is put away right now, most recently used first.
///
/// Straight from Hyprland rather than from [`Memory`], so the tray survives the
/// bar being restarted and shows everything that is put away, whoever put it
/// there.
///
/// `None` when Hyprland cannot be asked. That is not the same as an empty list:
/// an empty answer would draw an empty tray, which states that nothing is put
/// away - so a caller keeps the icons it has instead.
pub fn read() -> Option<Vec<Window>> {
    // Over Hyprland's own socket, like the bar's other dispatches and readings:
    // a key press must not pay for a process.
    let clients = hypripc::json("j/clients")?;
    let clients = clients.as_array()?;
    Some(windows::split(clients).put_away)
}

/// One window the tray draws: the window itself, and where it goes back to.
#[derive(Clone, Debug)]
pub struct Minimised {
    pub window: Window,
    /// The workspace it was on when it was put away, as far as the bar still
    /// remembers. `None` means "nobody wrote it down" - it then comes back to the
    /// workspace you are on.
    pub home: Option<String>,
}

impl Minimised {
    /// What the icon says on hover: which window this is, where it came from, and
    /// what a click does. Every part of it is a fact the bar has - the workspace
    /// is left unsaid when it is not known rather than guessed at.
    pub fn tooltip(&self) -> String {
        let mut lines = vec![self.window.label().to_string()];
        let class = if self.window.class.is_empty() {
            "no class".to_string()
        } else {
            self.window.class.clone()
        };
        lines.push(match &self.home {
            Some(home) => format!("{class} · put away from workspace {home}"),
            None => format!("{class} · put away before this bar started"),
        });
        lines.push(match &self.home {
            Some(_) => "Left click: bring it back where it was".to_string(),
            None => "Left click: bring it back to the workspace you are on".to_string(),
        });
        lines.join("\n")
    }

    /// Whether `what` names this window: its address, in either spelling. The
    /// `0x` counts because `hyprctl` prints it and a hand that copies one out of
    /// a tooltip or a `status` line may well drop it.
    pub fn matches(&self, what: &str) -> bool {
        let bare = |address: &str| address.trim_start_matches("0x").to_ascii_lowercase();
        !what.is_empty() && bare(&self.window.address) == bare(what)
    }

    /// The one-line form `hypr-osd-bar minimised` prints, and what `status` lists
    /// the icons by.
    pub fn describe(&self) -> String {
        format!(
            "{} ({}) · back to {} · {}",
            self.window.label(),
            if self.window.class.is_empty() {
                "no class"
            } else {
                &self.window.class
            },
            match &self.home {
                Some(home) => format!("workspace {home}"),
                None => "the workspace you are on".to_string(),
            },
            self.window.address
        )
    }
}

/// Where the windows the bar put away came from.
///
/// The bar keeps this for as long as it runs, and nothing on disk: a put-away
/// window is a window *you* put away, and the session in which you did it is the
/// session that knows where it belongs. Losing it to a restart costs a window its
/// old workspace, not its way back - see the module comment.
#[derive(Default)]
pub struct Memory {
    /// Newest first, so the icons read the way a taskbar does: the window you put
    /// away last is the one nearest the indicators.
    homes: Vec<Home>,
}

struct Home {
    address: String,
    workspace: String,
}

impl Memory {
    /// Note where a window came from. Called before it is moved, because
    /// afterwards the compositor no longer knows.
    fn remember(&mut self, address: &str, workspace: &str) {
        // Re-putting away a window that is somehow remembered twice would
        // otherwise leave the stale entry nearest the indicators.
        self.homes.retain(|home| home.address != address);
        self.homes.insert(
            0,
            Home {
                address: address.to_owned(),
                workspace: workspace.to_owned(),
            },
        );
    }

    /// Forget everything that is not put away any more: a window that came back,
    /// and one that was closed while it was away.
    ///
    /// The list read from Hyprland is the truth here, and this is only the
    /// workspace each window belongs to - so it is pruned against it rather than
    /// consulted for it.
    pub fn retain(&mut self, put_away: &[Window]) {
        self.homes
            .retain(|home| put_away.iter().any(|window| window.address == home.address));
    }

    /// The put-away windows in the order the tray draws them, each with the
    /// workspace it goes back to.
    ///
    /// A window this bar did not put away - it was already there when the bar
    /// started, or somebody moved it there by hand - has no home; it keeps
    /// Hyprland's own order (most recently used first) behind the remembered ones
    /// rather than being dropped or guessed at.
    pub fn arrange(&self, put_away: Vec<Window>) -> Vec<Minimised> {
        let mut entries: Vec<(usize, Minimised)> = put_away
            .into_iter()
            .map(|window| {
                let remembered = self
                    .homes
                    .iter()
                    .position(|home| home.address == window.address)
                    .map(|index| (index, self.homes[index].workspace.clone()));
                let (order, home) = match remembered {
                    Some((index, workspace)) => (index, Some(workspace)),
                    None => (usize::MAX, None),
                };
                (order, Minimised { window, home })
            })
            .collect();
        // A stable sort, so the windows without a home stay in the order Hyprland
        // listed them in.
        entries.sort_by_key(|(order, _)| *order);
        entries.into_iter().map(|(_, entry)| entry).collect()
    }
}

/// Put the focused window away - the verb the keybinding runs.
///
/// The answer is what `hypr-osd-bar minimise` prints: the window it put away, or
/// why nothing happened.
pub fn minimise(memory: &mut Memory) -> Result<String, String> {
    let window = focused().ok_or("no window has the focus")?;
    // A window that already sits on a special workspace is put away in the sense
    // that matters, and moving it to ours would trade one hidden workspace for
    // another - including the way back, which would lead to a workspace that is
    // just as hidden.
    if window.workspace.starts_with("special:") {
        return Err(format!(
            "{} is on {} - a special workspace is already put away",
            window.label(),
            window.workspace
        ));
    }
    if !dispatch(&move_to(&window.address, PUT_AWAY)) {
        return Err(format!("Hyprland did not put {} away", window.label()));
    }
    // The move showed the workspace (see the module comment); hiding it again is
    // what actually takes the window off the screen. A window that is put away and
    // visible - which is what one dispatch leaves behind - is the one state this
    // feature must never end in.
    hide_again();
    memory.remember(&window.address, &window.workspace);
    Ok(format!(
        "{} is put away (was on workspace {})",
        window.label(),
        window.workspace
    ))
}

/// Bring a window back: where it was when the bar remembers that, and the
/// workspace you are on otherwise.
///
/// A click on a tray icon must never look like it did nothing, which is why the
/// fallback is "in front of you" rather than "refuse".
pub fn bring_back(entry: &Minimised) -> Result<String, String> {
    let workspace = match &entry.home {
        Some(home) => home.clone(),
        None => {
            // A special workspace is the one wrong answer here: the window would
            // come out of the tray and straight into another hidden workspace,
            // which reads as a click that did nothing at all.
            let workspace =
                focused_workspace().ok_or("Hyprland cannot say which workspace is focused")?;
            if workspace.starts_with("special:") {
                return Err(format!(
                    "{} has no remembered workspace, and {} is a special one",
                    entry.window.label(),
                    workspace
                ));
            }
            workspace
        }
    };
    if !dispatch(&move_to(&entry.window.address, &workspace)) {
        return Err(format!(
            "Hyprland did not bring {} back",
            entry.window.label()
        ));
    }
    // Moving a window takes the focus with it; saying so is what makes the window
    // *yours* afterwards - which is the whole point of clicking it, and the only
    // way it can come up on a monitor that is not the one you are looking at.
    dispatch(&format!(
        "hl.dsp.focus({{ window = \"address:{}\" }})",
        entry.window.address
    ));
    Ok(format!(
        "{} is back on workspace {workspace}",
        entry.window.label()
    ))
}

/// The window that has the focus, or `None` when there is none (Hyprland answers
/// with an empty object on a workspace that holds nothing).
fn focused() -> Option<Window> {
    windows::parse(&hypripc::json("j/activewindow")?)
}

/// Put the put-away workspace back out of sight, if the compositor is showing it.
///
/// Toggling is the only way to hide a special workspace, and toggling one that is
/// *not* shown would put it on screen - so the state is asked for rather than
/// assumed. A read that fails does nothing at all: leaving the workspace as it is
/// is the smaller bug, and it is a bug about *pixels* rather than about state (the
/// window is on the put-away workspace either way, and the tray offers it back).
fn hide_again() {
    if !shows(PUT_AWAY) {
        return;
    }
    dispatch(&format!(
        "hl.dsp.workspace.toggle_special({})",
        workspace_argument(special_name())
    ));
}

/// Whether any monitor is *displaying* the put-away workspace.
fn shows(workspace: &str) -> bool {
    match hypripc::json("j/monitors") {
        Some(monitors) => monitors
            .as_array()
            .is_some_and(|monitors| shows_in(monitors, workspace)),
        None => false,
    }
}

/// The half of [`shows`] with the decision in it.
///
/// `specialWorkspace` is the field that answers "is the window on screen"; the
/// monitor's `activeWorkspace` does not, because displaying a special workspace
/// leaves the active one alone (`1` stays `1`) and draws the special over it -
/// which is exactly how a window that came back on screen passed for hidden once.
fn shows_in(monitors: &[Value], workspace: &str) -> bool {
    monitors.iter().any(|monitor| {
        monitor
            .get("specialWorkspace")
            .and_then(|special| special.get("name"))
            .and_then(Value::as_str)
            == Some(workspace)
    })
}

/// The name a special-workspace dispatcher wants: `toggle_special("magic")`
/// toggles `special:magic`, so the prefix is not part of the name.
fn special_name() -> &'static str {
    PUT_AWAY.strip_prefix("special:").unwrap_or(PUT_AWAY)
}

/// The name of the workspace the focused monitor shows.
fn focused_workspace() -> Option<String> {
    hypripc::json("j/activeworkspace")?
        .get("name")?
        .as_str()
        .map(str::to_owned)
}

/// Move one window to one workspace, in whatever dialect this Hyprland speaks.
///
/// The window is named by address, because the dispatcher would otherwise move
/// *the focused* window - which is right for the key press and wrong for a click
/// on an icon, where the window you are in is a different window entirely.
fn move_to(address: &str, workspace: &str) -> String {
    format!(
        "hl.dsp.window.move({{ workspace = {}, window = \"address:{address}\" }})",
        workspace_argument(workspace)
    )
}

/// A workspace as a dispatcher wants it: a number for a numbered workspace, a
/// quoted string for a name (`special:minimized`, `code`) - the same rule
/// `hyprctl::focus_workspace` follows, for the same reason (a Hyprland configured
/// in Lua takes `3` and `"special:magic"` and would read either one as the other's
/// type if we got it the wrong way round).
fn workspace_argument(workspace: &str) -> String {
    match workspace.parse::<i32>() {
        Ok(id) => id.to_string(),
        Err(_) => format!("\"{workspace}\""),
    }
}

/// Run one dispatcher over Hyprland's socket, and answer whether it went through.
///
/// Hyprland answers `ok` for a dispatch it performed and an `error: …` line for
/// one it refused, so the answer is what is matched - the socket has no exit
/// status, and a refused dispatch would otherwise be invisible.
fn dispatch(expression: &str) -> bool {
    hypripc::request(&format!("dispatch {expression}")).is_some_and(|answer| answer.trim() == "ok")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(address: &str, class: &str) -> Window {
        Window {
            address: address.to_string(),
            class: class.to_string(),
            title: format!("{class} title"),
            workspace: PUT_AWAY.to_string(),
            stable_id: "1800000b".to_string(),
            size: (800, 600),
        }
    }

    #[test]
    fn a_window_nobody_wrote_down_keeps_the_compositors_order() {
        let memory = Memory::default();
        let entries = memory.arrange(vec![window("0x1", "firefox"), window("0x2", "kitty")]);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.window.class.as_str())
                .collect::<Vec<_>>(),
            ["firefox", "kitty"]
        );
        assert!(entries.iter().all(|entry| entry.home.is_none()));
    }

    #[test]
    fn the_window_put_away_last_is_the_first_icon() {
        let mut memory = Memory::default();
        // Kitty went away first, firefox second - so firefox is the icon nearest
        // the indicators, whatever order Hyprland lists the two of them in.
        memory.remember("0x2", "3");
        memory.remember("0x1", "1");
        let entries = memory.arrange(vec![window("0x2", "kitty"), window("0x1", "firefox")]);
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.window.class.as_str(), entry.home.as_deref()))
                .collect::<Vec<_>>(),
            [("firefox", Some("1")), ("kitty", Some("3"))]
        );
    }

    #[test]
    fn a_forgotten_window_sorts_behind_the_remembered_ones() {
        let mut memory = Memory::default();
        memory.remember("0x3", "2");
        let entries = memory.arrange(vec![
            window("0x1", "firefox"),
            window("0x2", "kitty"),
            window("0x3", "code-oss"),
        ]);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.window.class.as_str())
                .collect::<Vec<_>>(),
            ["code-oss", "firefox", "kitty"]
        );
    }

    #[test]
    fn remembering_a_window_twice_leaves_one_home() {
        let mut memory = Memory::default();
        memory.remember("0x1", "1");
        memory.remember("0x2", "2");
        memory.remember("0x1", "1");
        let entries = memory.arrange(vec![window("0x1", "firefox"), window("0x2", "kitty")]);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.window.class.as_str())
                .collect::<Vec<_>>(),
            ["firefox", "kitty"]
        );
    }

    #[test]
    fn a_window_that_came_back_is_forgotten() {
        let mut memory = Memory::default();
        memory.remember("0x1", "1");
        memory.remember("0x2", "2");
        // 0x1 came back (or was closed while it was away), so it is not in the
        // list Hyprland gave the tray any more.
        memory.retain(&[window("0x2", "kitty")]);
        let entries = memory.arrange(vec![window("0x2", "kitty")]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].home.as_deref(), Some("2"));
    }

    #[test]
    fn a_named_workspace_is_a_string_and_a_number_is_a_number() {
        assert_eq!(
            workspace_argument("special:minimized"),
            "\"special:minimized\""
        );
        // A renamed workspace is not a number even when it looks like one.
        assert_eq!(workspace_argument("code"), "\"code\"");
        // The name a special-workspace dispatcher takes is the bare one.
        assert_eq!(special_name(), "minimized");
    }

    #[test]
    fn only_the_shown_special_workspace_counts_as_on_screen() {
        // A monitor as real output describes it: an ordinary workspace is active,
        // and a special workspace is drawn over it.
        let showing = serde_json::json!({
            "name": "eDP-1",
            "activeWorkspace": { "id": 1, "name": "1" },
            "specialWorkspace": { "id": -98, "name": PUT_AWAY },
        });
        assert!(shows_in(std::slice::from_ref(&showing), PUT_AWAY));
        assert!(!shows_in(&[showing], "special:magic"));

        // The trap this field exists for: the *active* workspace is the one the
        // put-away window came from, and it says nothing about the window.
        let hidden = serde_json::json!({
            "name": "eDP-1",
            "activeWorkspace": { "id": 1, "name": "1" },
            "specialWorkspace": { "id": -1, "name": "" },
        });
        assert!(!shows_in(&[hidden], PUT_AWAY));
        // A monitor that has never shown a special workspace answers with an
        // empty name, not a missing field.
        let plain = serde_json::json!({
            "name": "DP-1",
            "activeWorkspace": { "id": 2, "name": "2" },
            "specialWorkspace": { "id": -1, "name": "" },
        });
        assert!(!shows_in(&[plain], PUT_AWAY));
    }

    #[test]
    fn the_move_names_the_window_and_not_the_focused_one() {
        assert_eq!(
            move_to("0x559e6cdb4a30", PUT_AWAY),
            "hl.dsp.window.move({ workspace = \"special:minimized\", window = \"address:0x559e6cdb4a30\" })"
        );
    }

    #[test]
    fn a_window_is_named_by_its_address_either_way_round() {
        let entry = Minimised {
            window: window("0x559E6CDB4A30", "firefox"),
            home: None,
        };
        assert!(entry.matches("0x559E6CDB4A30"));
        assert!(entry.matches("559e6cdb4a30"));
        assert!(!entry.matches("0x559e6cdb4a31"));
        // An empty argument is not a match for a window with no address either:
        // `restore` would otherwise bring back the first window it found.
        assert!(!entry.matches(""));
    }

    #[test]
    fn the_tooltip_admits_when_the_workspace_is_not_known() {
        let entry = Minimised {
            window: window("0x1", "firefox"),
            home: Some("3".to_string()),
        };
        assert!(
            entry.tooltip().contains("put away from workspace 3"),
            "{entry:?}"
        );
        assert!(entry.describe().contains("back to workspace 3"));

        let forgotten = Minimised {
            window: window("0x1", "firefox"),
            home: None,
        };
        assert!(
            forgotten.tooltip().contains("before this bar started"),
            "{}",
            forgotten.tooltip()
        );
        assert!(forgotten.describe().contains("the workspace you are on"));
    }
}

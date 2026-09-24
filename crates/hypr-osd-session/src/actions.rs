//! What the card can do, and how it does it.
//!
//! The five actions are the ones the bar's own session menu offers (see
//! `~/.config/waybar/scripts/popup.py`): the same nerd-font glyphs, the same
//! commands, the same "destructive actions ask twice" rule. Two session menus on
//! one desktop should not disagree about what "Shut down" does, or where its icon
//! is.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use gtk::glib;

/// The glyphs the bar's menu uses for each action.
const GLYPH_LOCK: &str = "\u{f023}";
const GLYPH_SUSPEND: &str = "\u{f186}";
const GLYPH_LOGOUT: &str = "\u{f2f5}";
const GLYPH_REBOOT: &str = "\u{f021}";
const GLYPH_POWER: &str = "\u{f011}";

/// Which action a click leads to, and the verb that runs it from a keybinding.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Id {
    Lock,
    Suspend,
    Logout,
    Reboot,
    PowerOff,
}

impl Id {
    /// The verb an element answers to, for keybindings and scripts.
    pub fn from_verb(verb: &str) -> Option<Id> {
        match verb {
            "lock" => Some(Id::Lock),
            "suspend" => Some(Id::Suspend),
            "logout" | "log-out" => Some(Id::Logout),
            "reboot" => Some(Id::Reboot),
            "poweroff" | "power-off" | "shutdown" => Some(Id::PowerOff),
            _ => None,
        }
    }

    pub fn verb(self) -> &'static str {
        match self {
            Id::Lock => "lock",
            Id::Suspend => "suspend",
            Id::Logout => "logout",
            Id::Reboot => "reboot",
            Id::PowerOff => "poweroff",
        }
    }
}

/// One row of the menu.
pub struct Action {
    pub id: Id,
    pub glyph: &'static str,
    pub label: String,
    /// What the row says once it is armed, or `None` for an action that fires on
    /// the first click.
    pub confirm_label: Option<&'static str>,
    pub danger: bool,
    pub enabled: bool,
    /// What to run, in order. More than one command where an action has a step
    /// before the one you asked for (suspend locks the screen first, so the
    /// laptop does not come back to an open desktop).
    pub commands: Vec<Vec<String>>,
}

impl Action {
    /// One line for the `status` verb: what it would run, or why it is greyed
    /// out. Also the quickest way to answer "why is Lock not clickable?".
    pub fn describe(&self) -> String {
        if !self.enabled {
            return format!(
                "{:<9} disabled (no screen locker: install hyprlock, swaylock or gtklock)",
                self.id.verb()
            );
        }
        let commands = self
            .commands
            .iter()
            .map(|command| command.join(" "))
            .collect::<Vec<_>>()
            .join(" ; ");
        format!("{:<9} {commands}", self.id.verb())
    }
}

/// The menu, in the order the bar's menu lists it.
pub fn all() -> Vec<Action> {
    let locker = locker();

    let mut suspend = Vec::new();
    if let Some(locker) = &locker {
        suspend.push(locker.clone());
    }
    suspend.push(command(&["systemctl", "suspend"]));

    vec![
        Action {
            id: Id::Lock,
            glyph: GLYPH_LOCK,
            // The bar's menu annotates the row when locking is impossible;
            // greying it out silently would look like a bug.
            label: match locker {
                Some(_) => "Lock".to_string(),
                None => "Lock — install hyprlock or swaylock".to_string(),
            },
            confirm_label: None,
            danger: false,
            enabled: locker.is_some(),
            commands: locker.into_iter().collect(),
        },
        Action {
            id: Id::Suspend,
            glyph: GLYPH_SUSPEND,
            label: "Suspend".to_string(),
            confirm_label: None,
            danger: false,
            enabled: true,
            commands: suspend,
        },
        Action {
            id: Id::Logout,
            glyph: GLYPH_LOGOUT,
            label: "Log out (close Hyprland)".to_string(),
            confirm_label: None,
            danger: false,
            enabled: true,
            // The bar's form, fallback included. A shell only because of the
            // `||`: Hyprland's dispatcher accepts both spellings and older
            // versions only the second.
            commands: vec![command(&[
                "sh",
                "-c",
                "hyprctl dispatch 'hl.dsp.exit()' || hyprctl dispatch exit",
            ])],
        },
        Action {
            id: Id::Reboot,
            glyph: GLYPH_REBOOT,
            label: "Reboot".to_string(),
            confirm_label: Some("Click again to reboot"),
            danger: true,
            enabled: true,
            commands: vec![command(&["systemctl", "reboot"])],
        },
        Action {
            id: Id::PowerOff,
            glyph: GLYPH_POWER,
            label: "Shut down".to_string(),
            confirm_label: Some("Click again to shut down"),
            danger: true,
            enabled: true,
            commands: vec![command(&["systemctl", "poweroff"])],
        },
    ]
}

/// The action with this id, as the card would offer it.
pub fn find(id: Id) -> Option<Action> {
    all().into_iter().find(|action| action.id == id)
}

/// Run an action's commands.
///
/// Detached, because several of them outlive this daemon in spirit - `hyprlock`
/// runs until you unlock, `systemctl poweroff` returns while the machine is still
/// going down - and none of them may be waited for on the main loop.
pub fn run(action: &Action) -> Result<(), String> {
    if !action.enabled {
        return Err(format!(
            "{} is not available on this machine",
            action.id.verb()
        ));
    }
    for command in &action.commands {
        let (program, args) = command
            .split_first()
            .ok_or_else(|| "empty command".to_string())?;
        let child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("cannot run {program}: {error}"))?;
        reap(child);
    }
    Ok(())
}

/// Hand a finished child back to GLib, which reaps it - otherwise every action
/// would leave a zombie behind for as long as the daemon runs.
fn reap(child: Child) {
    let pid = glib::Pid(child.id() as i32);
    glib::child_watch_add_local(pid, |_, _| {});
}

/// The first screen locker that exists on this machine, the way the bar's menu
/// picks it.
fn locker() -> Option<Vec<String>> {
    [
        command(&["hyprlock"]),
        command(&["swaylock", "-f"]),
        command(&["gtklock"]),
    ]
    .into_iter()
    .find(|candidate| candidate.first().is_some_and(|program| which(program)))
}

fn command(argv: &[&str]) -> Vec<String> {
    argv.iter().map(|arg| (*arg).to_string()).collect()
}

/// `shutil.which` for Rust: is `program` an executable on `$PATH`?
fn which(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| executable(&directory.join(program)))
}

fn executable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbs_map_to_actions() {
        assert_eq!(Id::from_verb("lock"), Some(Id::Lock));
        assert_eq!(Id::from_verb("poweroff"), Some(Id::PowerOff));
        assert_eq!(Id::from_verb("shutdown"), Some(Id::PowerOff));
        assert_eq!(Id::from_verb("log-out"), Some(Id::Logout));
        assert_eq!(Id::from_verb("toggle"), None);
        assert_eq!(Id::from_verb("up"), None);
    }

    #[test]
    fn destructive_actions_ask_twice() {
        // The one rule that must not regress: a card that appeared under the
        // pointer must not be able to end a session with one click.
        for id in [Id::Reboot, Id::PowerOff] {
            let action = find(id).expect("the menu offers it");
            assert!(action.confirm_label.is_some(), "{id:?} must ask again");
            assert!(action.danger, "{id:?} must be marked destructive");
        }
        for id in [Id::Lock, Id::Suspend, Id::Logout] {
            let action = find(id).expect("the menu offers it");
            assert!(action.confirm_label.is_none(), "{id:?} fires at once");
        }
    }

    #[test]
    fn suspend_locks_first_when_a_locker_exists() {
        let suspend = find(Id::Suspend).expect("the menu offers it");
        let last = suspend.commands.last().expect("at least one command");
        assert_eq!(last, &command(&["systemctl", "suspend"]));
        if let Some(first) = suspend
            .commands
            .first()
            .filter(|_| suspend.commands.len() > 1)
        {
            assert!(matches!(
                first[0].as_str(),
                "hyprlock" | "swaylock" | "gtklock"
            ));
        }
    }

    #[test]
    fn the_lock_row_says_why_it_is_disabled() {
        let lock = find(Id::Lock).expect("the menu offers it");
        if !lock.enabled {
            assert!(lock.label.contains("install"));
            assert!(lock.describe().contains("disabled"));
        }
        // Every action describes itself for `status`.
        for action in all() {
            assert!(!action.describe().is_empty());
        }
    }

    #[test]
    fn programs_on_the_path_are_found() {
        assert!(which("sh"));
        assert!(!which("hypr-osd-definitely-not-installed"));
    }
}

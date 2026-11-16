//! The tiny flags the elements use to talk to each other.
//!
//! Three of this desktop's surfaces belong to different processes and still make
//! one object: the bar's clock pill and the island popup that unfolds beneath it,
//! and the bar's status pill with the system popup under the bar's right end. A
//! popup has to be its own process - a GTK widget can never paint outside its own
//! window, so an "expansion" of a 40px pill *is* a second surface - and the two
//! halves need to agree about whether the panel is on screen: the pill wears the
//! accent while it is.
//!
//! So the panel writes one file into `$XDG_RUNTIME_DIR` and the pill reads it.
//! That is a poor protocol on purpose: plain `1`/`0` text, so it can be written
//! by hand while debugging, survives either process restarting, and needs no
//! state in the other one. A missing file means "closed", which is both the
//! default and the correct answer after a crash.

use std::path::PathBuf;

/// The flag the island popup keeps: `1` while its panel is on screen.
///
/// It lives in the runtime directory rather than a config one because it is
/// session state that should not outlive a logout - a stale `1` in a home
/// directory would light the clock pill up on the next login with nothing
/// behind it.
pub fn island_panel() -> PathBuf {
    runtime_dir().join("hypr-osd-island-panel")
}

/// The flag the system popup keeps: `1` while its panel is on screen. Same
/// protocol, different pill - the bar's status pill reads it to light up.
pub fn stats_panel() -> PathBuf {
    runtime_dir().join("hypr-osd-stats-panel")
}

/// The flag the applications panel keeps: `1` while it is on screen. The bar's
/// left-end button reads it to light up, which is the only thing that tells the
/// two halves apart - the panel is a process of its own (see `state`'s module
/// comment), and the button is what asked for it.
pub fn apps_panel() -> PathBuf {
    runtime_dir().join("hypr-osd-apps-panel")
}

/// The flag the settings window keeps: `1` while it is on screen. The gear in
/// the system popup's header reads it to stay lit, the same way the bar's two
/// buttons read the two flags above.
pub fn settings_panel() -> PathBuf {
    runtime_dir().join("hypr-osd-settings-panel")
}

/// The flag that says the machine is being used as a **tablet** (`1`) rather
/// than as a laptop with its keyboard attached (`0`).
///
/// Unlike the panel flags this one is not a "who is on screen" question but a
/// fact about the machine, and it is a fact two processes have to agree on: the
/// bar and the panel it unfolds saw the same compositor event, but one of them
/// may have started after it. The tablet-mode switch moves once per fold, so the
/// last one to see it writes it down here and the next one to ask reads it
/// instead of guessing from the keyboard. See [`crate::touch`].
pub fn tablet_mode() -> PathBuf {
    runtime_dir().join("hypr-osd-tablet")
}

/// The directory session state belongs in, falling back to `/tmp` the way every
/// other Hyprland client does when `$XDG_RUNTIME_DIR` is unset.
fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Read a flag. Anything unreadable - missing, empty, unparseable - is `false`,
/// because "closed" is the safe answer for every flag here.
pub fn read(path: &PathBuf) -> bool {
    read_opt(path).unwrap_or(false)
}

/// Read a flag that has a *third* state: nothing has said yet.
///
/// The panel flags do not need it ("missing" and "closed" are the same answer),
/// but the tablet flag does: a session that has not seen the switch move yet has
/// no answer, and treating "nothing written" as "laptop" would leave a tablet
/// without its on-screen keyboard until the cover was folded twice.
pub fn read_opt(path: &PathBuf) -> Option<bool> {
    match std::fs::read_to_string(path) {
        Ok(text) => match text.trim() {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        },
        Err(_) => None,
    }
}

/// Write a flag. Failures are ignored deliberately: not being able to say "the
/// panel is open" must never take a panel down.
pub fn write(path: &PathBuf, on: bool) {
    let _ = std::fs::write(path, if on { "1" } else { "0" });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flag has three answers, and the third one - nothing has said yet - is
    /// what the tablet detection depends on: treating a missing file as "laptop"
    /// would leave a folded machine without its keyboard.
    #[test]
    fn a_flag_has_three_states() {
        let path = PathBuf::from(env!("OUT_DIR")).join("state-flag-test");
        let _ = std::fs::remove_file(&path);
        assert_eq!(read_opt(&path), None);
        write(&path, true);
        assert_eq!(read_opt(&path), Some(true));
        write(&path, false);
        assert_eq!(read_opt(&path), Some(false));
        std::fs::write(&path, "nonsense").unwrap();
        assert_eq!(read_opt(&path), None);
        let _ = std::fs::remove_file(&path);
    }
}

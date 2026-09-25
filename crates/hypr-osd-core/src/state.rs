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
    std::fs::read_to_string(path)
        .map(|text| text.trim() == "1")
        .unwrap_or(false)
}

/// Write a flag. Failures are ignored deliberately: not being able to say "the
/// panel is open" must never take a panel down.
pub fn write(path: &PathBuf, on: bool) {
    let _ = std::fs::write(path, if on { "1" } else { "0" });
}

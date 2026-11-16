//! The desktop's wallpaper: hyprpaper's config, generated from the settings
//! window's **Wallpaper** page.
//!
//! hyprpaper is not one of this collection's elements, but its config is one of
//! the desktop's faces - and the one picture the lock screen and the login screen
//! both want to agree with (see [`crate::lock`] and [`crate::login`]). The page
//! therefore edits `~/.config/hypr-osd/wallpaper.conf`, and this module turns
//! that file into hyprpaper's own `~/.config/hypr/hyprpaper.conf`:
//!
//! ```text
//! splash = false
//! ipc = on
//!
//! wallpaper {
//!     monitor =
//!     path = /home/lars/Bilder/Wallpapers/1022522.jpg
//!     fit_mode = cover
//! }
//! ```
//!
//! Every option hyprpaper 0.8 has is carried: the three splash knobs, `ipc`, and
//! the wallpaper block's `monitor`, `path`, `fit_mode`, `timeout`, `order` and
//! `recursive` (the last three turn a directory into a slideshow).
//!
//! `hyprpaper.conf` is written as a **generated** file - its header says so - and
//! a change is applied at once through hyprpaper's IPC (`hyprctl hyprpaper
//! wallpaper "[mon],path[,fit_mode]"`), which is the only request this version
//! takes. The splash and `ipc` are read once at start-up, so the page carries a
//! **Restart hyprpaper** button for those two; the picture itself needs no
//! restart.

use std::path::PathBuf;

use crate::config::{self, Config};

/// The settings file the Wallpaper page edits: `~/.config/hypr-osd/wallpaper.conf`.
pub const FILE: &str = "wallpaper";

/// The generated file, next to the compositor's own config.
pub const CONF: &str = "hyprpaper.conf";

// --- the defaults ----------------------------------------------------------
//
// hyprpaper's own defaults, except for `path`: an empty path means "whatever the
// file that is already there says", which is how the very first write keeps the
// picture this machine already had (see [`current_path`]).

pub const DEFAULT_PATH: &str = "";
pub const DEFAULT_MONITOR: &str = "";
pub const DEFAULT_FIT_MODE: &str = "cover";
pub const DEFAULT_TIMEOUT: &str = "30";
pub const DEFAULT_ORDER: &str = "";
pub const DEFAULT_RECURSIVE: bool = false;
pub const DEFAULT_SPLASH: bool = false;
pub const DEFAULT_SPLASH_OFFSET: &str = "20";
pub const DEFAULT_SPLASH_OPACITY: &str = "0.80";
pub const DEFAULT_IPC: bool = true;

/// The fit modes hyprpaper 0.8 accepts.
pub const FIT_MODES: &[&str] = &["cover", "contain", "tile", "fill"];

/// `~/.config/hypr/hyprpaper.conf` - what [`write`] fills in.
pub fn conf_path() -> PathBuf {
    config::config_home().join("hypr").join(CONF)
}

/// The `path = …` line of a hyprpaper config (the block form; 0.8 and later).
/// Kept here as well as in [`crate::lock`] because this module needs it to keep
/// the picture when the settings file has no opinion, and re-exporting it the
/// other way round would make the two modules depend on each other.
pub fn path_from(text: &str) -> Option<String> {
    let mut found = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "path" {
            let value = value.trim();
            if !value.is_empty() {
                found = Some(value.to_string());
            }
        }
    }
    found
}

/// The picture in force: the settings file's `path`, or the one hyprpaper's own
/// config already names - so opening the page and touching a different row never
/// makes the wallpaper disappear.
pub fn current_path(cfg: &Config) -> String {
    let chosen = text(cfg, "path", DEFAULT_PATH);
    if !chosen.is_empty() {
        return chosen;
    }
    std::fs::read_to_string(conf_path())
        .ok()
        .and_then(|text| path_from(&text))
        .unwrap_or_default()
}

/// Write the generated hyprpaper config. Returns the file, or why it could not be
/// written - the settings page shows that instead of pretending it stuck.
pub fn write() -> Result<PathBuf, String> {
    let path = conf_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    std::fs::write(&path, document())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(path)
}

/// The file, from the machine it is on.
pub fn document() -> String {
    document_from(
        &Config::load(FILE),
        &std::fs::read_to_string(conf_path()).unwrap_or_default(),
    )
}

/// The file, from a config and the hyprpaper config that is already there - which
/// is what makes this testable without a home directory.
pub fn document_from(cfg: &Config, existing: &str) -> String {
    let path = {
        let chosen = text(cfg, "path", DEFAULT_PATH);
        if chosen.is_empty() {
            path_from(existing).unwrap_or_default()
        } else {
            chosen
        }
    };
    let monitor = text(cfg, "monitor", DEFAULT_MONITOR);
    let fit_mode = text(cfg, "fit_mode", DEFAULT_FIT_MODE);
    let timeout = text(cfg, "timeout", DEFAULT_TIMEOUT);
    let order = text(cfg, "order", DEFAULT_ORDER);
    let recursive = flag(cfg, "recursive", DEFAULT_RECURSIVE);

    let mut out = String::from(
        "# GENERATED FILE - do not edit.\n\
         #\n\
         # hyprpaper's config, written by the settings window's Wallpaper page from\n\
         # ~/.config/hypr-osd/wallpaper.conf. Edit that file, or use the page - this one is\n\
         # replaced on the next change (and by `./scripts/install.sh` when hyprpaper is\n\
         # present and this file is missing).\n\
         #\n\
         # hyprpaper 0.8 reads the block form below; the old `preload`/`wallpaper = mon,path`\n\
         # lines are gone. `monitor` empty makes the block the fallback for every output.\n\
         # `path` may also name a directory, in which case `timeout`, `order = random` and\n\
         # `recursive` turn it into a slideshow.\n\
         \n",
    );
    out.push_str(&format!(
        "# The hyprland splash over a freshly loaded wallpaper.\nsplash = {}\n",
        flag(cfg, "splash", DEFAULT_SPLASH)
    ));
    out.push_str(&format!(
        "# How far up the splash sits, and how opaque it is.\nsplash_offset = {}\nsplash_opacity = {}\n",
        text(cfg, "splash_offset", DEFAULT_SPLASH_OFFSET),
        text(cfg, "splash_opacity", DEFAULT_SPLASH_OPACITY),
    ));
    out.push_str(&format!(
        "# On, so `hyprctl hyprpaper wallpaper` and `listactive` work.\nipc = {}\n",
        if cfg.bool("ipc", DEFAULT_IPC) {
            "on"
        } else {
            "off"
        },
    ));
    out.push_str("\nwallpaper {\n");
    out.push_str(&format!("    monitor = {monitor}\n"));
    if path.is_empty() {
        out.push_str("    # No picture chosen yet - hyprpaper keeps its own default.\n");
    } else {
        out.push_str(&format!("    path = {path}\n"));
    }
    out.push_str(&format!("    fit_mode = {fit_mode}\n"));
    out.push_str(&format!("    timeout = {timeout}\n"));
    if !order.is_empty() {
        out.push_str(&format!("    order = {order}\n"));
    }
    out.push_str(&format!("    recursive = {recursive}\n"));
    out.push_str("}\n");
    out
}

/// A knob's value, or the default when the file does not set it (an empty value
/// is "not set", like every other element's config).
fn text(cfg: &Config, key: &str, default: &str) -> String {
    cfg.get(key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
        .to_string()
}

fn flag(cfg: &Config, key: &str, default: bool) -> &'static str {
    if cfg.bool(key, default) {
        "true"
    } else {
        "false"
    }
}

/// The argument for `hyprctl hyprpaper wallpaper`: `[mon],path[,fit_mode]`.
/// `None` when there is no picture (which is not an error - a machine may simply
/// have no wallpaper file yet).
pub fn ipc_argument(cfg: &Config) -> Option<String> {
    ipc_argument_with(cfg, &current_path(cfg))
}

/// The same, with the picture handed in - which is what makes it testable without
/// this machine's own hyprpaper config.
pub fn ipc_argument_with(cfg: &Config, path: &str) -> Option<String> {
    if path.trim().is_empty() {
        return None;
    }
    let monitor = text(cfg, "monitor", DEFAULT_MONITOR);
    let fit = text(cfg, "fit_mode", DEFAULT_FIT_MODE);
    Some(format!("{monitor},{path},{fit}"))
}

/// Whether the daemon is up. `false` is the useful answer: a picture change then
/// writes a config nobody is reading, which is exactly what the page's Restart
/// button is for.
pub fn running() -> bool {
    std::process::Command::new("pgrep")
        .args(["-x", "hyprpaper"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Whether the program itself is installed, so the page can tell "not running"
/// from "not there at all".
pub fn installed() -> bool {
    crate::hardware::installed("hyprpaper")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config file of its own per call: the tests run in parallel, and a shared
    /// name would have one test reading another's file.
    fn config(text: &str) -> Config {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);

        let path = std::env::temp_dir().join(format!(
            "hypr-osd-wallpaper-test-{}-{}.conf",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, text).unwrap();
        Config::load_from(path)
    }

    /// An untouched file keeps the picture hyprpaper already has, and writes the
    /// rest of hyprpaper's defaults.
    #[test]
    fn the_defaults_keep_the_picture_that_is_there() {
        let existing =
            "splash = false\n\nwallpaper {\n    monitor =\n    path = /home/lars/x.jpg\n}\n";
        let doc = document_from(&config(""), existing);
        assert!(doc.contains("path = /home/lars/x.jpg\n"), "{doc}");
        assert!(doc.contains("fit_mode = cover\n"), "{doc}");
        assert!(doc.contains("splash = false\n"), "{doc}");
        assert!(doc.contains("ipc = on\n"), "{doc}");
    }

    /// The file wins over the picture that is already there, and every slideshow
    /// knob is carried.
    #[test]
    fn the_file_wins_and_carries_every_option() {
        let doc = document_from(
            &config(
                "path = /tmp/wall\nfit_mode = contain\nmonitor = DP-1\nsplash = true\n\
                 splash_offset = 40\nsplash_opacity = 0.5\nipc = false\ntimeout = 60\n\
                 order = random\nrecursive = true\n",
            ),
            "wallpaper {\n    path = /nope.jpg\n}\n",
        );
        for line in [
            "path = /tmp/wall",
            "fit_mode = contain",
            "monitor = DP-1",
            "splash = true",
            "splash_offset = 40",
            "splash_opacity = 0.5",
            "ipc = off",
            "timeout = 60",
            "order = random",
            "recursive = true",
        ] {
            assert!(doc.contains(line), "missing `{line}` in:\n{doc}");
        }
    }

    /// The IPC argument is what a change applies at once, and it is `None` when
    /// there is no picture at all.
    #[test]
    fn the_ipc_argument_is_the_hyprpaper_form() {
        let cfg = config("path = /tmp/a.jpg\nmonitor = eDP-1\nfit_mode = fill\n");
        assert_eq!(
            ipc_argument_with(&cfg, "/tmp/a.jpg").as_deref(),
            Some("eDP-1,/tmp/a.jpg,fill")
        );
        assert_eq!(ipc_argument_with(&config("monitor = DP-1\n"), ""), None);
    }
}

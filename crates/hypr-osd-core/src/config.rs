//! Per-element settings, read from a tiny `key = value` file.
//!
//! `~/.config/hypr-osd/<element>.conf`, because an OSD is the kind of thing
//! you want to nudge (how long it stays, how far up the screen it sits)
//! without rebuilding it. Everything has a default in the element's own source,
//! so the file is optional and stays optional.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// The directory (under `$XDG_CONFIG_HOME`) holding one file per element.
pub const DIR: &str = "hypr-osd";

pub struct Config {
    /// Where it was read from - printed by `--help`-ish paths and by errors, so
    /// "I edited the wrong file" is immediately visible.
    pub path: PathBuf,
    values: HashMap<String, String>,
}

impl Config {
    /// Load `<element>.conf`; a missing file is not an error.
    pub fn load(element: &str) -> Self {
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .unwrap_or_else(|| PathBuf::from(".config"));
        Self::load_from(config_home.join(DIR).join(format!("{element}.conf")))
    }

    pub fn load_from(path: PathBuf) -> Self {
        let mut values = HashMap::new();
        if let Ok(text) = fs::read_to_string(&path) {
            for line in text.lines() {
                // `#` starts a comment, including at the end of a line.
                let line = line.split('#').next().unwrap_or_default().trim();
                if let Some((key, value)) = line.split_once('=') {
                    values.insert(key.trim().to_string(), value.trim().to_string());
                }
            }
        }
        Config { path, values }
    }

    /// A single value, if it is set and parseable.
    fn parse<T: std::str::FromStr>(&self, key: &str) -> Option<T> {
        self.values.get(key)?.parse().ok()
    }

    /// A fraction-ish setting, e.g. `bottom_margin = 0.12`.
    pub fn float(&self, key: &str, default: f64) -> f64 {
        self.parse(key).unwrap_or(default)
    }

    /// A pixel-ish setting, e.g. `width = 340`.
    pub fn i32(&self, key: &str, default: i32) -> i32 {
        self.parse(key).unwrap_or(default)
    }

    /// A duration in milliseconds.
    pub fn millis(&self, key: &str, default: u64) -> std::time::Duration {
        std::time::Duration::from_millis(self.parse(key).unwrap_or(default))
    }

    /// `true`/`yes`/`on`/`1` are true, anything else (including absent) is not.
    pub fn bool(&self, key: &str, default: bool) -> bool {
        self.values
            .get(key)
            .map(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "true" | "yes" | "on" | "1"
                )
            })
            .unwrap_or(default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Config {
        let path = std::env::temp_dir().join("hypr-osd-config-test.conf");
        fs::write(&path, text).unwrap();
        Config::load_from(path)
    }

    #[test]
    fn reads_values_and_ignores_comments_and_junk() {
        let cfg = parse(
            "# a comment\n\
             width = 340   # trailing comment\n\
             bottom_margin = 0.12\n\
             unmute_on_raise = true\n\
             nonsense\n\
             broken = abc\n",
        );
        assert_eq!(cfg.i32("width", 0), 340);
        assert!((cfg.float("bottom_margin", 0.0) - 0.12).abs() < 1e-9);
        assert!(cfg.bool("unmute_on_raise", false));
        // Unparseable or unknown keys fall back to the caller's default.
        assert_eq!(cfg.i32("broken", 7), 7);
        assert_eq!(cfg.i32("missing", 7), 7);
        assert!(!cfg.bool("missing", false));
    }
}

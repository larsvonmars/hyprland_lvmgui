//! Per-element settings, read from a tiny `key = value` file.
//!
//! `~/.config/hypr-osd/<element>.conf`, because an OSD is the kind of thing
//! you want to nudge (how long it stays, how far up the screen it sits)
//! without rebuilding it. Everything has a default in the element's own source,
//! so the file is optional and stays optional.
//!
//! The file is *meant* to be edited by hand - every template says so - and one
//! program edits it for you: [`edit_value`] is what the settings window writes
//! with, and it is careful about the two things an editor has to be careful
//! about. It touches one line and leaves every other line (comments, blank
//! lines, the order) exactly where it was, and a value that is set back to its
//! default is *removed* rather than written out, because in this format an
//! absent key means the built-in default - writing the default down would turn a
//! setting that follows the element into one that pins it.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The directory (under `$XDG_CONFIG_HOME`) holding one file per element.
pub const DIR: &str = "hypr-osd";

/// `$XDG_CONFIG_HOME`, or `~/.config`. Every path in this collection is built
/// from it, and the settings window asks for it directly: it writes one file
/// that is *not* an element config (`~/.config/hypr/hypr-osd-desktop.lua`).
pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
}

/// The directory the elements' config files live in: `~/.config/hypr-osd`.
pub fn dir() -> PathBuf {
    config_home().join(DIR)
}

pub struct Config {
    /// Where it was read from - printed by `--help`-ish paths and by errors, so
    /// "I edited the wrong file" is immediately visible.
    pub path: PathBuf,
    values: HashMap<String, String>,
}

impl Config {
    /// Load `<file>.conf`; a missing file is not an error.
    pub fn load(file: &str) -> Self {
        Self::load_from(dir().join(format!("{file}.conf")))
    }

    pub fn load_from(path: PathBuf) -> Self {
        let mut values = HashMap::new();
        if let Ok(text) = fs::read_to_string(&path) {
            for line in text.lines() {
                let line = line.trim();
                // A line whose first character is `#` is a comment, including a
                // commented-out key (`# width = 300`), which must not be read as a
                // key called `# width`.
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let Some((key, rest)) = line.split_once('=') else {
                    continue;
                };
                values.insert(
                    key.trim().to_string(),
                    strip_comment(rest).trim().to_string(),
                );
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

    /// A text setting, e.g. `clock_format = %H:%M`.
    ///
    /// An empty value means "the default", like every other key. A value may
    /// begin with `#` (a colour: `accent = #33ccff`) - a `#` only starts an
    /// inline comment once there is a value before it, so the theme's colours can
    /// be written in their own spelling. A value that needs a literal ` #` still
    /// cannot have one.
    pub fn string(&self, key: &str, default: &str) -> String {
        self.values
            .get(key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| default.to_string())
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

    /// The text of one value, exactly as the file spells it, or `None` when the
    /// file does not set that key at all.
    ///
    /// The typed getters above answer "what is this worth"; this answers "is it
    /// written down", which is the question a settings window has to ask: an
    /// absent key is a setting that still follows the element's default, and
    /// showing the default as if it had been chosen would be a lie.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// Every key the file sets, sorted, with its value - the keys a settings
    /// window shows beyond the ones its catalogue knows by name.
    pub fn entries(&self) -> Vec<(String, String)> {
        let mut all: Vec<(String, String)> = self
            .values
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        all.sort();
        all
    }
}

/// Set one key in a config file, or remove it, leaving the rest of the file
/// alone.
///
/// `Some(value)` writes `key = value` (in place when the key is already there,
/// at the end of the file otherwise); `None` deletes the key's line, which is
/// how a value goes back to the element's built-in default.
///
/// A missing file is not an error - a key appended to nothing is a config file
/// with one line in it, which is exactly what setting a value on a fresh install
/// should produce. Only the first line matching the key is touched (the reader
/// lets a later one win, so a file that contradicts itself stays as the user
/// wrote it).
///
/// The one thing this does not preserve is a *trailing* comment on the line it
/// rewrites (`height = 40  # a bit tall`): the value is replaced whole. Every
/// other line - including the commented-out keys the templates carry - is left
/// exactly where it was. (One exception, and it is there on purpose: a blank line
/// that an earlier append put in front of the key goes back out with it, so
/// setting and unsetting a value a few times does not fill the file with gaps.)
pub fn edit_value(path: &Path, key: &str, value: Option<&str>) -> std::io::Result<()> {
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut found = false;
    let mut dropped_at: Option<usize> = None;

    for line in text.lines() {
        // A commented-out key is not that key: `# height = 40` has to stay a
        // comment, or the settings window would silently "uncomment" it.
        let body = line.split('#').next().unwrap_or_default();
        let is_ours = body
            .split_once('=')
            .is_some_and(|(name, _)| name.trim() == key);
        if !is_ours {
            lines.push(line.to_string());
            continue;
        }
        found = true;
        if let Some(value) = value {
            lines.push(format!("{key} = {value}"));
        } else {
            // `None`: the line is dropped, and where it was is remembered so the
            // blank line above it can go too.
            dropped_at = Some(lines.len());
        }
    }

    if let Some(value) = value {
        if !found {
            // A blank line before the new key, so a hand-written file does not
            // end up with the setting glued to whatever came last.
            if lines.last().is_some_and(|line| !line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(format!("{key} = {value}"));
        }
    } else if let Some(at) = dropped_at {
        let above_is_blank = at > 0 && lines[at - 1].trim().is_empty();
        let below_is_blank = match lines.get(at) {
            Some(line) => line.trim().is_empty(),
            None => true,
        };
        if above_is_blank && below_is_blank {
            lines.remove(at - 1);
        }
    }

    let mut body = lines.join("\n");
    body.push('\n');
    fs::write(path, body)
}

/// The value part of a line, without an inline comment.
///
/// A `#` is only a comment once there is a value in front of it: the first one in
/// `accent = #33ccff` has nothing before it and is the colour's own spelling,
/// while the one in `height = 40  # tall` follows a value and ends the line. A
/// second `#` after a colour (`accent = #33ccff  # blue`) is therefore still a
/// comment.
fn strip_comment(value: &str) -> &str {
    let mut from = 0;
    while let Some(position) = value[from..].find('#') {
        let at = from + position;
        if !value[..at].trim().is_empty() {
            return &value[..at];
        }
        from = at + 1;
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Config {
        // A file of its own per call: the tests run in parallel, and two of them
        // sharing one path made this flaky.
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);

        let path = std::env::temp_dir().join(format!(
            "hypr-osd-config-test-{}-{}.conf",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
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

    /// A colour is a value that begins with `#`, which used to be indistinguishable
    /// from a comment - so the login screen's palette could not be written at all.
    #[test]
    fn a_value_may_begin_with_a_hash() {
        let cfg = parse(
            "accent = #33ccff\n\
             fg = #dde7ef  # a trailing comment still ends the line\n\
             # height = 40\n",
        );
        assert_eq!(cfg.get("accent"), Some("#33ccff"));
        assert_eq!(cfg.get("fg"), Some("#dde7ef"));
        assert_eq!(cfg.get("height"), None);
    }

    #[test]
    fn raw_reads_tell_set_from_default() {
        let cfg = parse("height = 40\nempty =\n");
        // The typed getter cannot tell a set value from a default; `get` can.
        assert_eq!(cfg.get("height"), Some("40"));
        assert_eq!(cfg.get("margin_top"), None);
        assert_eq!(
            cfg.entries(),
            vec![("empty".into(), "".into()), ("height".into(), "40".into())]
        );
    }

    /// The writer the settings window uses: it rewrites one line and leaves the
    /// rest of the file - comments, order, the keys it was not asked about -
    /// exactly where it was.
    #[test]
    fn editing_a_value_touches_one_line() {
        let path = std::env::temp_dir().join("hypr-osd-config-edit-test.conf");
        fs::write(
            &path,
            "# a comment\nheight = 40  # a bit tall\nmargin_x = 12\n\nexclusive = true\n",
        )
        .unwrap();

        // In place: the comment above and the keys around it survive.
        edit_value(&path, "margin_x", Some("24")).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# a comment\nheight = 40"));
        assert!(text.contains("margin_x = 24"));
        assert!(text.contains("exclusive = true"));

        // A new key is appended, with a blank line before it.
        edit_value(&path, "workspaces", Some("8")).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\nexclusive = true\n\nworkspaces = 8\n"));

        // Removing a key deletes its line, so the default applies again.
        edit_value(&path, "margin_x", None).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("margin_x"));

        // The line is deleted whole - trailing comment and all.
        edit_value(&path, "height", None).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("height"),
            "the line should be gone: {text:?}"
        );

        let _ = fs::remove_file(&path);
    }

    /// A commented-out key is a comment, not a setting: writing one appends the
    /// real key rather than "uncommenting" the line.
    #[test]
    fn a_commented_out_key_stays_a_comment() {
        let path = std::env::temp_dir().join("hypr-osd-config-comment-test.conf");
        fs::write(&path, "# height = 40\noutput =\n").unwrap();
        edit_value(&path, "height", Some("48")).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# height = 40"));
        assert!(text.contains("height = 48"));
        let _ = fs::remove_file(&path);
    }

    /// Setting a value on a machine that has no config file yet is not an error:
    /// it produces a file with that one line.
    #[test]
    fn editing_a_missing_file_creates_it() {
        let path = std::env::temp_dir().join("hypr-osd-config-create-test.conf");
        let _ = fs::remove_file(&path);
        edit_value(&path, "width", Some("700")).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "width = 700\n");
        let _ = fs::remove_file(&path);
    }

    /// `set` then `unset` leaves the file as it started: the blank line an append
    /// put in front of the key comes back out with it.
    #[test]
    fn setting_and_unsetting_leaves_no_gaps() {
        let path = std::env::temp_dir().join("hypr-osd-config-roundtrip-test.conf");
        fs::write(&path, "height = 40\n").unwrap();
        edit_value(&path, "width", Some("700")).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "height = 40\n\nwidth = 700\n"
        );
        edit_value(&path, "width", None).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "height = 40\n");
        // A blank line that was *already* there stays where it was.
        fs::write(&path, "height = 40\n\n# a note\n").unwrap();
        edit_value(&path, "height", None).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "\n# a note\n");
        let _ = fs::remove_file(&path);
    }
}

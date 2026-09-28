//! The pinned applications: a plain list, one desktop id per line.
//!
//! It lives in `~/.config/hypr-osd/apps-pinned` - a file of its own rather than a
//! line in `apps.conf`, because it is *written* by the panel while the config is
//! read from the user: an element that rewrote its own config file would undo the
//! comments that explain it.
//!
//! Text, not JSON: one id per line is readable, editable by hand and diffable, and
//! `#` starts a comment, so a line can be retired without deleting it. An id is
//! the desktop file's name without `.desktop` (`firefox`, `code-oss`) - the same
//! identity the rest of the collection uses for an application (see
//! `hypr_osd_core::apps`).

use std::path::PathBuf;

/// The file the pins live in.
pub fn path() -> PathBuf {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    config_home
        .join(hypr_osd_core::config::DIR)
        .join("apps-pinned")
}

/// One id per line, with `#` comments and blanks ignored and duplicates dropped
/// (a pin that appears twice would be two rows in the pinned drawer).
pub fn parse(text: &str) -> Vec<String> {
    let mut pins: Vec<String> = Vec::new();
    for line in text.lines() {
        let id = line.split('#').next().unwrap_or_default().trim();
        if id.is_empty() || pins.iter().any(|known| known == id) {
            continue;
        }
        pins.push(id.to_string());
    }
    pins
}

/// The file's contents for a list of pins: one per line, newline-terminated so
/// the file ends like a text file should.
pub fn format(pins: &[String]) -> String {
    if pins.is_empty() {
        return String::new();
    }
    let mut text = pins.join("\n");
    text.push('\n');
    text
}

/// Read the pins. A missing or unreadable file is "nothing is pinned", which is
/// both the default and the right answer after a first start.
pub fn load() -> Vec<String> {
    std::fs::read_to_string(path())
        .map(|text| parse(&text))
        .unwrap_or_default()
}

/// Write the pins. Failures are ignored deliberately: not being able to remember
/// a pin must never take the panel down (the same rule the runtime flags follow).
pub fn save(pins: &[String]) {
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, format(pins));
}

/// Pin an application, or unpin it, and answer with the new state.
pub fn toggle(pins: &mut Vec<String>, id: &str) -> bool {
    match pins.iter().position(|known| known == id) {
        Some(position) => {
            pins.remove(position);
            false
        }
        None => {
            pins.push(id.to_string());
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_blanks_and_duplicates_are_dropped() {
        let pins = parse("# what I reach for\nfirefox\n\n  alacritty  # a terminal\nfirefox\n");
        assert_eq!(pins, vec!["firefox", "alacritty"]);
    }

    #[test]
    fn a_list_round_trips_through_the_file_format() {
        let pins = vec!["firefox".to_string(), "code-oss".to_string()];
        assert_eq!(format(&pins), "firefox\ncode-oss\n");
        assert_eq!(parse(&format(&pins)), pins);
        // Nothing pinned is an empty file, not a stray newline.
        assert_eq!(format(&[]), "");
        assert!(parse("").is_empty());
    }

    #[test]
    fn toggling_pins_and_unpins() {
        let mut pins: Vec<String> = Vec::new();
        assert!(toggle(&mut pins, "firefox"));
        assert_eq!(pins, vec!["firefox"]);
        assert!(toggle(&mut pins, "kitty"));
        assert_eq!(pins, vec!["firefox", "kitty"]);
        // The same call takes it back out - it is a toggle, not a pin.
        assert!(!toggle(&mut pins, "firefox"));
        assert_eq!(pins, vec!["kitty"]);
        // And a third time pins it again, at the end of the list: the order is
        // the order things were pinned in, which is what the drawer shows.
        assert!(toggle(&mut pins, "firefox"));
        assert_eq!(pins, vec!["kitty", "firefox"]);
    }
}

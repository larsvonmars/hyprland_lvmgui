//! Application icons: a window's class turned into a picture a card can draw.
//!
//! Two hops, each with a fallback, because neither one is reliable on its own:
//!
//! 1. **class → icon name**: the freedesktop entries in the usual places. A
//!    window's class is usually the entry's file name (`firefox` →
//!    `firefox.desktop`), but Electron apps and Steam set a class of their own
//!    and point `StartupWMClass` at it, which is the second way in. The user's
//!    own directory wins over the system's, exactly like freedesktop says.
//! 2. **icon name → picture**: GTK's own icon theme, so an element shows the same
//!    artwork as the rest of the desktop, in whatever theme is configured.
//!    `Icon=` may also be an absolute path, which is the file itself and not a
//!    lookup at all.
//!
//! Both hops are cached: this process lives for the whole session, and the
//! answer only changes if an application is installed while it is running.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use gtk::gdk;
use gtk::prelude::*;

/// class (lowercased) → icon, built once from the desktop entries on this
/// machine.
fn index() -> &'static HashMap<String, String> {
    static INDEX: OnceLock<HashMap<String, String>> = OnceLock::new();
    INDEX.get_or_init(|| build_index(&desktop_dirs()))
}

/// The picture for a window class, or `None` when there is nothing to draw -
/// the view keeps its letter tile then.
///
/// A paintable rather than a texture, so a themed icon stays whatever GTK found
/// (a PNG, an SVG, a symbolic sheet) instead of being decoded here.
pub fn paintable(class: &str, size: i32) -> Option<gdk::Paintable> {
    let name = icon_name(class)?;
    Some(cached(class, by_name(&name, size)?))
}

/// The picture for an icon *name* - the second hop of [`paintable`], split out
/// because the tray starts one hop in: a StatusNotifierItem hands over a name
/// that is already an icon name, not a window class to look up in the desktop
/// entries. (A class can be looked up by name too, which is why this is a
/// separate entry point rather than a different implementation.)
///
/// An absolute path is the file itself and not a lookup at all: the
/// freedesktop spec allows `Icon=` to be one, and applications that ship their
/// own artwork use it.
pub fn by_name(name: &str, size: i32) -> Option<gdk::Paintable> {
    if name.trim().is_empty() {
        return None;
    }
    if name.starts_with('/') {
        let texture = gdk::Texture::from_filename(Path::new(name)).ok()?;
        return Some(texture.upcast());
    }
    let display = gdk::Display::default()?;
    let theme = gtk::IconTheme::for_display(&display);
    // The second argument is GTK's own fallback list; an element has its own
    // idea of a fallback (the letter tile), so it stays empty.
    let found = theme.lookup_icon(
        name,
        &[],
        size,
        1,
        gtk::TextDirection::None,
        gtk::IconLookupFlags::empty(),
    );
    // GTK answers with *something* even for a name no theme knows - the
    // "image missing" icon. Only an icon that is the one that was asked for
    // counts; anything else is left to the caller's own fallback. (The binding
    // hands the name back as a path, hence the comparison in those terms.)
    if found.icon_name().as_deref() != Some(Path::new(name)) {
        return None;
    }
    Some(found.upcast())
}

/// The icon *name* for a window class.
///
/// With no desktop entry claiming the class, the class itself is the best guess,
/// because it is the icon name for plenty of applications; the tile's letter
/// covers whatever the theme does not have.
pub fn icon_name(class: &str) -> Option<String> {
    if class.trim().is_empty() {
        return None;
    }
    if let Some(icon) = index().get(&class.to_lowercase()) {
        return Some(icon.clone());
    }
    Some(class.to_string())
}

/// Read every desktop entry and map the names it answers to onto its icon.
///
/// A separate function from [`index`] so a test can point it at a directory of
/// its own; the real one only ever sees the freedesktop locations.
fn build_index(dirs: &[PathBuf]) -> HashMap<String, String> {
    let mut index = HashMap::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        // Sorted, so two entries claiming the same class resolve the same way on
        // every run: "whichever the filesystem listed first" is not an answer.
        let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        paths.sort();
        for path in paths {
            if path.extension().and_then(|extension| extension.to_str()) != Some("desktop") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(icon) = parse_string(&text, "Icon") else {
                continue;
            };
            if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                index
                    .entry(stem.to_lowercase())
                    .or_insert_with(|| icon.clone());
            }
            if let Some(class) = parse_string(&text, "StartupWMClass") {
                index.entry(class.to_lowercase()).or_insert(icon);
            }
        }
    }
    index
}

/// The freedesktop application directories, in the order they win.
fn desktop_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"));
    dirs.push(data_home.join("applications"));
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    for dir in data_dirs.split(':').filter(|dir| !dir.is_empty()) {
        dirs.push(PathBuf::from(dir).join("applications"));
    }
    dirs
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Pull a `Key=value` out of the file's `[Desktop Entry]` group.
///
/// The group matters: a desktop file has other groups too (actions,
/// translations) and an `Icon=` in one of those is not the application's icon.
fn parse_string(text: &str, key: &str) -> Option<String> {
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some((name, value)) = line.split_once('=') {
            let value = value.trim();
            if name.trim() == key && !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Textures and paintables already resolved, by class. Going through the theme
/// costs a few milliseconds per application, and a card is opened far more often
/// than an application is installed, so paying it once per class is enough.
fn cached(class: &str, paintable: gdk::Paintable) -> gdk::Paintable {
    thread_local! {
        static CACHE: RefCell<HashMap<String, gdk::Paintable>> = RefCell::new(HashMap::new());
    }
    CACHE.with(|cache| {
        cache
            .borrow_mut()
            .entry(class.to_lowercase())
            .or_insert(paintable)
            .clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIREFOX: &str = "[Desktop Entry]\nName=Firefox\nIcon=firefox\n";

    /// An entry whose class does not match its file name - what Electron and
    /// Steam do, and the reason the second lookup exists.
    const STEAM: &str = "\
[Desktop Entry]
Name=Steam
Icon=steam
StartupWMClass=Steam

[Desktop Action Library]
Name=Library
Icon=should-not-win
";

    #[test]
    fn the_icon_is_read_from_the_desktop_entry_group() {
        assert_eq!(parse_string(FIREFOX, "Icon").as_deref(), Some("firefox"));
    }

    #[test]
    fn an_icon_in_another_group_does_not_count() {
        // The only `Icon=` is inside `[Desktop Action …]`, so the entry has none.
        assert_eq!(parse_string(STEAM, "Icon").as_deref(), Some("steam"));
        let action_only = "[Desktop Entry]\nName=X\n\n[Desktop Action Y]\nIcon=nope\n";
        assert_eq!(parse_string(action_only, "Icon"), None);
    }

    #[test]
    fn an_empty_value_is_no_value() {
        assert_eq!(parse_string("[Desktop Entry]\nIcon=\n", "Icon"), None);
    }

    #[test]
    fn the_file_name_and_the_startup_class_both_find_the_entry() {
        let dir = std::env::temp_dir().join(format!("hypr-osd-icons-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("steam.desktop"), STEAM).expect("write");
        std::fs::write(dir.join("Firefox.desktop"), FIREFOX).expect("write");

        let index = build_index(std::slice::from_ref(&dir));
        // by file name
        assert_eq!(index.get("firefox").map(String::as_str), Some("firefox"));
        // by class, case-insensitively
        assert_eq!(index.get("steam").map(String::as_str), Some("steam"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_class_with_no_entry_falls_back_to_the_class_itself() {
        // `icon_name` must answer for classes nobody installed an entry for:
        // the icon theme still gets a chance, then the view's letter tile.
        assert_eq!(
            icon_name("nonexistent-app-xyz").as_deref(),
            Some("nonexistent-app-xyz")
        );
        // …and refuse to answer for a class that is not there at all.
        assert_eq!(icon_name("  "), None);
    }
}

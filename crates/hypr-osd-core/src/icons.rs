//! Icons: the ones this collection draws itself, and the ones it borrows from
//! the applications on this machine.
//!
//! # Our own: Lucide, in the binary
//!
//! [`lucide`] is what an element reaches for when it wants to draw an icon of
//! its own. The drawings are Lucide SVG files, baked into this crate as a
//! GResource (see `icons/README.md`) and looked up through GTK's icon theme as
//! *symbolic* icons - which is what makes an icon take the colour of the CSS
//! `color` it sits under, exactly like the text beside it. [`names`] is the
//! vocabulary: one constant per drawing, so the speaker is the same icon in the
//! bar, in the volume card and in the system popup.
//!
//! # Borrowed: an application's own icon
//!
//! [`paintable`] answers the other question a card asks - "what does this
//! *window* look like?" - in two hops, each with a fallback, because neither one
//! is reliable on its own:
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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Once, OnceLock};

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

// ---------------------------------------------------------------------------
// Lucide: the icons this collection draws itself
// ---------------------------------------------------------------------------

/// Where the bundled icons live inside the process. Not a free choice: GTK's
/// icon theme only reads a resource path as *hicolor*, in the subdirectories
/// hicolor declares - so the path ends in `scalable/actions/`, and every icon
/// in it is named `<name>-symbolic.svg`. See `icons/README.md` and `build.rs`.
const RESOURCE: &str = "/com/schells2/osd/icons";

/// Link the icon bundle into this process. Idempotent and almost free, so an
/// element may call it wherever it is convenient; `osd::run` calls it at
/// start-up, before the first card is built.
pub fn register() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        gtk::gio::resources_register_include!("lucide.gresource")
            .expect("the Lucide icon bundle is linked into hypr-osd-core");
    });
}

/// Point this process's icon theme at the bundle, once there is a display to
/// point.
///
/// Deferred to the first icon rather than done in [`register`], because the
/// theme belongs to a `GdkDisplay` and a `status` verb registers its resources
/// long before GTK has one. Until it works the attempt is simply made again, so
/// the order the two happen in does not matter.
fn attach() {
    thread_local! {
        static ATTACHED: Cell<bool> = const { Cell::new(false) };
    }
    register();
    ATTACHED.with(|attached| {
        if attached.get() {
            return;
        }
        let Some(display) = gdk::Display::default() else {
            return;
        };
        // An *added* path, never a replaced one: the display's own theme keeps
        // its icons (an application's icon, a tray's, GTK's own) and ours are
        // found beside them, because an icon is looked up by name and our names
        // are all prefixed by the vocabulary below.
        gtk::IconTheme::for_display(&display).add_resource_path(RESOURCE);
        attached.set(true);
    });
}

/// One of the collection's own icons, `size` pixels square.
///
/// The `-symbolic` suffix is added here and not at the call sites: that suffix is
/// what makes GTK recolour the drawing with the widget's CSS `color` instead of
/// leaving it in its own colours, and a name missing it would still *draw* - so
/// the mistake would survive review. (The drawings themselves are outlined
/// rather than stroked, because GTK's SVG engine fills and does not stroke: see
/// `icons/README.md`. Nothing here depends on that; it is only why the files
/// look the way they do.)
///
/// The unit test below keeps the names honest the other way, by checking each one
/// against the bundle.
pub fn lucide(name: &str, size: i32) -> gtk::Image {
    let image = gtk::Image::new();
    image.add_css_class("icon");
    set_lucide(&image, name, size);
    image
}

/// Re-point an icon at another drawing, or at another size.
///
/// The cheap way for a view that repaints to change its icon - a speaker that
/// follows the level, a play button that becomes a pause - without rebuilding
/// the widget, which is what a card that is *open* while the value changes
/// cannot afford (the widget under the pointer would be replaced).
pub fn set_lucide(image: &gtk::Image, name: &str, size: i32) {
    attach();
    image.set_icon_name(Some(&format!("{name}-symbolic")));
    image.set_pixel_size(size);
}

/// The icons this collection draws, by name.
///
/// One vocabulary for all of the elements, because the same thing has to look
/// the same everywhere: the speaker is [`names::VOLUME_HIGH`] in the bar's status
/// pill, in the volume card and in the system popup, and those three used to
/// carry their own copy of the same Nerd Font codepoint each.
///
/// The names are the *icons* - `volume-2`, `chevron-left` - rather than the
/// meaning an element gives one, so a drawing that several elements share has one
/// name. What an element means by it belongs at the call site, where it can be
/// explained.
pub mod names {
    macro_rules! names {
        ($($constant:ident => $icon:literal,)*) => {
            $(
                pub const $constant: &str = $icon;
            )*

            /// Every name above, as `(constant, icon)`. Generated from the
            /// declarations themselves, so the test that holds this module
            /// against the bundle cannot fall out of step with it.
            pub const ALL: &[(&str, &str)] = &[$( (stringify!($constant), $icon) ),*];
        };
    }

    names! {
        // The workspaces. Lucide draws outlines; the solid dot is ours (see
        // `icons/local/`), and it is the one thing that tells the workspace you
        // are on from the ones you can go to.
        WORKSPACE_ACTIVE => "circle-filled",
        WORKSPACE_IDLE => "circle",

        // The bar's own furniture: the clock (and the island's handle), the
        // applications button, the session button.
        CLOCK => "clock",
        GRID => "layout-grid",
        POWER => "power",

        // The system pill and the system popup: what the machine is doing.
        CPU => "cpu",
        MEMORY => "memory-stick",
        TEMPERATURE => "thermometer",
        UPDATES => "download",
        CONTROLS => "sliders-horizontal",
        BLUETOOTH => "bluetooth",
        TERMINAL => "terminal",
        BRIGHTNESS => "sun",
        POWER_MODE => "zap",
        PRESENTATION => "presentation",

        // The wireless link, weak to strong, and the two ways it is not there.
        //
        // Lucide's *signal* family (the bar chart) is deliberately not used here
        // even though it is the obvious reading: its ink sits in the lower left
        // of the box, so at the 14px a bar pill draws it is a dot and a 2px line
        // in a corner - measured, not guessed. The wifi arcs are drawn centred
        // and fill the box, which is what a pill needs. `wifi-zero` is the
        // weakest quarter: it is the "there is a link and it is barely there"
        // step, next to `wifi-off` (no card, or nothing joined).
        WIFI_ZERO => "wifi-zero",
        WIFI_LOW => "wifi-low",
        WIFI_HIGH => "wifi-high",
        WIFI => "wifi",
        WIFI_OFF => "wifi-off",

        // The sound, at the three levels the bar's pill, the volume card and
        // the system popup all draw.
        VOLUME_OFF => "volume-x",
        VOLUME_LOW => "volume-1",
        VOLUME_HIGH => "volume-2",

        // The battery, empty to full, and charging - which is the one state
        // that is not a level.
        BATTERY_EMPTY => "battery",
        BATTERY_LOW => "battery-low",
        BATTERY_MEDIUM => "battery-medium",
        BATTERY_FULL => "battery-full",
        BATTERY_CHARGING => "battery-charging",

        // Playing something. Lucide carries no brand marks - that is a
        // deliberate choice of theirs - so a player is drawn as the *kind* of
        // thing it plays rather than as itself.
        MUSIC => "music",
        PLAY => "play",
        PAUSE => "pause",
        SKIP_BACK => "skip-back",
        SKIP_FORWARD => "skip-forward",
        CLAPPERBOARD => "clapperboard",
        AUDIO_LINES => "audio-lines",
        COMPASS => "compass",
        GLOBE => "globe",

        // The session menu: the five actions a session has.
        LOCK => "lock",
        MOON => "moon",
        LOG_OUT => "log-out",
        ROTATE_CW => "rotate-cw",

        // Notifications, and the small controls of the island's tiles.
        BELL => "bell",
        BELL_OFF => "bell-off",
        LIST => "list",
        TRASH => "trash-2",
        CHEVRON_LEFT => "chevron-left",
        CHEVRON_RIGHT => "chevron-right",
        EYE => "eye",
        EYE_OFF => "eye-off",

        // Choosing something: the launcher, the applications panel.
        SEARCH => "search",
        PIN => "pin",
        FOLDER => "folder",
        FILE => "file",

        // What a bluetooth device *is*, and what can be done to it. The one
        // exception is the fallback, which is the bluetooth mark itself.
        HEADPHONES => "headphones",
        SPEAKER => "speaker",
        KEYBOARD => "keyboard",
        MOUSE => "mouse",
        SMARTPHONE => "smartphone",
        MONITOR => "monitor",
        GAMEPAD => "gamepad-2",
        WATCH => "watch",
        CAMERA => "camera",
        PRINTER => "printer",
        DISPLAY => "tv",
        PLUS => "plus",
        LINK => "link",
        UNLINK => "unlink",
        CLOSE => "x",
    }
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

    // The three tests below hold the vocabulary against the bundle. They need no
    // display: a GResource is Gio's, not Gdk's, so they run wherever `cargo
    // test` runs - which is the point, because the failure they catch (`a name
    // with no drawing behind it`) would otherwise only appear when a card is
    // opened on screen.

    /// Every name in [`names`] has an icon in the bundle.
    ///
    /// Without this, a name the icons do not have draws GTK's "image missing"
    /// placeholder - an icon that looks like a bug report in the middle of a
    /// card, and only on the machine that happens to open that card.
    #[test]
    fn the_bundle_ships_every_icon_the_vocabulary_names() {
        register();
        for (constant, icon) in names::ALL {
            let path = format!("{RESOURCE}/scalable/actions/{icon}-symbolic.svg");
            assert!(
                gtk::gio::resources_lookup_data(&path, gtk::gio::ResourceLookupFlags::NONE).is_ok(),
                "{constant} asks for `{icon}`, which the bundle does not ship ({path})"
            );
        }
    }

    /// Two constants for one drawing means one of them is a second word for
    /// something that already has one - the vocabulary is meant to be the
    /// smallest set that covers what the elements draw.
    #[test]
    fn no_two_names_draw_the_same_icon() {
        let mut seen: HashMap<&str, &str> = HashMap::new();
        for (constant, icon) in names::ALL {
            if let Some(other) = seen.insert(icon, constant) {
                panic!("{constant} and {other} are both `{icon}`");
            }
        }
    }

    /// [`set_lucide`] adds the `-symbolic` suffix itself, so a name that already
    /// carries it would be looked up as `…-symbolic-symbolic` - a lookup that
    /// simply misses.
    #[test]
    fn the_symbolic_suffix_is_added_exactly_once() {
        for (constant, icon) in names::ALL {
            assert!(
                !icon.ends_with("-symbolic"),
                "{constant} is `{icon}`: the suffix is added by `lucide()`"
            );
        }
    }
}

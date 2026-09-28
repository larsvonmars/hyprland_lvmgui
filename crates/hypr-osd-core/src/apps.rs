//! The applications installed on this machine.
//!
//! Two elements are built on this: the launcher (the search card on `SUPER +
//! SPACE`) and the applications panel (the grid behind the bar's left-end
//! button). Both need the same three things, so they live here rather than in
//! one of them:
//!
//! * **Parsing.** The freedesktop `[Desktop Entry]` subset a launcher needs:
//!   which entries should be *offered* (launchable, not `NoDisplay`/`Hidden`,
//!   not restricted to another desktop environment) and what a person can search
//!   them by - the localized name, the generic name, the comment, the keywords
//!   and the categories.
//! * **Grouping.** A `Categories` token is a programming-language word
//!   (`WebBrowser`, `AudioVideo`); a person looking for something asks for a
//!   *drawer*. [`category`] turns the first into the second, with a keyword
//!   fallback for entries that never filled the field in.
//! * **Starting one.** [`launch`] hands the file to `gio launch`, which knows
//!   about `Exec` field codes, `Terminal=true` and `TryExec` - better than
//!   re-implementing the desktop entry spec would.
//!
//! It is deliberately a parser rather than `gio::DesktopAppInfo`: the keywords
//! and categories GIO does not hand out are exactly what makes a launcher
//! usable. Starting an entry is a different question, and there the environment
//! does a better job than we would.
//!
//! (An older project of the same author, `linux-launchpad`, had a much larger
//! version of this file with a Tauri front end on top. The parsing, the filters,
//! the maps and the scoring are the same; the app catalogue and the settings page
//! around them are not here.)

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::hardware;

/// One application the user can be offered.
#[derive(Debug, Clone, Default)]
pub struct App {
    /// Desktop file id: the file name without `.desktop`. It is the entry's
    /// identity - what makes two directories' copies the same application, and
    /// what a favourites file stores.
    pub id: String,
    /// Display name (localized when the entry has a translation for the locale).
    pub name: String,
    pub generic: Option<String>,
    pub comment: Option<String>,
    /// `Keywords` tokens - the second half of a good search, after the name.
    pub keywords: Vec<String>,
    /// `Categories` tokens (freedesktop names, e.g. `WebBrowser`).
    pub categories: Vec<String>,
    /// The drawer this entry belongs in (see [`category`]).
    pub category: String,
    /// The raw `Icon=` value: an icon *name*, or an absolute path.
    pub icon: Option<String>,
    /// The file itself - what `gio launch` is handed.
    pub desktop_path: PathBuf,
    /// The raw `Exec=` line. Only used when `gio` is not installed.
    pub exec: String,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// A very small INI-ish parser for `.desktop` files, keyed `"GROUP.KEY"`.
///
/// The group matters: an entry has one `[Desktop Entry]` group in practice, and
/// keying by it is what keeps a `[Desktop Action new-window]`'s `Exec` from
/// overwriting the one that belongs to the entry.
fn parse_desktop(content: &str) -> HashMap<String, String> {
    let mut values = HashMap::new();
    let mut group = String::new();

    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            group = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim();
            let value = line[eq + 1..].trim();
            values.insert(format!("{group}.{key}"), value.to_string());
        }
    }
    values
}

/// Split a `;`-separated list, dropping empty tokens.
fn split_list(value: Option<&String>) -> Vec<String> {
    value
        .map(|v| {
            v.split(';')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Two-letter language tag from the message locale (`de_DE.UTF-8` -> `de`).
///
/// `$LC_MESSAGES`/`$LC_ALL` win over `$LANG`, which is what a shell does too - an
/// entry that ships `Name[de]` should answer with it when the user asked for
/// German by either route.
fn locale_tag() -> Option<String> {
    let raw = std::env::var("LC_ALL")
        .ok()
        .or_else(|| std::env::var("LC_MESSAGES").ok())
        .or_else(|| std::env::var("LANG").ok())?;
    let tag = raw.split('.').next()?.split('_').next()?.to_lowercase();
    if tag.is_empty() {
        None
    } else {
        Some(tag)
    }
}

/// A localized value (`Name[de]`), falling back to the plain key.
fn localized(values: &HashMap<String, String>, key: &str) -> Option<String> {
    if let Some(tag) = locale_tag() {
        if let Some(value) = values.get(&format!("{key}[{tag}]")) {
            return Some(value.clone());
        }
    }
    values.get(key).cloned()
}

/// The current desktop's name (`$XDG_CURRENT_DESKTOP`), e.g. `Hyprland`.
fn current_desktop() -> String {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()
}

/// Whether the entry should be offered in this desktop environment.
fn passes_desktop_filter(values: &HashMap<String, String>, desktop: &str) -> bool {
    let only_show_in = split_list(values.get("Desktop Entry.OnlyShowIn"));
    let not_show_in = split_list(values.get("Desktop Entry.NotShowIn"));

    // An unknown desktop means not over-filtering: an entry that says "only in
    // GNOME" is still a program the user may want to launch here.
    if desktop.is_empty() {
        return true;
    }
    let in_list = |list: &[String]| list.iter().any(|d| d.eq_ignore_ascii_case(desktop));
    if !only_show_in.is_empty() && !in_list(&only_show_in) {
        return false;
    }
    if in_list(&not_show_in) {
        return false;
    }
    true
}

/// Directories that may hold `.desktop` files, highest priority first: the user's
/// own entries override the system's for the same id.
fn app_directories() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut push = |path: PathBuf| {
        if !dirs.contains(&path) {
            dirs.push(path);
        }
    };

    match std::env::var_os("XDG_DATA_HOME") {
        Some(path) => {
            let base = PathBuf::from(path);
            push(base.join("applications"));
            push(base.join("flatpak/exports/share/applications"));
        }
        None => {
            if let Some(home) = std::env::var_os("HOME") {
                let base = PathBuf::from(home);
                push(base.join(".local/share/applications"));
                push(base.join(".local/share/flatpak/exports/share/applications"));
            }
        }
    }

    if let Some(dirs_var) = std::env::var_os("XDG_DATA_DIRS") {
        for dir in std::env::split_paths(&dirs_var) {
            push(dir.join("applications"));
        }
    }

    push(PathBuf::from("/usr/local/share/applications"));
    push(PathBuf::from("/usr/share/applications"));
    push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));

    dirs
}

/// An `Icon=` value normalized for the icon theme: a bare `app.png` is looked up
/// as `app`, because a theme indexes names, not file names.
fn icon_value(raw: &str) -> Option<String> {
    let raw = raw.trim().strip_prefix("file://").unwrap_or(raw.trim());
    if raw.is_empty() {
        return None;
    }
    if raw.starts_with('/') {
        return Some(raw.to_string());
    }
    const EXTENSIONS: &[&str] = &["png", "svg", "xpm"];
    for extension in EXTENSIONS {
        if let Some(stem) = raw
            .strip_suffix(&format!(".{extension}"))
            .filter(|stem| !stem.is_empty())
        {
            return Some(stem.to_string());
        }
    }
    Some(raw.to_string())
}

/// Scan every known application directory for the entries worth offering.
///
/// Reading a few hundred small files is a handful of milliseconds, so this runs
/// once at start-up and is only repeated by an element's `refresh` verb - never
/// on every keystroke, and never while a card is waiting to be drawn.
pub fn scan() -> Vec<App> {
    let desktop = current_desktop();
    let mut apps: Vec<App> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for dir in app_directories() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Some(id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            if id.is_empty() || seen.contains(&id) {
                continue; // the user's own copy wins
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let values = parse_desktop(&content);

            // Only launchable applications...
            if values.get("Desktop Entry.Type").map(String::as_str) != Some("Application") {
                continue;
            }
            // ...that are not hidden, and that this desktop should offer.
            let flag = |key: &str| {
                values
                    .get(key)
                    .is_some_and(|v| v.eq_ignore_ascii_case("true"))
            };
            if flag("Desktop Entry.NoDisplay") || flag("Desktop Entry.Hidden") {
                continue;
            }
            if !passes_desktop_filter(&values, &desktop) {
                continue;
            }

            let name = localized(&values, "Desktop Entry.Name")
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| id.clone());
            let generic =
                localized(&values, "Desktop Entry.GenericName").filter(|value| !value.is_empty());
            let categories = split_list(values.get("Desktop Entry.Categories"));
            let keywords = split_list(values.get("Desktop Entry.Keywords"));

            apps.push(App {
                category: category(&categories, &name, generic.as_deref(), &keywords),
                id: id.clone(),
                name,
                generic,
                comment: localized(&values, "Desktop Entry.Comment")
                    .filter(|value| !value.is_empty()),
                keywords,
                categories,
                icon: values
                    .get("Desktop Entry.Icon")
                    .and_then(|value| icon_value(value)),
                desktop_path: path,
                exec: values
                    .get("Desktop Entry.Exec")
                    .cloned()
                    .unwrap_or_default(),
            });
            seen.insert(id);
        }
    }

    apps.sort_by_key(|app| app.name.to_lowercase());
    apps
}

// ---------------------------------------------------------------------------
// Grouping: which drawer an application belongs in
// ---------------------------------------------------------------------------

/// Map freedesktop.org category tokens to a friendly display group.
///
/// The groups are the old launcher's, kept in their order: the first map entry
/// that claims *any* of an application's tokens wins, so the order is a
/// tie-break between an entry that lists several (`AudioVideo;Player;`).
const CATEGORY_MAP: &[(&str, &[&str])] = &[
    (
        "Multimedia",
        &[
            "AudioVideo",
            "Audio",
            "Video",
            "Music",
            "Player",
            "Recorder",
            "Midi",
            "Sequencer",
            "Mixer",
            "Tuner",
        ],
    ),
    (
        "Development",
        &[
            "Development",
            "IDE",
            "Debugger",
            "Programming",
            "Building",
            "RevisionControl",
            "TextEditor",
        ],
    ),
    (
        "Education & Science",
        &[
            "Education",
            "Science",
            "Math",
            "Physics",
            "Chemistry",
            "Astronomy",
            "Biology",
            "Geography",
            "Geology",
            "History",
            "Languages",
            "Literature",
            "Electronics",
            "Engineering",
            "ComputerScience",
            "Medical",
            "Sports",
        ],
    ),
    (
        "Games",
        &[
            "Game",
            "ActionGame",
            "AdventureGame",
            "ArcadeGame",
            "BoardGame",
            "BlocksGame",
            "CardGame",
            "KidsGame",
            "LogicGame",
            "RolePlaying",
            "Shooter",
            "Simulation",
            "SportsGame",
            "StrategyGame",
        ],
    ),
    (
        "Graphics",
        &[
            "Graphics",
            "2DGraphics",
            "3DGraphics",
            "VectorGraphics",
            "RasterGraphics",
            "Scanning",
            "Photography",
            "OCR",
            "Publishing",
            "Viewer",
        ],
    ),
    (
        "Internet & Networking",
        &[
            "Network",
            "WebBrowser",
            "InstantMessaging",
            "Chat",
            "IRCClient",
            "Feed",
            "Email",
            "News",
            "Telephony",
            "RemoteAccess",
            "FileTransfer",
            "P2P",
            "VideoConference",
            "HamRadio",
            "Monitor",
        ],
    ),
    (
        "Office",
        &[
            "Office",
            "WordProcessor",
            "Spreadsheet",
            "Presentation",
            "PIM",
            "Calendar",
            "ContactManagement",
            "Database",
            "Dictionary",
            "Finance",
            "Accounting",
            "ProjectManagement",
        ],
    ),
    (
        "System & Settings",
        &[
            "Settings",
            "DesktopSettings",
            "HardwareSettings",
            "Printing",
            "PackageManager",
            "Security",
            "System",
        ],
    ),
    (
        "Utilities",
        &[
            "Utility",
            "Archiving",
            "Compression",
            "FileManager",
            "FileTools",
            "TerminalEmulator",
            "Accessibility",
            "Core",
            "Emulator",
        ],
    ),
];

/// Keyword fallback used when the `Categories` field is missing or unknown.
///
/// Half the entries on a typical machine leave `Categories` empty, so without
/// this the "Other" drawer is the biggest one - which is a drawer nobody opens.
const KEYWORD_MAP: &[(&str, &[&str])] = &[
    (
        "Multimedia",
        &[
            "music", "audio", "video", "player", "record", "sound", "media", "song", "album",
            "spotify", "vlc", "youtube", "podcast",
        ],
    ),
    (
        "Development",
        &[
            "code",
            "editor",
            "ide",
            "program",
            "build",
            "git",
            "terminal",
            "console",
            "vim",
            "emacs",
            "jetbrains",
            "sublime",
            "python",
            "node",
            "compiler",
            "debugger",
            "developer",
            "studio",
        ],
    ),
    (
        "Internet & Networking",
        &[
            "browser",
            "web",
            "internet",
            "mail",
            "email",
            "chat",
            "messenger",
            "network",
            "ssh",
            "download",
            "feed",
            "rss",
            "firefox",
            "chrome",
            "discord",
            "signal",
            "telegram",
            "slack",
            "matrix",
        ],
    ),
    (
        "Games",
        &["game", "steam", "lutris", "heroes", "minecraft", "play"],
    ),
    (
        "Graphics",
        &[
            "image",
            "photo",
            "gimp",
            "inkscape",
            "draw",
            "paint",
            "vector",
            "scan",
            "camera",
            "design",
            "illustrator",
            "viewer",
            "model",
            "render",
            "3d",
            "blender",
            "cad",
        ],
    ),
    (
        "Office",
        &[
            "office",
            "word",
            "spreadsheet",
            "sheet",
            "present",
            "slide",
            "document",
            "pdf",
            "libreoffice",
            "calc",
            "writer",
            "impress",
            "notes",
            "calendar",
            "mail",
        ],
    ),
    (
        "Education & Science",
        &[
            "learn",
            "educat",
            "science",
            "math",
            "school",
            "study",
            "dictionary",
            "wiki",
            "language",
        ],
    ),
    (
        "System & Settings",
        &[
            "settings",
            "system",
            "control",
            "preferences",
            "config",
            "monitor",
            "display",
            "printer",
            "driver",
            "about",
            "info",
            "tweak",
        ],
    ),
    (
        "Utilities",
        &[
            "file",
            "archive",
            "zip",
            "extract",
            "manager",
            "disk",
            "backup",
            "utility",
            "clipboard",
            "screenshot",
            "search",
            "calculator",
            "clock",
            "todo",
            "text",
        ],
    ),
];

/// The drawer to show an application in.
///
/// Three answers, in order of trust: the freedesktop `Categories` tokens when the
/// entry filled them in, then the words in its name, generic name and keywords,
/// and finally `Other` - which is a real drawer (it is where an entry nobody can
/// classify ends up), not an error.
pub fn category(
    categories: &[String],
    name: &str,
    generic: Option<&str>,
    keywords: &[String],
) -> String {
    for (display, tokens) in CATEGORY_MAP {
        if categories.iter().any(|c| tokens.contains(&c.as_str())) {
            return (*display).to_string();
        }
    }

    let haystack = format!(
        "{name} {} {}",
        generic.unwrap_or_default(),
        keywords.join(" ")
    )
    .to_lowercase();
    for (display, tokens) in KEYWORD_MAP {
        if tokens.iter().any(|token| haystack.contains(token)) {
            return (*display).to_string();
        }
    }

    "Other".to_string()
}

/// Every drawer, in the order the panel's sidebar shows them: `Other` last,
/// because it is where the things nobody could classify are.
pub const OTHER: &str = "Other";

/// The drawers that exist for `apps`, alphabetically with `Other` moved to the
/// end - the order a sidebar wants, so the leftovers are not the first thing a
/// person sees.
pub fn categories_in(apps: &[App]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for app in apps {
        if !names.contains(&app.category) {
            names.push(app.category.clone());
        }
    }
    names.sort_by_key(|name| name.to_lowercase());
    if let Some(position) = names.iter().position(|name| name == OTHER) {
        let other = names.remove(position);
        names.push(other);
    }
    names
}

// ---------------------------------------------------------------------------
// Searching
// ---------------------------------------------------------------------------

/// Whether the characters of `query` appear in `text`, in order.
///
/// The `fzf`-style subsequence test: `ffx` finds `Firefox`. It is what makes a
/// half-remembered word work, and it is cheap enough to run over a few hundred
/// names per keystroke.
pub fn fuzzy(text: &str, query: &str) -> bool {
    let mut query = query.chars();
    let mut wanted = query.next();
    for character in text.chars() {
        if Some(character) == wanted {
            wanted = query.next();
            if wanted.is_none() {
                return true;
            }
        }
    }
    wanted.is_none()
}

/// The score for one application, or `0` when it does not match at all.
///
/// The ladder, in order: an exact name beats a name prefix, which beats a fuzzy
/// subsequence of the name, which beats the generic name, which beats a literal
/// substring of the metadata. Two guards keep it honest:
///
/// * A subsequence only counts when the name is not far longer than what was
///   typed. `ffx` finding `Firefox` is a shortcut; `icf` "finding" `Avahi
///   Zeroconf Browser` (i, c and f forty characters apart) is noise that fills a
///   list with applications nobody asked for.
/// * Nothing matches a *subsequence* of the metadata. A keyword or a category is
///   a match (`internet` -> Firefox); three letters scattered through a comment
///   are not.
pub fn score(app: &App, query: &str) -> u32 {
    let name = app.name.to_lowercase();
    if name == query {
        return 1000;
    }
    let length = name.chars().count() as u32;
    if name.starts_with(query) {
        return 900u32.saturating_sub(length);
    }
    if fuzzy(&name, query) && length <= 3 * query.chars().count() as u32 {
        // The further the name is from what was typed, the less certain it is.
        let distance = length.abs_diff(query.chars().count() as u32);
        return 600u32.saturating_sub(distance);
    }

    let generic = app.generic.as_deref().unwrap_or_default().to_lowercase();
    if !generic.is_empty() && generic.starts_with(query) {
        return 450;
    }

    if haystack(app).contains(query) {
        return 300;
    }
    0
}

/// Whether an application matches `query` at all - the filter the applications
/// panel uses, where the list keeps its alphabetical order and only narrows.
pub fn matches(app: &App, query: &str) -> bool {
    query.trim().is_empty() || score(app, query.trim()) > 0
}

/// The searchable metadata around the name: the generic name, the keywords, the
/// categories and the comment, joined for a substring test.
fn haystack(app: &App) -> String {
    let mut parts: Vec<String> = vec![app.name.to_lowercase()];
    parts.extend(app.generic.iter().map(|value| value.to_lowercase()));
    parts.extend(app.keywords.iter().map(|value| value.to_lowercase()));
    parts.extend(app.categories.iter().map(|value| value.to_lowercase()));
    parts.extend(app.comment.iter().map(|value| value.to_lowercase()));
    parts.join(" ")
}

// ---------------------------------------------------------------------------
// Launching
// ---------------------------------------------------------------------------

/// Replace `%` field codes in an `Exec=` line so it can be run through a shell.
///
/// Only the fallback path needs this - `gio launch` understands the codes
/// itself - but an `Exec` written for a file argument (`firefox %u`) must not
/// hand a literal `%u` to the program when the fallback runs.
fn expand_exec(exec: &str) -> String {
    const DROPPED: &[char] = &[
        'f', 'F', 'u', 'U', 'd', 'D', 'n', 'N', 'v', 'm', 'c', 'i', 'k',
    ];
    let mut out = String::new();
    let mut chars = exec.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '%' {
            out.push(character);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some(code) if DROPPED.contains(&code) => {}
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => {}
        }
    }
    out.trim().to_string()
}

/// Start an application and walk away.
///
/// `gio launch` is the right tool when it is there: it is the same code the
/// desktop uses, so `Exec` field codes, `Terminal=true` and `TryExec` behave
/// exactly as they do in the menu. The fallback runs the `Exec` line through a
/// shell for the one machine that has no `gio` - which is to say, none (`gio`
/// ships with glib, which GTK4 links anyway) - so it does not try to reproduce
/// `Terminal=true`; an element that silently does nothing would be worse than one
/// that starts the program without its terminal.
///
/// Never a `status()`/`wait()`: an application outlives the click by hours, and a
/// launcher that waited for it would freeze the shell that ran the verb.
pub fn launch(app: &App) {
    if hardware::installed("gio") {
        hardware::launch("gio", &["launch", &app.desktop_path.to_string_lossy()]);
        return;
    }
    let command = expand_exec(&app.exec);
    if command.is_empty() {
        eprintln!(
            "hypr-osd: {} has no Exec line and there is no gio to ask",
            app.name
        );
        return;
    }
    hardware::launch("sh", &["-c", &command]);
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIREFOX: &str = "\
[Desktop Entry]
Type=Application
Name=Firefox
Name[de]=Feuerfuchs
GenericName=Web Browser
Comment=Browse the Web
Exec=firefox %u
Icon=firefox
Categories=Network;WebBrowser;
Keywords=Internet;Browser;
Terminal=false
NoDisplay=false
";

    fn app(name: &str, generic: Option<&str>, keywords: &[&str]) -> App {
        App {
            id: name.to_lowercase(),
            name: name.to_string(),
            generic: generic.map(str::to_string),
            keywords: keywords.iter().map(|value| value.to_string()).collect(),
            ..App::default()
        }
    }

    #[test]
    fn reads_the_group_it_was_asked_for() {
        let values = parse_desktop(FIREFOX);
        assert_eq!(
            values.get("Desktop Entry.Name").map(String::as_str),
            Some("Firefox")
        );
        // A later group does not overwrite the entry's own keys.
        let values = parse_desktop(&format!(
            "{FIREFOX}\n[Desktop Action new-window]\nExec=firefox --new-window\n"
        ));
        assert_eq!(
            values.get("Desktop Entry.Exec").map(String::as_str),
            Some("firefox %u")
        );
        assert_eq!(
            values
                .get("Desktop Action new-window.Exec")
                .map(String::as_str),
            Some("firefox --new-window")
        );
    }

    #[test]
    fn lists_are_split_and_trimmed() {
        let values = parse_desktop(FIREFOX);
        assert_eq!(
            split_list(values.get("Desktop Entry.Categories")),
            vec!["Network", "WebBrowser"]
        );
        assert!(split_list(values.get("Desktop Entry.Missing")).is_empty());
        assert!(split_list(Some(&String::new())).is_empty());
    }

    #[test]
    fn a_localized_name_is_preferred_when_the_locale_asks_for_it() {
        let values = parse_desktop(FIREFOX);
        let name = if locale_tag().as_deref() == Some("de") {
            "Feuerfuchs"
        } else {
            "Firefox"
        };
        assert_eq!(
            localized(&values, "Desktop Entry.Name").as_deref(),
            Some(name)
        );
    }

    #[test]
    fn only_show_in_and_not_show_in_are_honoured() {
        let mut values = HashMap::new();
        values.insert(
            "Desktop Entry.OnlyShowIn".to_string(),
            "GNOME;KDE;".to_string(),
        );
        assert!(passes_desktop_filter(&values, "GNOME"));
        assert!(!passes_desktop_filter(&values, "Hyprland"));
        // An unknown desktop never filters anything out.
        assert!(passes_desktop_filter(&values, ""));

        let mut values = HashMap::new();
        values.insert(
            "Desktop Entry.NotShowIn".to_string(),
            "Hyprland;".to_string(),
        );
        assert!(!passes_desktop_filter(&values, "Hyprland"));
        assert!(passes_desktop_filter(&values, "GNOME"));
    }

    #[test]
    fn an_icon_file_name_becomes_the_name_a_theme_indexes() {
        assert_eq!(icon_value("firefox").as_deref(), Some("firefox"));
        assert_eq!(icon_value("app.png").as_deref(), Some("app"));
        assert_eq!(
            icon_value("file:///opt/x/icon.svg").as_deref(),
            Some("/opt/x/icon.svg")
        );
        assert_eq!(icon_value("   "), None);
    }

    #[test]
    fn field_codes_are_dropped_from_the_fallback_exec() {
        assert_eq!(expand_exec("firefox %u"), "firefox");
        assert_eq!(expand_exec("foo %F --bar %i"), "foo  --bar");
        assert_eq!(expand_exec("printf 100%%"), "printf 100%");
        // A code that is not one we know stays as it was written.
        assert_eq!(expand_exec("weird %z"), "weird %z");
    }

    #[test]
    fn a_category_token_beats_a_keyword() {
        assert_eq!(
            category(&["WebBrowser".into()], "Firefox", None, &[]),
            "Internet & Networking"
        );
        // Several tokens: the map's own order decides, not the entry's.
        assert_eq!(
            category(&["Player".into(), "AudioVideo".into()], "VLC", None, &[]),
            "Multimedia"
        );
    }

    #[test]
    fn keyword_fallback_covers_entries_with_no_categories() {
        assert_eq!(category(&[], "Steam", None, &[]), "Games");
        assert_eq!(category(&[], "GIMP", Some("Photo"), &[]), "Graphics");
        // The first map entry that claims the haystack wins, and `editor` is a
        // Development keyword - so anything *described* as an editor lands
        // there, including an image editor. It is the trade the old launcher
        // made too, and it is the right way round: the entries this fallback
        // runs for are the ones with no `Categories` at all, and "Text Editor"
        // is a far more common generic name than "Image Editor" (GIMP and
        // Inkscape carry a Graphics token, so they never reach this path).
        assert_eq!(
            category(&[], "Kate", Some("Text Editor"), &[]),
            "Development"
        );
        // Nothing to go on is still a drawer - the honest one.
        assert_eq!(category(&[], "Foo", None, &[]), OTHER);
    }

    #[test]
    fn other_sorts_last_among_the_drawers() {
        let mut fallback = app("Foo", None, &[]);
        fallback.category = OTHER.to_string();
        let mut browser = app("Firefox", None, &[]);
        browser.category = "Internet & Networking".to_string();
        let mut player = app("VLC", None, &[]);
        player.category = "Multimedia".to_string();
        assert_eq!(
            categories_in(&[fallback, browser, player]),
            vec!["Internet & Networking", "Multimedia", OTHER]
        );
    }

    #[test]
    fn fuzzy_is_a_subsequence_test() {
        assert!(fuzzy("firefox", "ffx"));
        assert!(fuzzy("firefox", "firefox"));
        assert!(!fuzzy("firefox", "xf"));
        assert!(!fuzzy("firefox", "firefoxx"));
    }

    #[test]
    fn the_ladder_is_exact_prefix_fuzzy_then_the_metadata() {
        let firefox = app("Firefox", Some("Web Browser"), &["Internet", "Browser"]);
        assert_eq!(score(&firefox, "firefox"), 1000);
        assert!(score(&firefox, "fire") > 800);
        assert!(score(&firefox, "ffx") > 500 && score(&firefox, "ffx") < 600);

        let mut editor = app("Kate", Some("Text Editor"), &[]);
        editor.comment = Some("Advanced text editor".into());
        assert_eq!(score(&editor, "text editor"), 450);
        assert_eq!(score(&editor, "advanced"), 300);
        // A keyword still finds it...
        assert_eq!(score(&firefox, "internet"), 300);
        // ...but letters scattered through a comment do not.
        assert_eq!(score(&editor, "adte"), 0);
        // Neither do letters scattered through a long *name*.
        let avahi = app("Avahi Zeroconf Browser", None, &[]);
        assert_eq!(score(&avahi, "icf"), 0);
        assert_eq!(score(&firefox, "zzz"), 0);

        // The filter the panel uses says yes to everything the ladder scores.
        assert!(matches(&firefox, ""));
        assert!(matches(&firefox, "  "));
        assert!(matches(&firefox, "ffx"));
        assert!(!matches(&firefox, "zzz"));
    }

    #[test]
    fn a_shorter_name_wins_a_prefix_tie() {
        let short = app("Firefox", None, &[]);
        let long = app("Firefox Developer Edition", None, &[]);
        assert!(score(&short, "firefox") > score(&long, "firefox"));
        assert!(score(&short, "fire") > score(&long, "fire"));
    }
}

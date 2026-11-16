//! The login screen: SDDM's hypr-osd theme, generated from the settings window's
//! **Login screen** page.
//!
//! The login screen is the one face of this desktop that is not drawn by
//! Hyprland: SDDM's greeter runs before any compositor, in Qt, from a theme
//! directory. Its source lives in `~/.config/hypr/sddm-theme/` (Main.qml,
//! metadata.desktop, `theme.conf` and the rendered `images/background.jpg`), and
//! the greeter reads `theme.conf` for everything it can vary.
//!
//! This module writes the two halves that page edits:
//!
//! * **`theme.conf`**, from `~/.config/hypr-osd/login.conf`: the picture, the
//!   face, and the eight palette colours the QML reads (which mirror
//!   `~/.config/hypr-osd/theme.css`, because a Qt greeter cannot read CSS - the
//!   same reason the lock screen's palette is generated, see [`crate::lock`]).
//! * **`images/background.jpg`**, rendered from the wallpaper with `magick` - the
//!   greeter has no compositor behind it, so the blur and the dimming happen once
//!   here rather than on every login. A picture of your own can be named instead,
//!   or the rendering switched off so the greeter draws the file directly.
//!
//! What this module cannot do is install the theme: `/usr/share/sddm/themes/` and
//! `/etc/sddm.conf.d/` belong to root. `publish` therefore copies the files into
//! the installed theme when the directory happens to be writable and otherwise
//! says so; `sudo ~/.config/hypr/sddm-theme/install.sh` is the step that makes a
//! change visible at the next logout.

use std::path::{Path, PathBuf};

use crate::config::{self, Config};

/// The settings file the Login screen page edits: `~/.config/hypr-osd/login.conf`.
pub const FILE: &str = "login";

/// The theme's source, as a directory under `$XDG_CONFIG_HOME/hypr`.
pub const THEME_DIR: &str = "sddm-theme";
/// The installed copy the greeter actually reads.
pub const INSTALLED: &str = "/usr/share/sddm/themes/hypr-osd";
/// SDDM's per-theme settings, including `CursorSize`.
pub const DROPIN: &str = "/etc/sddm.conf.d/hypr-osd.conf";

// --- the defaults ----------------------------------------------------------
//
// The picture follows hyprpaper, the rendering is on, and the numbers are the
// ones `sddm-theme/install.sh` renders with. The colours are theme.css's, so a
// machine that never opens the page gets the palette the desktop already has.

pub const DEFAULT_BACKGROUND: &str = "";
pub const DEFAULT_RENDER: bool = true;
pub const DEFAULT_BLUR: &str = "16";
pub const DEFAULT_DIM: &str = "62";
pub const DEFAULT_SATURATION: &str = "112";
pub const DEFAULT_RENDER_WIDTH: &str = "2560";
pub const DEFAULT_RENDER_HEIGHT: &str = "1440";
pub const DEFAULT_FONT: &str = "Cantarell";
pub const DEFAULT_BG: &str = "#1a1412";
pub const DEFAULT_FG: &str = "#efe3d8";
pub const DEFAULT_FG_DIM: &str = "#b4a49a";
pub const DEFAULT_FG_FAINT: &str = "#6f6059";
pub const DEFAULT_ACCENT: &str = "#f2b49b";
pub const DEFAULT_ON_ACCENT: &str = "#2b1a14";
pub const DEFAULT_WARN: &str = "#f0d59c";
pub const DEFAULT_CRIT: &str = "#ee7e84";
pub const DEFAULT_CURSOR_SIZE: &str = "24";

/// `~/.config/hypr/sddm-theme` - the theme's source.
pub fn theme_dir() -> PathBuf {
    config::config_home().join("hypr").join(THEME_DIR)
}

/// The theme.conf the greeter reads: the source one, which `publish` copies.
pub fn conf_path() -> PathBuf {
    theme_dir().join("theme.conf")
}

/// The rendered background the greeter draws when `render` is on.
pub fn image_path() -> PathBuf {
    theme_dir().join("images").join("background.jpg")
}

/// The theme's own installer: the step that copies the generated files into
/// `/usr/share/sddm/themes/hypr-osd` and writes `/etc/sddm.conf.d/hypr-osd.conf`.
///
/// It lives beside the theme because the destination belongs to root, which is
/// also why the page cannot run it itself: the settings window hands it to a
/// terminal, where `sudo` can ask for the password and the installer's own
/// output can be read.
pub fn install_script() -> PathBuf {
    theme_dir().join("install.sh")
}

/// The picture the rendering starts from: the page's own `background`, else the
/// wallpaper hyprpaper draws - the one place the two can only disagree if the page
/// says so.
pub fn source(cfg: &Config) -> Option<String> {
    let chosen = text(cfg, "background", DEFAULT_BACKGROUND);
    if !chosen.is_empty() {
        return Some(chosen);
    }
    crate::lock::wallpaper()
}

/// Render the background, if the page asks for it and there is a picture to start
/// from. Returns the program and arguments to hand to `hardware::launch`, or
/// `None` - the caller decides whether waiting is acceptable (it is not: a 4K
/// blur takes a moment, and a settings window must not park on it).
pub fn render_command(cfg: &Config) -> Option<(String, Vec<String>)> {
    render_command_from(cfg, source(cfg).as_deref())
}

/// The same, from a picture handed in - which is what makes it testable without
/// this machine's own wallpaper.
pub fn render_command_from(cfg: &Config, source: Option<&str>) -> Option<(String, Vec<String>)> {
    if !cfg.bool("render", DEFAULT_RENDER) {
        return None;
    }
    let source = source?;
    let width = text(cfg, "render_width", DEFAULT_RENDER_WIDTH);
    let height = text(cfg, "render_height", DEFAULT_RENDER_HEIGHT);
    let blur = text(cfg, "blur", DEFAULT_BLUR);
    let dim = text(cfg, "dim", DEFAULT_DIM);
    let saturation = text(cfg, "saturation", DEFAULT_SATURATION);
    let image = image_path();
    Some((
        "magick".to_string(),
        vec![
            source.to_string(),
            "-resize".to_string(),
            format!("{width}x{height}^"),
            "-gravity".to_string(),
            "center".to_string(),
            "-extent".to_string(),
            format!("{width}x{height}"),
            "-blur".to_string(),
            format!("0x{blur}"),
            "-modulate".to_string(),
            format!("{dim},{saturation}"),
            "-quality".to_string(),
            "88".to_string(),
            image.to_string_lossy().into_owned(),
        ],
    ))
}

/// Write `theme.conf` (and, if the installed theme is writable, copy the files
/// into it). Returns the source file written and a line about the installed copy,
/// or why the write failed.
pub fn write() -> Result<(PathBuf, String), String> {
    let cfg = Config::load(FILE);
    let path = conf_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    std::fs::write(
        &path,
        document_from(&cfg, source(&cfg).as_deref(), &crate::css::tokens()),
    )
    .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    // The image belongs beside it; `magick` cannot create the directory.
    if let Some(parent) = image_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    Ok((path, publish(&cfg)))
}

/// Copy the theme's files into the installed theme, when that directory is
/// writable - which it is not on a normal installation, because it belongs to
/// root. The answer is a sentence for the settings page, never an error: the
/// source file is the thing this module owns.
fn publish(cfg: &Config) -> String {
    let installed = Path::new(INSTALLED);
    let image = image_path();
    let mut copied = Vec::new();
    for name in ["Main.qml", "metadata.desktop", "theme.conf"] {
        let from = theme_dir().join(name);
        let to = installed.join(name);
        if from.is_file() && std::fs::copy(&from, &to).is_ok() {
            copied.push(name);
        }
    }
    if image.is_file() && std::fs::copy(&image, installed.join("images/background.jpg")).is_ok() {
        copied.push("images/background.jpg");
    }
    if copied.is_empty() {
        "written; press Install to copy it into /usr/share/sddm (needs root)".to_string()
    } else {
        // The drop-in carries the cursor size, which is not a theme.conf key.
        let cursor = text(cfg, "cursor_size", DEFAULT_CURSOR_SIZE);
        let _ = std::fs::write(
            DROPIN,
            format!(
                "# The hypr-osd login screen, written by hypr-osd-settings.\n\
                 [Theme]\nCurrent=hypr-osd\nCursorSize={cursor}\n"
            ),
        );
        format!(
            "written and copied into {INSTALLED} ({})",
            copied.join(", ")
        )
    }
}

/// The theme.conf keys that are colours. When `login.conf` does not set one, the
/// value comes from `~/.config/hypr-osd/theme.css` - the palette every element
/// draws from, and the one the lock screen is generated from - rather than from
/// this module's copy of it.
pub const PALETTE: &[&str] = &[
    "bg",
    "fg",
    "fg_dim",
    "fg_faint",
    "accent",
    "on_accent",
    "warn",
    "crit",
];

/// The file, from the machine it is on.
pub fn document() -> String {
    let cfg = Config::load(FILE);
    document_from(&cfg, source(&cfg).as_deref(), &crate::css::tokens())
}

/// The theme.conf, from a config, the picture the page points at and the palette
/// in force.
///
/// `background` in theme.conf has to be relative to the theme directory (as
/// `images/background.jpg` is) or absolute. With `render` on it is always the
/// rendered image; with it off it is the picture itself, which the greeter reads
/// as the `sddm` user - an absolute path into a home directory only works when
/// that directory is world-readable, which is why rendering is the default.
pub fn document_from(cfg: &Config, source: Option<&str>, palette: &[(String, String)]) -> String {
    let render = cfg.bool("render", DEFAULT_RENDER);
    let background = if render {
        "images/background.jpg".to_string()
    } else {
        source
            .map(str::to_string)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "images/background.jpg".to_string())
    };

    let mut out = String::from(
        "; GENERATED FILE - do not edit.\n\
         ;\n\
         ; hypr-osd's login screen settings, written by the settings window's Login screen\n\
         ; page from ~/.config/hypr-osd/login.conf. Main.qml reads every key here and falls\n\
         ; back to its own copy of the value when one is absent.\n\
         ;\n\
         ; SDDM reads this file with QSettings, so `;` starts a comment.\n\
         \n[General]\n\n",
    );
    out.push_str(&format!(
        "; The picture behind the login. `images/background.jpg` is rendered from the\n\
         ; wallpaper below; an absolute path works too, if the greeter can read it.\n\
         background={background}\n\n",
    ));
    out.push_str(&format!(
        "; ---- typography ----\nfont={}\n\n",
        text(cfg, "font", DEFAULT_FONT)
    ));
    out.push_str("; ---- palette (theme.css, mirrored because a greeter cannot read CSS) ----\n");
    for (key, default) in [
        ("bg", DEFAULT_BG),
        ("fg", DEFAULT_FG),
        ("fg_dim", DEFAULT_FG_DIM),
        ("fg_faint", DEFAULT_FG_FAINT),
        ("accent", DEFAULT_ACCENT),
        ("on_accent", DEFAULT_ON_ACCENT),
        ("warn", DEFAULT_WARN),
        ("crit", DEFAULT_CRIT),
    ] {
        out.push_str(&format!(
            "{key}={}\n",
            palette_value(cfg, palette, key, default)
        ));
    }
    out
}

/// A colour: the file's own value, else the theme's token, else this module's copy
/// of the palette. That order is what lets a retint of `theme.css` reach the login
/// screen without the page having to write eight lines.
fn palette_value(cfg: &Config, palette: &[(String, String)], key: &str, default: &str) -> String {
    if let Some(value) = cfg
        .get(key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return value.to_string();
    }
    palette
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| default.to_string())
}

/// Whether the installed theme still carries what this page writes, in one
/// sentence for the page's reading row.
///
/// The greeter reads `/usr/share/sddm/themes/hypr-osd/`, which belongs to root, so
/// the window can write the source and render the picture but cannot install
/// them. Saying "out of date" is what turns "my change did nothing at the login
/// screen" into the one command that fixes it.
pub fn installed_state() -> String {
    let source = std::fs::read(conf_path());
    let installed = std::fs::read(Path::new(INSTALLED).join("theme.conf"));
    match (source, installed) {
        (Ok(source), Ok(installed)) if source == installed => "up to date".to_string(),
        (Ok(_), Ok(_)) => "out of date - press Install".to_string(),
        (Ok(_), Err(_)) => "not installed - press Install".to_string(),
        (Err(_), _) => "theme.conf is not written yet".to_string(),
    }
}

/// The rendered background, as a size - or the reason there is none.
pub fn image_state() -> String {
    match std::fs::metadata(image_path()) {
        Ok(meta) => format!("{} KB", (meta.len() / 1024).max(1)),
        Err(_) => "not rendered yet".to_string(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> Config {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);

        let path = std::env::temp_dir().join(format!(
            "hypr-osd-login-test-{}-{}.conf",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, text).unwrap();
        Config::load_from(path)
    }

    /// The defaults are the theme the installer ships: the rendered image, the
    /// desktop's palette, the font.
    #[test]
    fn the_defaults_are_the_theme_the_installer_renders() {
        let doc = document_from(&config(""), Some("/home/lars/x.jpg"), &[]);
        assert!(doc.contains("background=images/background.jpg\n"), "{doc}");
        assert!(doc.contains("bg=#1a1412\n"), "{doc}");
        assert!(doc.contains("accent=#f2b49b\n"), "{doc}");
        assert!(doc.contains("font=Cantarell\n"), "{doc}");
    }

    /// The installer is the theme's own, beside the files it copies - which is
    /// what the page's Install button names.
    #[test]
    fn the_installer_sits_beside_the_theme() {
        assert!(install_script().ends_with("hypr/sddm-theme/install.sh"));
    }

    /// A palette colour the file does not set comes from theme.css - the same
    /// token every other surface follows - and a line in login.conf still wins.
    #[test]
    fn an_unset_colour_follows_the_theme() {
        let palette = vec![("bg".to_string(), "#123456".to_string())];
        let doc = document_from(&config(""), None, &palette);
        assert!(doc.contains("bg=#123456\n"), "{doc}");
        let chosen = document_from(&config("bg = #ff0000\n"), None, &palette);
        assert!(chosen.contains("bg=#ff0000\n"), "{chosen}");
    }

    /// A change is carried into theme.conf, and switching the rendering off points
    /// the greeter at the picture itself.
    #[test]
    fn the_file_wins_and_can_skip_rendering() {
        let doc = document_from(
            &config("render = false\nbackground = /tmp/mine.png\naccent = #ff0000\n"),
            Some("/tmp/mine.png"),
            &[],
        );
        assert!(doc.contains("background=/tmp/mine.png\n"), "{doc}");
        assert!(doc.contains("accent=#ff0000\n"), "{doc}");
    }

    /// The rendering is one `magick` call with every knob in it, and it is not run
    /// at all when the page turns it off or there is no picture.
    #[test]
    fn the_render_command_carries_every_knob() {
        let (program, args) =
            render_command_from(&config("background = /tmp/a.jpg\n"), Some("/tmp/a.jpg")).unwrap();
        assert_eq!(program, "magick");
        let joined = args.join(" ");
        assert!(joined.contains("/tmp/a.jpg"), "{joined}");
        assert!(joined.contains("-resize 2560x1440^"), "{joined}");
        assert!(joined.contains("-blur 0x16"), "{joined}");
        assert!(joined.contains("-modulate 62,112"), "{joined}");
        assert!(joined.contains("background.jpg"), "{joined}");

        assert!(render_command_from(
            &config("render = false\nbackground = /tmp/a.jpg\n"),
            Some("/tmp/a.jpg")
        )
        .is_none());
        assert!(render_command_from(&config("render = true\n"), None).is_none());
    }
}

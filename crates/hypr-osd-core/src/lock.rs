//! The lock screen's generated half: the variables `hyprlock.conf` is written in
//! terms of.
//!
//! hyprlock is the locker — `SUPER + L`, the session card's Lock row, hypridle
//! before sleep — and its config is `~/.config/hypr/hyprlock.conf`, carried by
//! this repository as `configs/hyprlock.conf` and installed when the file is
//! missing. That file is *structure*: the glass panel, the clock, the field and
//! where each of them sits, in hyprlang, with the reasoning beside it.
//!
//! What a hand-written hyprlang file cannot carry is the half that has to keep
//! up with the desktop:
//!
//! * **the palette.** hyprlock reads hyprlang, never CSS, so it cannot read
//!   `~/.config/hypr-osd/theme.css`. Writing the twenty colours out by hand is
//!   what the file used to do, and a mirrored colour drifts the moment the theme
//!   changes - the one place in this collection where retinting did not follow.
//! * **its numbers** - blur, dim, the panel's size, the behaviour flags - which
//!   are exactly the things a person nudges, and nudging them meant editing
//!   hyprlang by hand.
//!
//! So `hyprlock.conf` starts with `source = ~/.config/hypr/hyprlock-vars.conf`
//! and this module writes that file: the palette, converted to hyprlock's
//! `rgb()`/`rgba()` dialect, and the knobs of `~/.config/hypr-osd/lock.conf` -
//! the same per-element `key = value` file every element reads, edited by the
//! settings window's **Lock screen** page (see [`crate::settings`]).
//!
//! hyprlang's `source` is what makes it work at all: it is parsed inline, so the
//! variables exist before the structure that uses them, and `$name` is
//! substituted while the structure is read.
//!
//! Nothing has to be restarted for a change to take effect - hyprlock reads its
//! config at every lock - so the file is written
//!
//! * by `hypr-osd-lock-vars`, which the installer runs after writing the
//!   structure and which is installed so a retint can be applied by hand,
//! * when the settings window opens, and on every change on its Lock screen page.
//!
//! The values are the ones `configs/hyprlock.conf` was tuned to, so a machine that
//! has never touched `lock.conf` renders exactly the lock screen it had before
//! the palette moved here. A test holds this module and the settings page
//! together: every row on that page is a variable this file defines.

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::config::{self, Config};
use crate::css;

/// The settings file the Lock screen page edits: `~/.config/hypr-osd/lock.conf`.
pub const FILE: &str = "lock";

/// The generated variables, next to the hand-written structure that sources them.
pub const VARS: &str = "hyprlock-vars.conf";

/// The hand-written structure. Not read here - named so "where this goes" is one
/// constant in one place.
pub const CONF: &str = "hyprlock.conf";

// --- the defaults ----------------------------------------------------------
//
// The `configs/hyprlock.conf` numbers. They are `&str` (or `bool` for a switch) so
// that the settings page and the generator below can share them instead of
// spelling each value twice: the page shows the default beside an unset key, and
// this module writes the same value into the variables when the key is absent.

/// Empty means "the picture hyprpaper draws": see [`wallpaper`].
pub const DEFAULT_WALLPAPER: &str = "";
pub const DEFAULT_BLUR_PASSES: &str = "1";
pub const DEFAULT_BLUR_SIZE: &str = "1";
pub const DEFAULT_BRIGHTNESS: &str = "0.7";
pub const DEFAULT_CONTRAST: &str = "1.02";
pub const DEFAULT_VIBRANCY: &str = "0.1";
pub const DEFAULT_VIBRANCY_DARKNESS: &str = "0.1";
pub const DEFAULT_NOISE: &str = "0.004";
pub const DEFAULT_PANEL_WIDTH: &str = "800";
pub const DEFAULT_PANEL_HEIGHT: &str = "670";
pub const DEFAULT_HIDE_CURSOR: bool = true;
pub const DEFAULT_IGNORE_EMPTY_INPUT: bool = true;
pub const DEFAULT_IMMEDIATE_RENDER: bool = true;
pub const DEFAULT_FAIL_TIMEOUT: &str = "2000";

/// `~/.config/hypr/hyprlock-vars.conf` - what [`write_vars`] fills in.
pub fn vars_path() -> PathBuf {
    config::config_home().join("hypr").join(VARS)
}

/// `~/.config/hypr/hyprlock.conf` - the structure this module feeds. It is
/// written by the installer, never here; only its name is needed (the settings
/// page prints it, and the header of the generated file names it).
pub fn conf_path() -> PathBuf {
    config::config_home().join("hypr").join(CONF)
}

/// The wallpaper the desktop is using, out of hyprpaper's own config, so that the
/// lock screen and the desktop can only disagree if hyprpaper's config says so.
/// `None` when that file cannot be read or names no picture.
pub fn wallpaper() -> Option<String> {
    let path = config::config_home().join("hypr").join("hyprpaper.conf");
    wallpaper_from(&std::fs::read_to_string(path).ok()?)
}

/// The `path = …` line of a hyprpaper config (`wallpaper { … }` block; 0.8 and
/// later dropped the older `wallpaper = monitor,path` form). The last one wins,
/// which is what hyprpaper does with a config that names two pictures.
pub fn wallpaper_from(text: &str) -> Option<String> {
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

/// Write the variables. Returns the file, or why it could not be written - the
/// settings page shows that instead of pretending the change stuck.
pub fn write_vars() -> Result<PathBuf, String> {
    let path = vars_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    std::fs::write(&path, document())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(path)
}

/// The file, from the machine it is on (the theme's palette, the user's knobs).
pub fn document() -> String {
    document_from(&Config::load(FILE), &css::tokens())
}

/// The file, from a palette and a config handed in - which is what makes this
/// testable without a home directory.
pub fn document_from(cfg: &Config, palette: &[(String, String)]) -> String {
    let mut out = String::new();

    out.push_str(
        "# GENERATED FILE - do not edit.\n\
         #\n\
         # The variables `~/.config/hypr/hyprlock.conf` is written in terms of: the\n\
         # palette, cut from ~/.config/hypr-osd/theme.css, and the knobs on the settings\n\
         # window's Lock screen page (~/.config/hypr-osd/lock.conf). hyprlock cannot read\n\
         # CSS, so this file is how the desktop's one palette reaches the lock screen.\n\
         #\n\
         # Written by `hypr-osd-lock-vars` (the installer runs it, the settings window\n\
         # runs it when it opens and on every change on that page). Edit the theme or\n\
         # lock.conf instead - the next write replaces everything here.\n\
         \n\
         # ---- palette (theme.css, as hyprlock's rgb()/rgba()) ----\n",
    );
    for (name, value) in palette {
        let _ = writeln!(out, "${name} = {}", hypr_color(value));
    }

    // The colours that are not tokens but are *cut* from them, so that nothing on
    // this screen is invented here - the same three the hand-written file had to
    // spell out as literals.
    let bg = token(palette, "bg");
    let accent = token(palette, "accent");
    let accent2 = token(palette, "accent2");
    out.push_str(
        "\n# ---- cut from the palette ----\n\
         # The accent at 60 %: the rule under the clock, and the field's outline at rest.\n",
    );
    let _ = writeln!(out, "$accent_soft = {}", with_alpha(&accent, 0.60));
    out.push_str("# The accent at 8 %: the fill of the password field.\n");
    let _ = writeln!(out, "$glass = {}", with_alpha(&accent, 0.08));
    out.push_str("# $bg at 65 %: the sheet of glass the whole arrangement sits on.\n");
    let _ = writeln!(out, "$panel_color = {}", with_alpha(&bg, 0.65));
    out.push_str(
        "# The field's outline - the one place the accent and the accent2 meet, the pair\n\
         # the bar's progress fill uses.\n",
    );
    let _ = writeln!(
        out,
        "$outline_gradient = {} {}",
        with_alpha(&accent, 0.40),
        with_alpha(&accent2, 0.20)
    );

    // The clock, colon and all. It is a whole value rather than a colour because
    // the colon is pango markup, and pango cannot see hyprlang variables - the
    // `##` is hyprlang's escape for the `#` that markup needs.
    let accent_hex = rgb_hex(&accent).unwrap_or_else(|| "33ccff".to_string());
    let _ = writeln!(
        out,
        "\n# The clock: hours, an accent colon, minutes. The colon is markup, which is why\n\
         # this is a whole value and why the `#` is doubled.\n\
         $clock_text = cmd[update:1000] echo \"$(date +'%H')<span foreground='##{accent_hex}'>:</span>$(date +'%M')\""
    );

    // ---- the background ----
    let wallpaper = {
        let chosen = text(cfg, "wallpaper", DEFAULT_WALLPAPER);
        if chosen.is_empty() {
            wallpaper().unwrap_or_else(|| "screenshot".to_string())
        } else {
            chosen
        }
    };
    out.push_str(
        "\n# ---- the background ----\n\
         # The picture behind the lock: hyprpaper's own wallpaper unless `wallpaper` in\n\
         # lock.conf names another one. `screenshot` is the fallback on a machine with no\n\
         # hyprpaper config at all - the live desktop, re-taken at every lock.\n",
    );
    let _ = writeln!(out, "$wallpaper = {wallpaper}");
    let _ = writeln!(out, "$blur_passes = {}", text(cfg, "blur_passes", DEFAULT_BLUR_PASSES));
    let _ = writeln!(out, "$blur_size = {}", text(cfg, "blur_size", DEFAULT_BLUR_SIZE));
    let _ = writeln!(out, "$brightness = {}", text(cfg, "brightness", DEFAULT_BRIGHTNESS));
    let _ = writeln!(out, "$contrast = {}", text(cfg, "contrast", DEFAULT_CONTRAST));
    let _ = writeln!(out, "$vibrancy = {}", text(cfg, "vibrancy", DEFAULT_VIBRANCY));
    let _ = writeln!(
        out,
        "$vibrancy_darkness = {}",
        text(cfg, "vibrancy_darkness", DEFAULT_VIBRANCY_DARKNESS)
    );
    let _ = writeln!(out, "$noise = {}", text(cfg, "noise", DEFAULT_NOISE));

    // ---- the glass panel ----
    out.push_str("\n# ---- the glass panel ----\n");
    let _ = writeln!(out, "$panel_width = {}", text(cfg, "panel_width", DEFAULT_PANEL_WIDTH));
    let _ = writeln!(
        out,
        "$panel_height = {}",
        text(cfg, "panel_height", DEFAULT_PANEL_HEIGHT)
    );

    // ---- behaviour ----
    out.push_str("\n# ---- behaviour ----\n");
    let _ = writeln!(
        out,
        "$hide_cursor = {}",
        flag(cfg, "hide_cursor", DEFAULT_HIDE_CURSOR)
    );
    let _ = writeln!(
        out,
        "$ignore_empty_input = {}",
        flag(cfg, "ignore_empty_input", DEFAULT_IGNORE_EMPTY_INPUT)
    );
    let _ = writeln!(
        out,
        "$immediate_render = {}",
        flag(cfg, "immediate_render", DEFAULT_IMMEDIATE_RENDER)
    );
    let _ = writeln!(out, "$fail_timeout = {}", text(cfg, "fail_timeout", DEFAULT_FAIL_TIMEOUT));

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

/// A palette token by name. Missing is a bug, not a colour, so it is spelled in
/// magenta rather than guessed at.
fn token(palette: &[(String, String)], name: &str) -> String {
    palette
        .iter()
        .find(|(known, _)| known == name)
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| {
            eprintln!("hypr-osd: the palette has no `{name}` token");
            "rgb(ff00ff)".to_string()
        })
}

/// A CSS colour as hyprlock spells it: `#rrggbb` -> `rgb(rrggbb)`,
/// `#rrggbbaa` -> `rgba(rrggbbaa)`, and CSS `rgba(r, g, b, a)` ->
/// `rgba(rrggbbAA)`. Hyprland's colour parser has no CSS `rgba()` form, so the
/// conversion is not cosmetic - a value passed through unchanged is a colour
/// hyprlock refuses.
pub fn hypr_color(value: &str) -> String {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let hex = hex.to_ascii_lowercase();
        return match hex.len() {
            3 => format!("rgb({})", hex.chars().flat_map(|c| [c, c]).collect::<String>()),
            6 => format!("rgb({hex})"),
            8 => format!("rgba({hex})"),
            _ => value.to_string(),
        };
    }
    if let Some((rgb, alpha)) = rgba(value) {
        // Fully opaque is `rgb()`: hyprlock's parser reads `rgba()` as the
        // eight-digit form, so `rgba(rrggbb, ff)` would be a colour it rejects.
        if alpha >= 1.0 {
            return format!("rgb({rgb})");
        }
        return format!("rgba({rgb}{:02x})", alpha_byte(alpha));
    }
    value.to_string()
}

/// The same colour with a different opacity - how the panel, the field's fill and
/// the outline are cut from the tokens. An unparseable colour comes back as
/// hyprlock's spelling of itself, which at least says what went wrong.
pub fn with_alpha(value: &str, alpha: f64) -> String {
    match rgba(value) {
        Some((rgb, _)) => format!("rgba({rgb}{:02x})", alpha_byte(alpha)),
        None => hypr_color(value),
    }
}

/// `#33ccff` from any spelling of an opaque-ish colour: the clock's colon needs
/// the bare hex, because pango markup has no `rgb()`.
fn rgb_hex(value: &str) -> Option<String> {
    rgba(value).map(|(rgb, _)| rgb)
}

/// A colour as `("rrggbb", alpha in 0..1)`.
fn rgba(value: &str) -> Option<(String, f64)> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let hex = hex.to_ascii_lowercase();
        return match hex.len() {
            3 => Some((
                hex.chars().flat_map(|c| [c, c]).collect::<String>(),
                1.0,
            )),
            6 => Some((hex, 1.0)),
            8 => {
                let alpha = u8::from_str_radix(&hex[6..8], 16).ok()? as f64 / 255.0;
                Some((hex[..6].to_string(), alpha))
            }
            _ => None,
        };
    }
    let rest = value
        .strip_prefix("rgba(")
        .or_else(|| value.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let channel = |text: &str| -> Option<u8> {
        let value: f64 = text.parse().ok()?;
        Some(value.round().clamp(0.0, 255.0) as u8)
    };
    let rgb = format!(
        "{:02x}{:02x}{:02x}",
        channel(parts[0])?,
        channel(parts[1])?,
        channel(parts[2])?
    );
    let alpha = match parts.get(3) {
        Some(text) => text.parse::<f64>().ok()?.clamp(0.0, 1.0),
        None => 1.0,
    };
    Some((rgb, alpha))
}

fn alpha_byte(alpha: f64) -> u8 {
    (alpha.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Vec<(String, String)> {
        [
            ("bg", "rgba(11, 15, 21, 0.92)"),
            ("fg", "#dde7ef"),
            ("fg_dim", "#8b98a8"),
            ("fg_faint", "#4d5661"),
            ("accent", "#33ccff"),
            ("accent2", "#00ff99"),
            ("warn", "#ffcc66"),
            ("crit", "#ff5566"),
            ("border", "rgba(51, 204, 255, 0.22)"),
            ("surface", "rgba(255, 255, 255, 0.045)"),
        ]
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
    }

    /// A config file of its own per call: the tests run in parallel, and a
    /// shared name would have one test reading another's file.
    fn config(text: &str) -> Config {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);

        let path = std::env::temp_dir().join(format!(
            "hypr-osd-lock-test-{}-{}.conf",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, text).unwrap();
        Config::load_from(path)
    }

    /// Every spelling theme.css uses, in the dialect hyprlock parses.
    #[test]
    fn css_colours_become_hyprlocks_dialect() {
        assert_eq!(hypr_color("#dde7ef"), "rgb(dde7ef)");
        assert_eq!(hypr_color("#33ccff80"), "rgba(33ccff80)");
        assert_eq!(hypr_color("#fff"), "rgb(ffffff)");
        assert_eq!(hypr_color("rgba(11, 15, 21, 0.92)"), "rgba(0b0f15eb)");
        assert_eq!(hypr_color("rgb(51, 204, 255)"), "rgb(33ccff)");
        // Not a colour we know: passed through, so the error names itself.
        assert_eq!(hypr_color("tomato"), "tomato");
    }

    /// The three cut colours keep the exact literals the hand-written file used,
    /// which is what makes the generated file a drop-in replacement.
    #[test]
    fn the_cut_colours_are_the_ones_the_file_had() {
        let bg = token(&palette(), "bg");
        let accent = token(&palette(), "accent");
        let accent2 = token(&palette(), "accent2");
        assert_eq!(with_alpha(&bg, 0.65), "rgba(0b0f15a6)");
        assert_eq!(with_alpha(&accent, 0.60), "rgba(33ccff99)");
        assert_eq!(with_alpha(&accent, 0.08), "rgba(33ccff14)");
        assert_eq!(format!("{} {}", with_alpha(&accent, 0.40), with_alpha(&accent2, 0.20)), "rgba(33ccff66) rgba(00ff9933)");
    }

    /// An untouched `lock.conf` produces the numbers `configs/hyprlock.conf` was
    /// tuned to - the file the installer ships has to look the same as it did.
    #[test]
    fn the_defaults_are_the_tuned_ones() {
        let doc = document_from(&config(""), &palette());
        assert!(doc.contains("$blur_size = 1\n"), "{doc}");
        assert!(doc.contains("$brightness = 0.7\n"), "{doc}");
        assert!(doc.contains("$panel_width = 800\n"), "{doc}");
        assert!(doc.contains("$panel_height = 670\n"), "{doc}");
        assert!(doc.contains("$hide_cursor = true\n"), "{doc}");
        assert!(doc.contains("$fail_timeout = 2000\n"), "{doc}");
        assert!(doc.contains("$fg = rgb(dde7ef)\n"), "{doc}");
    }

    /// A key the file sets wins over the default, and an empty value is "unset".
    #[test]
    fn the_file_wins_over_the_defaults() {
        let doc = document_from(
            &config("blur_size = 12\nhide_cursor = false\nbrightness =\n"),
            &palette(),
        );
        assert!(doc.contains("$blur_size = 12\n"), "{doc}");
        assert!(doc.contains("$hide_cursor = false\n"), "{doc}");
        assert!(doc.contains("$brightness = 0.7\n"), "{doc}");
    }

    /// The clock's colon is markup, so it is a whole value - and it carries the
    /// accent, which is the point of generating it rather than writing it out.
    #[test]
    fn the_clock_text_carries_the_accent() {
        let doc = document_from(&config(""), &palette());
        assert!(
            doc.contains("$clock_text = cmd[update:1000] echo \"$(date +'%H')<span foreground='##33ccff'>:</span>$(date +'%M')\""),
            "{doc}"
        );
    }

    /// The wallpaper follows hyprpaper unless the page's `wallpaper` row says
    /// otherwise, and the fallback is the live desktop rather than nothing.
    #[test]
    fn the_wallpaper_follows_hyprpaper() {
        let hyprpaper = "splash = false\n\nwallpaper {\n    monitor =\n    \
                         path = /home/lars/Bilder/Wallpapers/1022522.jpg\n    fit_mode = cover\n}\n";
        assert_eq!(
            wallpaper_from(hyprpaper),
            Some("/home/lars/Bilder/Wallpapers/1022522.jpg".to_string())
        );
        // A commented-out path is a comment, and no path at all is `None`.
        assert_eq!(wallpaper_from("# path = /nope.jpg\n"), None);
        assert_eq!(wallpaper_from("fit_mode = cover\n"), None);

        let chosen = document_from(&config("wallpaper = /tmp/mine.jpg\n"), &palette());
        assert!(chosen.contains("$wallpaper = /tmp/mine.jpg\n"), "{chosen}");
    }

    /// The settings page and this module name each other: every row the Lock
    /// screen page draws is a variable the generated file defines, and every knob
    /// variable is a row. Without that, a switch would sit on a page and change
    /// nothing, or a variable would have no way of being edited.
    #[test]
    fn the_lock_page_carries_every_variable() {
        let doc = document_from(&config(""), &palette());
        let rows: Vec<&str> = crate::settings::LOCK
            .iter()
            .map(|setting| setting.key)
            .collect();
        for key in &rows {
            assert!(
                doc.contains(&format!("${key} = ")),
                "the Lock screen page draws `{key}` but the generated file has no `${key}`"
            );
        }
        // The knobs are the section before `# ---- cut from the palette ----` and
        // the ones after it; the palette itself is not editable from that page.
        for line in doc.lines() {
            let Some(rest) = line.strip_prefix('$') else {
                continue;
            };
            let Some((name, _)) = rest.split_once('=') else {
                continue;
            };
            let name = name.trim();
            let palette_token = palette().iter().any(|(token, _)| token == name);
            let cut = matches!(
                name,
                "accent_soft" | "glass" | "panel_color" | "outline_gradient" | "clock_text"
            );
            assert!(
                palette_token || cut || rows.contains(&name),
                "`${name}` is written by the generator but is neither a palette token nor a \
                 row on the Lock screen page"
            );
        }
    }
}

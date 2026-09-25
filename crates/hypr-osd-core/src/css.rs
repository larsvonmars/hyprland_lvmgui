//! The GTK4 stylesheet every element wears.
//!
//! GTK CSS providers are per process, so a stylesheet cannot be *imported* from
//! another process's - the palette has to be pasted into each one. It is read at
//! runtime from `~/.config/hypr-osd/theme.css` (installed from `configs/` in this
//! repository), which is the desktop's single source of truth for colour: the
//! bar, every card, the island popup and the lock screen are all painted from
//! the same `@tokens`. That is what [`palette`] returns, and it is why editing
//! one file rethemes the whole desktop.
//!
//! (This used to read `~/.config/waybar/style.css`. It does not any more:
//! waybar is not part of this desktop - see the README.)

use std::fs;
use std::path::PathBuf;

/// The stylesheet all elements share: typography, the transparent surface, the
/// card and bar recipes. See the file - it documents itself.
pub const BASE: &str = include_str!("base.css");

/// A mirror of the `@define-color` block in `~/.config/hypr-osd/theme.css`, in
/// that file's order. Only a safety net: `palette()` prefers whatever the real
/// theme file says and fills the gaps from here, so an element can never
/// reference an undefined colour (GTK drops such a declaration silently, which
/// looks like a styling bug). It also has to be here for a daemon started
/// before the installer ever ran.
const FALLBACK: &[(&str, &str)] = &[
    ("bg", "rgba(11, 15, 21, 0.92)"),
    ("bg_gradient", "rgba(30, 38, 48, 0.55)"),
    ("pill_top", "rgba(20, 26, 34, 0.96)"),
    ("pill_bottom", "rgba(6, 9, 13, 0.98)"),
    ("card_top", "#131921"),
    ("card_bottom", "#06090d"),
    ("surface", "rgba(255, 255, 255, 0.045)"),
    ("surface_h", "rgba(255, 255, 255, 0.10)"),
    ("hairline", "rgba(255, 255, 255, 0.06)"),
    ("hairline_soft", "rgba(255, 255, 255, 0.07)"),
    ("border", "rgba(51, 204, 255, 0.22)"),
    ("border_soft", "rgba(51, 204, 255, 0.18)"),
    ("border_h", "rgba(51, 204, 255, 0.42)"),
    ("fg", "#dde7ef"),
    ("fg_dim", "#8b98a8"),
    ("fg_faint", "#4d5661"),
    ("on_accent", "#0a1216"),
    ("accent", "#33ccff"),
    ("accent2", "#00ff99"),
    ("warn", "#ffcc66"),
    ("crit", "#ff5566"),
    ("accent_tint", "rgba(51, 204, 255, 0.14)"),
    ("accent2_tint", "rgba(0, 255, 153, 0.12)"),
    ("warn_tint", "rgba(255, 204, 102, 0.14)"),
    ("crit_tint", "rgba(255, 85, 102, 0.16)"),
    ("glow", "rgba(51, 204, 255, 0.25)"),
    ("shadow", "rgba(0, 0, 0, 0.45)"),
];

/// The single source of truth for the palette, in a `$XDG_CONFIG_HOME`-aware
/// way: `~/.config/hypr-osd/theme.css`, installed from this repository's
/// `configs/theme.css`.
pub fn theme_file() -> PathBuf {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    config_home.join(crate::config::DIR).join("theme.css")
}

/// `@define-color name value;` for every token, ready to be prepended to a
/// stylesheet. Values come from the theme file when it can be read, and from
/// [`FALLBACK`] otherwise.
pub fn palette() -> String {
    let mut tokens: Vec<(String, String)> = FALLBACK
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect();

    if let Ok(text) = fs::read_to_string(theme_file()) {
        for line in text.lines() {
            let Some((name, value)) = parse_define_color(line) else {
                continue;
            };
            match tokens.iter_mut().find(|(known, _)| *known == name) {
                // Keep the mirror's order so the output is stable and diffable.
                Some(slot) => slot.1 = value,
                None => tokens.push((name, value)),
            }
        }
    }

    tokens
        .iter()
        .map(|(name, value)| format!("@define-color {name} {value};\n"))
        .collect()
}

/// One line of the `@define-color` block, e.g.
/// `@define-color card_top      #131921;` -> `("card_top", "#131921")`.
fn parse_define_color(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix("@define-color")?;
    let rest = rest.trim();
    let (name, value) = rest.split_once(char::is_whitespace)?;
    let value = value.trim().trim_end_matches(';').trim();
    if name.is_empty() || value.is_empty() {
        return None;
    }
    Some((name.to_string(), value.to_string()))
}

/// An element's complete stylesheet: palette + shared base + the element's own
/// rules (which is where everything specific to one element belongs).
pub fn stylesheet(element_css: &str) -> String {
    format!("{}\n{}\n{}", palette(), BASE, element_css)
}

/// A stylesheet from the shared base plus an element's own rules, without the
/// palette - for a *second* surface in a process that has already installed
/// one. Installing the palette twice would be harmless but would also let two
/// copies of the same tokens drift apart; this is how the bar adds the island
/// popup's rules to the stylesheet it already wears.
pub fn stylesheet_without_palette(element_css: &str) -> String {
    format!("{BASE}\n{element_css}")
}

/// Install a stylesheet for the whole app. Parse errors are printed instead of
/// being swallowed: GTK silently drops a declaration it cannot read, which from
/// the outside looks like "my CSS did nothing".
pub fn install(stylesheet: &str) {
    let provider = gtk::CssProvider::new();
    provider.connect_parsing_error(|_, section, error| {
        let location = section.start_location();
        eprintln!(
            "hypr-osd: CSS error (line {}): {error}",
            location.lines() + 1
        );
    });
    provider.load_from_string(stylesheet);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_define_color_line() {
        assert_eq!(
            parse_define_color("@define-color card_top      #131921;"),
            Some(("card_top".to_string(), "#131921".to_string()))
        );
        // Values may contain spaces, e.g. rgba().
        assert_eq!(
            parse_define_color("  @define-color border rgba(51, 204, 255, 0.22);"),
            Some(("border".to_string(), "rgba(51, 204, 255, 0.22)".to_string()))
        );
        assert_eq!(parse_define_color("/* ---------------- palette */"), None);
        assert_eq!(parse_define_color("@define-color"), None);
    }

    /// The theme file that ships in this repository is what the palette falls
    /// back to when the installed copy is missing, so the two must agree: a
    /// token added to one and not the other would show up as an undefined
    /// colour in one of the two situations.
    const SHIPPED_THEME: &str = include_str!("../../../configs/theme.css");

    #[test]
    fn palette_defines_every_fallback_token() {
        let css = palette();
        for (name, value) in FALLBACK {
            assert!(
                css.contains(&format!("@define-color {name} {value};")),
                "{name} is missing from the palette"
            );
        }
    }

    #[test]
    fn the_shipped_theme_defines_every_token_the_mirror_knows() {
        for (name, value) in FALLBACK {
            assert_eq!(
                parse_define_color(&format!("@define-color {name} {value};")),
                Some(((*name).to_string(), (*value).to_string())),
                "the mirror is not parseable"
            );
            let shipped = SHIPPED_THEME
                .lines()
                .find_map(|line| match parse_define_color(line) {
                    Some((found, value)) if found == *name => Some(value),
                    _ => None,
                });
            assert_eq!(
                shipped.as_deref(),
                Some(*value),
                "configs/theme.css and the built-in mirror disagree about `{name}`"
            );
        }
    }
}

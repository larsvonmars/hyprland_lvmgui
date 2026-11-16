//! The lock screen, generated.
//!
//! hyprlock is the locker — `SUPER + L`, the session card's Lock row, hypridle
//! before sleep — and its config is `~/.config/hypr/hyprlock.conf`. That file is
//! the whole look of the screen: the wallpaper treatment, the glass panel, the
//! clock and the date, the password field and the resting hint, each with its
//! position, size and colour. hyprlock also cannot read CSS, so the palette has to
//! be written out for it.
//!
//! So the file is **generated** here, from
//!
//! * `~/.config/hypr-osd/lock.conf` — the knobs the settings window's **Lock
//!   screen** page edits, one row per option; and
//! * `~/.config/hypr-osd/theme.css` — the one palette every element draws from
//!   (see [`crate::css`]), converted to hyprlock's `rgb()`/`rgba()` dialect.
//!
//! Every option hyprlock 0.9 has for the widgets this screen uses is a row on that
//! page and a line in this file: `general`, `auth`, `animations`, `background`,
//! and the `shape`, `label` and `input-field` widgets. Nothing has to be restarted
//! for a change to take effect — hyprlock reads its config at every lock — so the
//! file is written
//!
//! * by `hypr-osd-lock-vars`, which the installer runs after installing the
//!   binaries and which is installed so a retint can be applied by hand,
//! * when the settings window opens, and on every change on its Lock screen page.
//!
//! The defaults are the values `configs/hyprlock.conf` was tuned to, so a machine
//! that has never opened the page renders exactly the lock screen it had before
//! this module took the file over.
//!
//! Check an edit without locking the session:
//!
//! ```text
//! WAYLAND_DISPLAY=hypr-osd-check hyprlock --config ~/.config/hypr/hyprlock.conf -v
//! ```
//!
//! It parses first (with line numbers on any error), then fails to connect.

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::config::{self, Config};
use crate::css;

/// The settings file the Lock screen page edits: `~/.config/hypr-osd/lock.conf`.
pub const FILE: &str = "lock";

/// The generated file, next to the compositor's own config.
pub const CONF: &str = "hyprlock.conf";

// --- defaults ---------------------------------------------------------------
//
// `&str` (or `bool` for a switch) so the settings page and the generator share
// one copy: the page shows the default beside an unset key, and the generator
// writes the same value when the key is absent. These are hyprlock 0.9's own
// defaults where the tuned file had no opinion, and the tuned file's numbers
// where it did.

/// Empty means "the picture hyprpaper draws": see [`wallpaper`].
pub const DEFAULT_WALLPAPER: &str = "";
pub const DEFAULT_MONITOR: &str = "";
pub const DEFAULT_BLUR_PASSES: &str = "1";
pub const DEFAULT_BLUR_SIZE: &str = "1";
pub const DEFAULT_BRIGHTNESS: &str = "0.70";
pub const DEFAULT_CONTRAST: &str = "1.02";
pub const DEFAULT_VIBRANCY: &str = "0.10";
pub const DEFAULT_VIBRANCY_DARKNESS: &str = "0.10";
pub const DEFAULT_NOISE: &str = "0.004";
pub const DEFAULT_RELOAD_TIME: &str = "-1";
pub const DEFAULT_CROSSFADE_TIME: &str = "-1.0";

pub const DEFAULT_HIDE_CURSOR: bool = true;
pub const DEFAULT_IGNORE_EMPTY_INPUT: bool = true;
pub const DEFAULT_IMMEDIATE_RENDER: bool = true;
pub const DEFAULT_TEXT_TRIM: bool = true;
pub const DEFAULT_FRACTIONAL_SCALING: &str = "2";
pub const DEFAULT_SCREENCOPY_MODE: &str = "0";
pub const DEFAULT_FAIL_TIMEOUT: &str = "2000";
pub const DEFAULT_ANIMATIONS: bool = true;

pub const DEFAULT_PAM_ENABLED: bool = true;
pub const DEFAULT_PAM_MODULE: &str = "hyprlock";
pub const DEFAULT_FINGERPRINT_ENABLED: bool = false;
pub const DEFAULT_FINGERPRINT_READY_MESSAGE: &str = "(Scan fingerprint to unlock)";
pub const DEFAULT_FINGERPRINT_PRESENT_MESSAGE: &str = "Scanning fingerprint";
pub const DEFAULT_FINGERPRINT_RETRY_DELAY: &str = "250";

pub const DEFAULT_PANEL_WIDTH: &str = "800";
pub const DEFAULT_PANEL_HEIGHT: &str = "670";
pub const DEFAULT_PANEL_X: &str = "0";
pub const DEFAULT_PANEL_Y: &str = "0";
pub const DEFAULT_PANEL_ROUNDING: &str = "32";
pub const DEFAULT_PANEL_BORDER: &str = "1";
pub const DEFAULT_PANEL_OPACITY: &str = "0.65";
pub const DEFAULT_PANEL_SHADOW_SIZE: &str = "36";
pub const DEFAULT_PANEL_SHADOW_PASSES: &str = "2";
pub const DEFAULT_PANEL_SHADOW_BOOST: &str = "1.0";
pub const DEFAULT_PANEL_SHADOW_OPACITY: &str = "0.65";

pub const DEFAULT_CLOCK_FONT: &str = "MesloLGS Nerd Font Mono";
pub const DEFAULT_CLOCK_SIZE: &str = "170";
pub const DEFAULT_CLOCK_X: &str = "0";
pub const DEFAULT_CLOCK_Y: &str = "109";
pub const DEFAULT_CLOCK_FORMAT: &str = "%H:%M";
pub const DEFAULT_CLOCK_ACCENT_COLON: bool = true;
pub const DEFAULT_DATE_FORMAT: &str = "%A, %-d %B";
pub const DEFAULT_DATE_SIZE: &str = "26";
pub const DEFAULT_DATE_X: &str = "0";
pub const DEFAULT_DATE_Y: &str = "259";
pub const DEFAULT_DATE_UPPERCASE: bool = true;

pub const DEFAULT_SHOW_RULE: bool = true;
pub const DEFAULT_RULE_WIDTH: &str = "84";
pub const DEFAULT_RULE_HEIGHT: &str = "3";
pub const DEFAULT_RULE_Y: &str = "-31";
pub const DEFAULT_RULE_ROUNDING: &str = "2";
pub const DEFAULT_RULE_OPACITY: &str = "0.60";

pub const DEFAULT_SHOW_LOCK_ICON: bool = true;
pub const DEFAULT_LOCK_ICON: &str = "󰌾";
pub const DEFAULT_LOCK_SIZE: &str = "44";
pub const DEFAULT_LOCK_Y: &str = "-91";

pub const DEFAULT_FIELD_FONT: &str = "MesloLGS Nerd Font Mono";
pub const DEFAULT_FIELD_WIDTH: &str = "440";
pub const DEFAULT_FIELD_HEIGHT: &str = "78";
pub const DEFAULT_FIELD_X: &str = "0";
pub const DEFAULT_FIELD_Y: &str = "-186";
pub const DEFAULT_FIELD_ROUNDING: &str = "20";
pub const DEFAULT_FIELD_TINT: &str = "0.08";
pub const DEFAULT_FIELD_OUTLINE: &str = "2";
pub const DEFAULT_FIELD_OUTLINE_ACCENT: &str = "0.40";
pub const DEFAULT_FIELD_OUTLINE_ACCENT2: &str = "0.20";
pub const DEFAULT_DOTS_SIZE: &str = "0.30";
pub const DEFAULT_DOTS_SPACING: &str = "0.20";
pub const DEFAULT_DOTS_CENTER: bool = true;
pub const DEFAULT_DOTS_ROUNDING: &str = "-1";
pub const DEFAULT_DOTS_TEXT_FORMAT: &str = "";
pub const DEFAULT_PLACEHOLDER_TEXT: &str = "<i>Password</i>";
pub const DEFAULT_CHECK_TEXT: &str = "<i>Verifying…</i>";
pub const DEFAULT_FAIL_TEXT: &str = "$FAIL ($ATTEMPTS)";
pub const DEFAULT_FADE_ON_EMPTY: bool = true;
pub const DEFAULT_FADE_TIMEOUT: &str = "0";
pub const DEFAULT_HIDE_INPUT: bool = false;
pub const DEFAULT_SWAP_FONT_COLOR: bool = false;
pub const DEFAULT_INVERT_NUMLOCK: bool = false;

pub const DEFAULT_HINT_TEXT: &str = "$ATTEMPTS[Type your password]";
pub const DEFAULT_HINT_SIZE: &str = "20";
pub const DEFAULT_HINT_X: &str = "0";
pub const DEFAULT_HINT_Y: &str = "-266";

/// `~/.config/hypr/hyprlock.conf` - what [`write`] fills in.
pub fn conf_path() -> PathBuf {
    config::config_home().join("hypr").join(CONF)
}

/// The wallpaper the desktop is using, out of hyprpaper's own config, so that the
/// lock screen and the desktop can only disagree if hyprpaper's config says so.
/// `None` when that file cannot be read or names no picture.
///
/// The Wallpaper page's own file is the fallback: a picture chosen there but not
/// yet handed to hyprpaper (or a machine whose hyprpaper config is missing) should
/// still be the one the lock screen follows, rather than `screenshot`.
pub fn wallpaper() -> Option<String> {
    let path = config::config_home().join("hypr").join("hyprpaper.conf");
    wallpaper_from(&std::fs::read_to_string(path).ok()?).or_else(|| {
        let chosen = Config::load(crate::wallpaper::FILE)
            .get("path")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        chosen
    })
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

/// Write the generated lock screen config. Returns the file, or why it could not
/// be written - the settings page shows that instead of pretending the change
/// stuck.
pub fn write() -> Result<PathBuf, String> {
    let path = conf_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    // A hand-written file from before the lock screen was generated is kept once:
    // the generated one replaces it, and the old one may carry measurements the
    // settings page does not expose yet.
    if let Ok(text) = std::fs::read_to_string(&path) {
        let backup = path.with_extension("conf.bak-handwritten");
        if !text.starts_with("# GENERATED FILE") && !backup.exists() {
            let _ = std::fs::copy(&path, &backup);
        }
    }
    std::fs::write(&path, document())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(path)
}

/// The file, from the machine it is on (the theme's palette, the user's knobs, the
/// wallpaper hyprpaper draws).
pub fn document() -> String {
    document_from(&Config::load(FILE), &css::tokens(), wallpaper().as_deref())
}

/// The file, from a config, a palette and the wallpaper that is in force - which
/// is what makes this testable without a home directory.
pub fn document_from(cfg: &Config, palette: &[(String, String)], follow: Option<&str>) -> String {
    let mut out = String::new();

    out.push_str(
        "# GENERATED FILE - do not edit.\n\
         #\n\
         # The lock screen: SUPER + L, the session card's Lock row, hypridle before sleep.\n\
         # hyprlock reads this file at every lock, so a change is live without a restart.\n\
         #\n\
         # Written by `hypr-osd-lock-vars` (the installer runs it; the settings window runs\n\
         # it when it opens and on every change on its Lock screen page) from\n\
         # ~/.config/hypr-osd/lock.conf and the palette in ~/.config/hypr-osd/theme.css.\n\
         # hyprlock cannot read CSS, which is why the colours are spelled out here.\n\
         #\n\
         # To check an edit without locking yourself out, point hyprlock at a display that\n\
         # does not exist - it reads the config first, prints errors with line numbers, and\n\
         # only then fails to connect:\n\
         #\n\
         #     WAYLAND_DISPLAY=none hyprlock --config ~/.config/hypr/hyprlock.conf -v\n\
         \n",
    );

    // ---- colours ----
    let bg = token(palette, "bg");
    let fg = token(palette, "fg");
    let fg_dim = token(palette, "fg_dim");
    let fg_faint = token(palette, "fg_faint");
    let accent = token(palette, "accent");
    let accent2 = token(palette, "accent2");
    let warn = token(palette, "warn");
    let crit = token(palette, "crit");
    let border = token(palette, "border");

    // ---- general, auth, animations ----
    let _ = write!(
        out,
        "# ---- behaviour ----\n\
         # The cursor is not part of a lock screen; an empty Enter is not a wrong password\n\
         # (so it should not spend a PAM attempt); and a failed attempt stays red for the\n\
         # configured time before the field fades back.\n\
         general {{\n\
         \x20   hide_cursor = {}\n\
         \x20   ignore_empty_input = {}\n\
         \x20   immediate_render = {}\n\
         \x20   text_trim = {}\n\
         \x20   fractional_scaling = {}\n\
         \x20   screencopy_mode = {}\n\
         \x20   fail_timeout = {}\n\
         }}\n\
         \n\
         # ---- authentication ----\n\
         auth {{\n\
         \x20   pam:enabled = {}\n\
         \x20   pam:module = {}\n\
         \x20   fingerprint:enabled = {}\n\
         \x20   fingerprint:ready_message = {}\n\
         \x20   fingerprint:present_message = {}\n\
         \x20   fingerprint:retry_delay = {}\n\
         }}\n\
         \n\
         # ---- animations ----\n\
         animations {{\n\
         \x20   enabled = {}\n\
         }}\n",
        flag(cfg, "hide_cursor", DEFAULT_HIDE_CURSOR),
        flag(cfg, "ignore_empty_input", DEFAULT_IGNORE_EMPTY_INPUT),
        flag(cfg, "immediate_render", DEFAULT_IMMEDIATE_RENDER),
        flag(cfg, "text_trim", DEFAULT_TEXT_TRIM),
        text(cfg, "fractional_scaling", DEFAULT_FRACTIONAL_SCALING),
        text(cfg, "screencopy_mode", DEFAULT_SCREENCOPY_MODE),
        text(cfg, "fail_timeout", DEFAULT_FAIL_TIMEOUT),
        flag(cfg, "pam_enabled", DEFAULT_PAM_ENABLED),
        text(cfg, "pam_module", DEFAULT_PAM_MODULE),
        flag(cfg, "fingerprint_enabled", DEFAULT_FINGERPRINT_ENABLED),
        text(
            cfg,
            "fingerprint_ready_message",
            DEFAULT_FINGERPRINT_READY_MESSAGE
        ),
        text(
            cfg,
            "fingerprint_present_message",
            DEFAULT_FINGERPRINT_PRESENT_MESSAGE
        ),
        text(
            cfg,
            "fingerprint_retry_delay",
            DEFAULT_FINGERPRINT_RETRY_DELAY
        ),
        flag(cfg, "animations", DEFAULT_ANIMATIONS),
    );

    // ---- background ----
    let chosen = text(cfg, "wallpaper", DEFAULT_WALLPAPER);
    let wallpaper = if chosen.is_empty() {
        follow.unwrap_or("screenshot").to_string()
    } else {
        chosen
    };
    let _ = write!(
        out,
        "\n# ---- the screen behind everything ----\n\
         # The desktop's own wallpaper, not a screenshot of the desktop: what should be\n\
         # behind the lock screen is the background you chose, not a blurred photograph of\n\
         # whatever windows were open. `screenshot` (the fallback on a machine with no\n\
         # hyprpaper config) re-takes the live desktop at every lock.\n\
         background {{\n\
         \x20   monitor = {}\n\
         \x20   path = {}\n\
         \x20   color = {}\n\
         \x20   blur_passes = {}\n\
         \x20   blur_size = {}\n\
         \x20   brightness = {}\n\
         \x20   contrast = {}\n\
         \x20   vibrancy = {}\n\
         \x20   vibrancy_darkness = {}\n\
         \x20   noise = {}\n\
         \x20   reload_time = {}\n\
         \x20   crossfade_time = {}\n\
         \x20   zindex = -1\n\
         }}\n",
        text(cfg, "monitor", DEFAULT_MONITOR),
        wallpaper,
        hypr_color(&bg),
        text(cfg, "blur_passes", DEFAULT_BLUR_PASSES),
        text(cfg, "blur_size", DEFAULT_BLUR_SIZE),
        text(cfg, "brightness", DEFAULT_BRIGHTNESS),
        text(cfg, "contrast", DEFAULT_CONTRAST),
        text(cfg, "vibrancy", DEFAULT_VIBRANCY),
        text(cfg, "vibrancy_darkness", DEFAULT_VIBRANCY_DARKNESS),
        text(cfg, "noise", DEFAULT_NOISE),
        text(cfg, "reload_time", DEFAULT_RELOAD_TIME),
        text(cfg, "crossfade_time", DEFAULT_CROSSFADE_TIME),
    );

    // ---- the glass panel ----
    let _ = write!(
        out,
        "\n# ---- the glass panel ----\n\
         # One sheet of dark glass behind the whole arrangement, so the type sits on a known\n\
         # contrast instead of on whatever the wallpaper is doing - the job the OSD cards'\n\
         # `box.card` does, one layer down.\n\
         shape {{\n\
         \x20   monitor = {}\n\
         \x20   size = {}, {}\n\
         \x20   position = {}, {}\n\
         \x20   halign = center\n\
         \x20   valign = center\n\
         \x20   rounding = {}\n\
         \x20   color = {}\n\
         \x20   border_size = {}\n\
         \x20   border_color = {}\n\
         \x20   zindex = -1\n\
         \x20   shadow_color = {}\n\
         \x20   shadow_size = {}\n\
         \x20   shadow_passes = {}\n\
         \x20   shadow_boost = {}\n\
         }}\n",
        text(cfg, "monitor", DEFAULT_MONITOR),
        text(cfg, "panel_width", DEFAULT_PANEL_WIDTH),
        text(cfg, "panel_height", DEFAULT_PANEL_HEIGHT),
        text(cfg, "panel_x", DEFAULT_PANEL_X),
        text(cfg, "panel_y", DEFAULT_PANEL_Y),
        text(cfg, "panel_rounding", DEFAULT_PANEL_ROUNDING),
        with_alpha(&bg, number(cfg, "panel_opacity", DEFAULT_PANEL_OPACITY)),
        text(cfg, "panel_border", DEFAULT_PANEL_BORDER),
        hypr_color(&border),
        black(number(
            cfg,
            "panel_shadow_opacity",
            DEFAULT_PANEL_SHADOW_OPACITY
        )),
        text(cfg, "panel_shadow_size", DEFAULT_PANEL_SHADOW_SIZE),
        text(cfg, "panel_shadow_passes", DEFAULT_PANEL_SHADOW_PASSES),
        text(cfg, "panel_shadow_boost", DEFAULT_PANEL_SHADOW_BOOST),
    );

    // ---- clock and date ----
    let _ = write!(
        out,
        "\n# ---- clock and date ----\n\
         # Positions are relative to the middle of the monitor (x to the right, **y up**:\n\
         # a positive y is above the centre, which is the opposite of what you would guess).\n\
         label {{\n\
         \x20   monitor = {}\n\
         \x20   text = {}\n\
         \x20   font_family = {}\n\
         \x20   font_size = {}\n\
         \x20   color = {}\n\
         \x20   position = {}, {}\n\
         \x20   halign = center\n\
         \x20   valign = center\n\
         \x20   zindex = 1\n\
         \x20   shadow_color = {}\n\
         \x20   shadow_size = 18\n\
         \x20   shadow_passes = 2\n\
         \x20   shadow_boost = 1.1\n\
         }}\n\
         \n\
         label {{\n\
         \x20   monitor = {}\n\
         \x20   text = {}\n\
         \x20   font_family = {}\n\
         \x20   font_size = {}\n\
         \x20   color = {}\n\
         \x20   position = {}, {}\n\
         \x20   halign = center\n\
         \x20   valign = center\n\
         \x20   zindex = 1\n\
         \x20   shadow_color = rgba(00000066)\n\
         \x20   shadow_size = 10\n\
         \x20   shadow_passes = 1\n\
         }}\n",
        text(cfg, "monitor", DEFAULT_MONITOR),
        clock_text(cfg, &accent),
        text(cfg, "clock_font", DEFAULT_CLOCK_FONT),
        text(cfg, "clock_size", DEFAULT_CLOCK_SIZE),
        hypr_color(&fg),
        text(cfg, "clock_x", DEFAULT_CLOCK_X),
        text(cfg, "clock_y", DEFAULT_CLOCK_Y),
        black(0.50),
        text(cfg, "monitor", DEFAULT_MONITOR),
        date_text(cfg),
        text(cfg, "clock_font", DEFAULT_CLOCK_FONT),
        text(cfg, "date_size", DEFAULT_DATE_SIZE),
        hypr_color(&fg_dim),
        text(cfg, "date_x", DEFAULT_DATE_X),
        text(cfg, "date_y", DEFAULT_DATE_Y),
    );

    // ---- the accent rule ----
    if cfg.bool("show_rule", DEFAULT_SHOW_RULE) {
        let _ = write!(
            out,
            "\n# ---- the accent rule ----\n\
             # A short line where the clock and the field meet. Pure decoration, and the only\n\
             # place the accent appears until the field's outline picks it up.\n\
             shape {{\n\
             \x20   monitor = {}\n\
             \x20   size = {}, {}\n\
             \x20   position = {}, {}\n\
             \x20   halign = center\n\
             \x20   valign = center\n\
             \x20   rounding = {}\n\
             \x20   color = {}\n\
             \x20   zindex = 1\n\
             }}\n",
            text(cfg, "monitor", DEFAULT_MONITOR),
            text(cfg, "rule_width", DEFAULT_RULE_WIDTH),
            text(cfg, "rule_height", DEFAULT_RULE_HEIGHT),
            text(cfg, "clock_x", DEFAULT_CLOCK_X),
            text(cfg, "rule_y", DEFAULT_RULE_Y),
            text(cfg, "rule_rounding", DEFAULT_RULE_ROUNDING),
            with_alpha(&accent, number(cfg, "rule_opacity", DEFAULT_RULE_OPACITY)),
        );
    }

    // ---- the lock mark ----
    if cfg.bool("show_lock_icon", DEFAULT_SHOW_LOCK_ICON) {
        let _ = write!(
            out,
            "\n# ---- the lock mark ----\n\
             # hyprlock draws text and cannot read the Lucide drawings the cards wear, so the\n\
             # glyph stays a Nerd Font character - the one place on this desktop that is still\n\
             # a font glyph rather than a drawing.\n\
             label {{\n\
             \x20   monitor = {}\n\
             \x20   text = {}\n\
             \x20   font_family = {}\n\
             \x20   font_size = {}\n\
             \x20   color = {}\n\
             \x20   position = {}, {}\n\
             \x20   halign = center\n\
             \x20   valign = center\n\
             \x20   zindex = 1\n\
             }}\n",
            text(cfg, "monitor", DEFAULT_MONITOR),
            text(cfg, "lock_icon", DEFAULT_LOCK_ICON),
            text(cfg, "clock_font", DEFAULT_CLOCK_FONT),
            text(cfg, "lock_size", DEFAULT_LOCK_SIZE),
            hypr_color(&fg_faint),
            text(cfg, "clock_x", DEFAULT_CLOCK_X),
            text(cfg, "lock_y", DEFAULT_LOCK_Y),
        );
    }

    // ---- the password field ----
    let _ = write!(
        out,
        "\n# ---- the password field ----\n\
         # A taller box, so the dots and the placeholder are legible from arm's length, an\n\
         # accent-tinted glass fill, and the bar's gradient on the outline (the one place on\n\
         # this screen where the accent and the accent2 meet).\n\
         #\n\
         # `fade_on_empty` is what makes the field *appear*: with `fade_timeout = 0` it is\n\
         # invisible while the buffer is empty and fades in with the first keystroke.\n\
         input-field {{\n\
         \x20   monitor = {}\n\
         \x20   size = {}, {}\n\
         \x20   position = {}, {}\n\
         \x20   halign = center\n\
         \x20   valign = center\n\
         \x20   zindex = 1\n\
         \x20   rounding = {}\n\
         \x20   font_family = {}\n\
         \x20   inner_color = {}\n\
         \x20   font_color = {}\n\
         \x20   dots_size = {}\n\
         \x20   dots_spacing = {}\n\
         \x20   dots_center = {}\n\
         \x20   dots_rounding = {}\n\
         \x20   dots_text_format = {}\n\
         \x20   outline_thickness = {}\n\
         \x20   outer_color = {} {}\n\
         \x20   check_color = {}\n\
         \x20   fail_color = {}\n\
         \x20   capslock_color = {}\n\
         \x20   hide_input = {}\n\
         \x20   swap_font_color = {}\n\
         \x20   invert_numlock = {}\n\
         \x20   placeholder_text = {}\n\
         \x20   check_text = {}\n\
         \x20   fail_text = {}\n\
         \x20   fade_on_empty = {}\n\
         \x20   fade_timeout = {}\n\
         \x20   shadow_color = rgba(00000099)\n\
         \x20   shadow_size = 20\n\
         \x20   shadow_passes = 2\n\
         \x20   shadow_boost = 1\n\
         }}\n",
        text(cfg, "monitor", DEFAULT_MONITOR),
        text(cfg, "field_width", DEFAULT_FIELD_WIDTH),
        text(cfg, "field_height", DEFAULT_FIELD_HEIGHT),
        text(cfg, "field_x", DEFAULT_FIELD_X),
        text(cfg, "field_y", DEFAULT_FIELD_Y),
        text(cfg, "field_rounding", DEFAULT_FIELD_ROUNDING),
        text(cfg, "field_font", DEFAULT_FIELD_FONT),
        with_alpha(&accent, number(cfg, "field_tint", DEFAULT_FIELD_TINT)),
        hypr_color(&fg),
        text(cfg, "dots_size", DEFAULT_DOTS_SIZE),
        text(cfg, "dots_spacing", DEFAULT_DOTS_SPACING),
        flag(cfg, "dots_center", DEFAULT_DOTS_CENTER),
        text(cfg, "dots_rounding", DEFAULT_DOTS_ROUNDING),
        text(cfg, "dots_text_format", DEFAULT_DOTS_TEXT_FORMAT),
        text(cfg, "field_outline", DEFAULT_FIELD_OUTLINE),
        with_alpha(
            &accent,
            number(cfg, "field_outline_accent", DEFAULT_FIELD_OUTLINE_ACCENT)
        ),
        with_alpha(
            &accent2,
            number(cfg, "field_outline_accent2", DEFAULT_FIELD_OUTLINE_ACCENT2)
        ),
        hypr_color(&accent),
        hypr_color(&crit),
        hypr_color(&warn),
        flag(cfg, "hide_input", DEFAULT_HIDE_INPUT),
        flag(cfg, "swap_font_color", DEFAULT_SWAP_FONT_COLOR),
        flag(cfg, "invert_numlock", DEFAULT_INVERT_NUMLOCK),
        text(cfg, "placeholder_text", DEFAULT_PLACEHOLDER_TEXT),
        text(cfg, "check_text", DEFAULT_CHECK_TEXT),
        text(cfg, "fail_text", DEFAULT_FAIL_TEXT),
        flag(cfg, "fade_on_empty", DEFAULT_FADE_ON_EMPTY),
        text(cfg, "fade_timeout", DEFAULT_FADE_TIMEOUT),
    );

    // ---- the resting hint ----
    let hint = text(cfg, "hint_text", DEFAULT_HINT_TEXT);
    if !hint.is_empty() {
        let _ = write!(
            out,
            "\n# ---- the resting hint ----\n\
             # The field is invisible while it is empty, so at rest this is what says what to\n\
             # do - and it says it below the field, where the placeholder would have been.\n\
             label {{\n\
             \x20   monitor = {}\n\
             \x20   text = {}\n\
             \x20   font_family = {}\n\
             \x20   font_size = {}\n\
             \x20   color = {}\n\
             \x20   position = {}, {}\n\
             \x20   halign = center\n\
             \x20   valign = center\n\
             \x20   zindex = 1\n\
             }}\n",
            text(cfg, "monitor", DEFAULT_MONITOR),
            hint,
            text(cfg, "clock_font", DEFAULT_CLOCK_FONT),
            text(cfg, "hint_size", DEFAULT_HINT_SIZE),
            hypr_color(&fg_dim),
            text(cfg, "hint_x", DEFAULT_HINT_X),
            text(cfg, "hint_y", DEFAULT_HINT_Y),
        );
    }

    out
}

/// The clock's `text`: a whole generated value, because the colon between hours and
/// minutes is pango markup coloured with the accent - and pango cannot see a
/// hyprlang variable. `##` is hyprlang's escape for the `#` pango needs.
fn clock_text(cfg: &Config, accent: &str) -> String {
    let format = text(cfg, "clock_format", DEFAULT_CLOCK_FORMAT);
    if format == "%H:%M" && cfg.bool("clock_accent_colon", DEFAULT_CLOCK_ACCENT_COLON) {
        let hex = rgb_hex(accent).unwrap_or_else(|| "33ccff".to_string());
        format!(
            "cmd[update:1000] echo \"$(date +'%H')<span foreground='##{hex}'>:</span>$(date +'%M')\""
        )
    } else {
        format!("cmd[update:1000] date +'{format}'")
    }
}

/// The date's `text`: the format, uppercased through `tr` unless the page says
/// otherwise.
fn date_text(cfg: &Config) -> String {
    let format = text(cfg, "date_format", DEFAULT_DATE_FORMAT);
    if cfg.bool("date_uppercase", DEFAULT_DATE_UPPERCASE) {
        format!("cmd[update:60000] echo \"$(date +'{format}' | tr '[:lower:]' '[:upper:]')\"")
    } else {
        format!("cmd[update:60000] date +'{format}'")
    }
}

/// A knob's value, or the default when the file does not set it (an empty value is
/// "not set", like every other element's config).
fn text(cfg: &Config, key: &str, default: &str) -> String {
    cfg.get(key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
        .to_string()
}

/// A number as an `f64`, for the opacity knobs; an unparseable value falls back to
/// the default rather than producing a colour hyprlock refuses.
fn number(cfg: &Config, key: &str, default: &str) -> f64 {
    text(cfg, key, default)
        .parse()
        .unwrap_or_else(|_| default.parse().unwrap_or(1.0))
}

fn flag(cfg: &Config, key: &str, default: bool) -> &'static str {
    if cfg.bool(key, default) {
        "true"
    } else {
        "false"
    }
}

/// Black at an opacity, for the shadows - the one colour that is not a palette
/// token. hyprlock reads `rgba(rrggbbaa)`.
fn black(alpha: f64) -> String {
    format!("rgba(000000{:02x})", alpha_byte(alpha))
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
            3 => format!(
                "rgb({})",
                hex.chars().flat_map(|c| [c, c]).collect::<String>()
            ),
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
            3 => Some((hex.chars().flat_map(|c| [c, c]).collect::<String>(), 1.0)),
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

/// Ask hyprlock's own parser whether the generated file is sound, without
/// locking anything: point it at a display that does not exist, so it reads the
/// config (printing every error with a line number) and only then fails to
/// connect.
///
/// The answer is one sentence for the page's reading row - "no config errors" is
/// the whole point, because hyprlock *ignores* a bad option instead of refusing to
/// run, so a typo would otherwise stay invisible until it mattered.
pub fn check() -> String {
    let path = conf_path();
    let modified = match std::fs::metadata(&path).and_then(|meta| meta.modified()) {
        Ok(modified) => modified,
        Err(_) => return "not written yet".to_string(),
    };
    // The page is drawn again after every change, and the parser is the only
    // thing that can answer - so an unchanged file reuses the last answer. The
    // generated file only moves when a knob or the theme does.
    use std::sync::Mutex;
    static LAST: Mutex<Option<(std::time::SystemTime, String)>> = Mutex::new(None);
    if let Ok(cache) = LAST.lock() {
        if let Some((when, answer)) = &*cache {
            if *when == modified {
                return answer.clone();
            }
        }
    }
    let answer = run_check(&path);
    if let Ok(mut cache) = LAST.lock() {
        *cache = Some((modified, answer.clone()));
    }
    answer
}

/// The check itself, with the core limit turned off: hyprlock aborts when it
/// cannot reach the display it was pointed at, which is exactly how the parser is
/// made to run without locking anything - but an abort under this session's
/// unlimited core limit would leave a ~550 KB coredump behind on every page
/// render (measured: `coredumpctl list hyprlock` had one per check).
fn run_check(path: &std::path::Path) -> String {
    if !crate::hardware::installed("hyprlock") {
        return "hyprlock is not installed".to_string();
    }
    match std::process::Command::new("sh")
        .arg("-c")
        .arg("ulimit -c 0; exec hyprlock --config \"$1\" -v")
        .arg("sh")
        .arg(path)
        .env("WAYLAND_DISPLAY", "hypr-osd-check")
        .output()
    {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stderr);
            let errors = text
                .lines()
                .filter(|line| line.contains("Config error"))
                .count();
            if errors == 0 {
                "no config errors".to_string()
            } else {
                format!(
                    "{errors} config error{} - `hyprlock --config {} -v` names the lines",
                    if errors == 1 { "" } else { "s" },
                    path.display()
                )
            }
        }
        Err(error) => format!("could not run hyprlock: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Kind;

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

    /// A config file of its own per call: the tests run in parallel, and a shared
    /// name would have one test reading another's file.
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

    fn doc(text: &str) -> String {
        document_from(&config(text), &palette(), Some("/home/lars/x.jpg"))
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

    /// The cut colours keep the exact literals the hand-written file used, which is
    /// what makes the generated file a drop-in replacement.
    #[test]
    fn the_cut_colours_are_the_ones_the_file_had() {
        let bg = token(&palette(), "bg");
        let accent = token(&palette(), "accent");
        let accent2 = token(&palette(), "accent2");
        assert_eq!(with_alpha(&bg, 0.65), "rgba(0b0f15a6)");
        assert_eq!(with_alpha(&accent, 0.60), "rgba(33ccff99)");
        assert_eq!(with_alpha(&accent, 0.08), "rgba(33ccff14)");
        assert_eq!(
            format!(
                "{} {}",
                with_alpha(&accent, 0.40),
                with_alpha(&accent2, 0.20)
            ),
            "rgba(33ccff66) rgba(00ff9933)"
        );
    }

    /// An untouched `lock.conf` produces the screen `configs/hyprlock.conf` was
    /// tuned to - the numbers and the palette, not just a file that parses.
    #[test]
    fn the_defaults_are_the_tuned_ones() {
        let text = doc("");
        assert!(text.contains("blur_size = 1\n"), "{text}");
        assert!(text.contains("brightness = 0.70\n"), "{text}");
        assert!(text.contains("size = 800, 670\n"), "{text}");
        assert!(text.contains("hide_cursor = true\n"), "{text}");
        assert!(text.contains("fail_timeout = 2000\n"), "{text}");
        assert!(text.contains("color = rgb(dde7ef)\n"), "{text}");
        assert!(text.contains("path = /home/lars/x.jpg\n"), "{text}");
    }

    /// A key the file sets wins over the default, and an empty value is "unset".
    #[test]
    fn the_file_wins_over_the_defaults() {
        let text = doc("blur_size = 12\nhide_cursor = false\nbrightness =\n");
        assert!(text.contains("blur_size = 12\n"), "{text}");
        assert!(text.contains("hide_cursor = false\n"), "{text}");
        assert!(text.contains("brightness = 0.70\n"), "{text}");
    }

    /// The clock's colon is markup, so it is a whole value - and it carries the
    /// accent, which is the point of generating it rather than writing it out.
    #[test]
    fn the_clock_text_carries_the_accent() {
        let text = doc("");
        assert!(
            text.contains(
                "text = cmd[update:1000] echo \"$(date +'%H')<span foreground='##33ccff'>:</span>$(date +'%M')\""
            ),
            "{text}"
        );
        // A format of its own is handed to `date` unchanged, and the accent colon
        // is not invented for it.
        let plain = doc("clock_format = %H.%M\n");
        assert!(
            plain.contains("text = cmd[update:1000] date +'%H.%M'\n"),
            "{plain}"
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

        let chosen = doc("wallpaper = /tmp/mine.jpg\n");
        assert!(chosen.contains("path = /tmp/mine.jpg\n"), "{chosen}");

        let none = document_from(&config(""), &palette(), None);
        assert!(none.contains("path = screenshot\n"), "{none}");
    }

    /// The settings page and this module name each other: every row the Lock
    /// screen page draws changes the generated file, so a switch cannot sit on the
    /// page changing nothing.
    #[test]
    fn the_lock_page_carries_every_knob() {
        let base = doc("");
        for setting in crate::settings::LOCK {
            let sample = sample_for(setting);
            let changed = doc(&format!("{} = {}\n", setting.key, sample));
            assert_ne!(
                changed, base,
                "the Lock screen page draws `{}` but the generated file ignores it",
                setting.key
            );
        }
    }

    /// A value that is certainly not the default, per kind.
    fn sample_for(setting: &crate::settings::Setting) -> String {
        match setting.kind {
            Kind::Bool => {
                if setting.default == "true" {
                    "false".into()
                } else {
                    "true".into()
                }
            }
            Kind::Int { .. } => {
                let value: i32 = setting.default.parse().unwrap_or(0);
                format!("{}", if value == -1 { 7 } else { value + 1 })
            }
            Kind::Fraction { .. } => {
                let value: f64 = setting.default.parse().unwrap_or(0.0);
                format!("{:.2}", value + 0.13)
            }
            Kind::Choice(options) => options
                .iter()
                .find(|option| **option != setting.default)
                .map(|option| option.to_string())
                .unwrap_or_else(|| setting.default.to_string()),
            Kind::Text
            | Kind::Path
            | Kind::PathFolder
            | Kind::Font
            | Kind::Keymap(_)
            | Kind::Monitor(_) => format!("{}x", setting.default),
            Kind::Color => "#123456".to_string(),
            Kind::Info | Kind::Actions(_) => panic!("a reading or a button is not a knob"),
        }
    }
}

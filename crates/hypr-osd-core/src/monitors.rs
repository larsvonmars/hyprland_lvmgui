//! Which output an OSD belongs on.
//!
//! A layer-shell surface can be pinned to one monitor, but GDK has no notion of
//! "the focused monitor" - that is a compositor concept. Hyprland exposes it as
//! the monitor of the active workspace, i.e. the output whose windows are
//! focused, which is exactly where the user pressed the volume key.

use gtk::gdk;
use gtk::prelude::*;
use serde_json::Value;

use crate::hyprctl;

/// An output, in the terms an element needs it: how big it is to draw on, and
/// how many physical pixels make one of those.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Output {
    /// Width in *layout* pixels - the pixels GTK lays widgets out in, which is
    /// the output's resolution divided by its scale factor.
    pub width: i32,
    /// Height in layout pixels.
    pub height: i32,
    /// Physical pixels per layout pixel: `2.0` on a HiDPI screen.
    pub scale: f64,
}

/// Every monitor GDK knows about, in the compositor's order.
pub fn all() -> Vec<gdk::Monitor> {
    let Some(display) = gdk::Display::default() else {
        return Vec::new();
    };
    let monitors = display.monitors();
    (0..monitors.n_items())
        .filter_map(|index| monitors.item(index).and_downcast::<gdk::Monitor>())
        .collect()
}

/// The connector name (`eDP-1`, `DP-2`, ...) of the monitor Hyprland considers
/// focused, or `None` when hyprctl cannot be asked - a single-monitor session
/// never needs this to work.
pub fn focused_connector() -> Option<String> {
    let workspace = hyprctl::json(&["activeworkspace"])?;
    workspace.get("monitor")?.as_str().map(str::to_owned)
}

/// The GDK monitor with this connector name.
pub fn find(connector: &str) -> Option<gdk::Monitor> {
    all()
        .into_iter()
        .find(|monitor| monitor.connector().as_deref() == Some(connector))
}

/// The output the focused workspace lives on.
///
/// Hyprland reports a monitor in physical pixels and, separately, how many of
/// them make one logical one. A card is drawn in logical pixels, so an element
/// that has to fill the screen - the overview, placing tiles to fit - needs the
/// division done, and needs the scale factor itself to know how much detail a
/// capture of a window is worth asking for.
pub fn focused_output() -> Option<Output> {
    let connector = focused_connector()?;
    let monitors = hyprctl::json(&["monitors"])?;
    let monitor = monitors
        .as_array()?
        .iter()
        .find(|monitor| monitor.get("name").and_then(Value::as_str) == Some(connector.as_str()))?;
    output_of(monitor)
}

/// The arithmetic, split out from [`focused_output`] so it can be tested against
/// a monitor exactly as Hyprland writes it.
fn output_of(monitor: &Value) -> Option<Output> {
    let number = |key: &str| monitor.get(key).and_then(Value::as_i64);
    let width = number("width")?;
    let height = number("height")?;
    // A monitor always has a scale factor, but a zero or a negative one would
    // turn into a division by zero or a negative size below.
    let scale = monitor
        .get("scale")
        .and_then(Value::as_f64)
        .filter(|scale| *scale > 0.0)
        .unwrap_or(1.0);
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(Output {
        width: (width as f64 / scale).round() as i32,
        height: (height as f64 / scale).round() as i32,
        scale,
    })
}

/// The first monitor, used when the focused one cannot be identified.
pub fn first() -> Option<gdk::Monitor> {
    all().into_iter().next()
}

/// The connector the settings window's Display page designated as the **primary
/// display**, or `None` when the collection should go on following the focused
/// output.
///
/// The value is the `primary` key of `~/.config/hypr-osd/desktop.conf` (see
/// [`crate::settings::PRIMARY_DISPLAY`]), which the Display page writes - a
/// setting of *this collection* rather than of Hyprland, because the compositor
/// has no notion of a primary output at all, and that is exactly why the cards
/// need one. A value that is not a connector, or one that is not plugged in right
/// now, is treated as "no choice" rather than pulling a card onto a screen that is
/// not there.
pub fn primary_connector() -> Option<String> {
    let config = crate::config::Config::load(crate::desktop::FILE);
    let name = primary_name(config.get(crate::settings::PRIMARY_DISPLAY))?;
    find(&name).is_some().then_some(name)
}

/// The reading of the `primary` key as a connector: `None` for "no opinion" - an
/// absent key, [`PRIMARY_FOCUSED`](crate::settings::PRIMARY_FOCUSED), or an empty
/// value.
///
/// Split out from [`primary_connector`] so the parsing can be tested without a
/// display: whether the connector it answers exists is GTK's to say, and that half
/// only runs in a session.
fn primary_name(value: Option<&str>) -> Option<String> {
    let name = value?.trim();
    if name.is_empty() || name == crate::settings::PRIMARY_FOCUSED {
        return None;
    }
    Some(name.to_string())
}

/// One output as the settings window's Display page needs it: the connector, what
/// it is doing *now*, and the modes it reports.
///
/// This is the compositor's own state (`hyprctl -j monitors`), not a config file:
/// the Display page shows a monitor's current mode/position/scale as the value of
/// a field nobody has set, because that is what is in force, and it offers the
/// modes the monitor itself lists. The mode/position strings are spelled the way
/// `hl.monitor` takes them, so they can be written into a rule unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    pub name: String,
    /// Physical pixels, as Hyprland reports them (a 2x screen is 2736 wide here).
    pub width: i32,
    pub height: i32,
    pub refresh: f64,
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    pub transform: i32,
    /// The output this one mirrors, or `none`.
    pub mirror: String,
    pub disabled: bool,
    /// Whether this is the output the focused workspace is on.
    pub focused: bool,
    /// Every mode the monitor reports, without the `Hz` (`2560x1440@74.97`).
    pub modes: Vec<String>,
}

impl Display {
    /// The mode it is running now (`2736x1824@59.96`).
    pub fn mode(&self) -> String {
        format!("{}x{}@{:.2}", self.width, self.height, self.refresh)
    }

    /// Where it sits now (`1903x1440`).
    pub fn position(&self) -> String {
        format!("{}x{}", self.x, self.y)
    }

    /// How much of the *layout* this output takes: physical pixels divided by the
    /// scale factor, which is the unit a monitor's position is measured in (and
    /// the one GTK draws a card in). A 90° or 270° rotation swaps the two, because
    /// that is how the monitor then sits on the desk.
    ///
    /// This is what makes a picture of the outputs honest: a 2x panel is twice as
    /// many pixels but *not* twice as wide in the layout, so drawing a 2736x1824
    /// screen at 2x as wide as a 2560x1440 one at 1x would be a lie about the
    /// desktop the user is arranging.
    pub fn logical_size(&self) -> (i32, i32) {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        let width = (self.width as f64 / scale).round() as i32;
        let height = (self.height as f64 / scale).round() as i32;
        // Hyprland's transforms: 1, 3, 5 and 7 are the ones that add a quarter
        // turn (5 and 7 are their flipped versions), 0/2/4/6 are upright.
        if matches!(self.transform, 1 | 3 | 5 | 7) {
            (height, width)
        } else {
            (width, height)
        }
    }

    /// One line about what this output is doing now (`2560 × 1440 · 59.95 Hz`),
    /// for the heading the Display page draws over its rows.
    pub fn summary(&self) -> String {
        format!(
            "{} \u{d7} {} \u{b7} {:.2} Hz",
            self.width, self.height, self.refresh
        )
    }

    /// The value of one Display-page field as it stands now, or `None` for a name
    /// that is not a field of a monitor rule.
    pub fn value(&self, field: &str) -> Option<String> {
        Some(match field {
            "mode" => self.mode(),
            "position" => self.position(),
            "scale" => format!("{}", self.scale),
            "transform" => self.transform.to_string(),
            "mirror" => self.mirror.clone(),
            "disabled" => self.disabled.to_string(),
            _ => return None,
        })
    }
}

/// Every output the compositor knows about, in its own order.
///
/// An empty list means `hyprctl` could not be asked - which is also what a session
/// that is not Hyprland answers - and the Display page then has no rows to draw.
pub fn list() -> Vec<Display> {
    let Some(monitors) = hyprctl::json(&["monitors"]) else {
        return Vec::new();
    };
    let Some(array) = monitors.as_array() else {
        return Vec::new();
    };
    array.iter().filter_map(display_of).collect()
}

/// One monitor out of `hyprctl -j monitors`, as the Display page needs it.
fn display_of(monitor: &Value) -> Option<Display> {
    let name = monitor.get("name").and_then(Value::as_str)?.to_string();
    let int = |key: &str| monitor.get(key).and_then(Value::as_i64).unwrap_or(0);
    let float = |key: &str| monitor.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let modes = monitor
        .get("availableModes")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                // Hyprland lists them as `2560x1440@59.95Hz`; the `Hz` is not part
                // of the mode a rule names.
                .map(|mode| mode.trim_end_matches("Hz").to_string())
                .collect()
        })
        .unwrap_or_default();
    Some(Display {
        name,
        width: int("width") as i32,
        height: int("height") as i32,
        refresh: float("refreshRate"),
        x: int("x") as i32,
        y: int("y") as i32,
        scale: monitor.get("scale").and_then(Value::as_f64).unwrap_or(1.0),
        transform: int("transform") as i32,
        mirror: monitor
            .get("mirrorOf")
            .and_then(Value::as_str)
            .unwrap_or("none")
            .to_string(),
        disabled: monitor
            .get("disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        focused: monitor
            .get("focused")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        modes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape `hyprctl -j monitors` really answers with: a 2736x1824 panel
    /// that calls itself 2x is 1368x912 as far as GTK is concerned.
    #[test]
    fn an_output_is_measured_in_the_pixels_gtk_draws_in() {
        let monitor = serde_json::json!({
            "name": "eDP-1",
            "width": 2736,
            "height": 1824,
            "scale": 2.0,
        });
        assert_eq!(
            output_of(&monitor),
            Some(Output {
                width: 1368,
                height: 912,
                scale: 2.0,
            })
        );
    }

    #[test]
    fn a_monitor_with_no_usable_size_is_no_output() {
        assert_eq!(
            output_of(&serde_json::json!({ "width": 0, "height": 0 })),
            None
        );
        assert_eq!(output_of(&serde_json::json!({ "scale": 1.0 })), None);
        // A missing scale factor is a 1x screen, not a zero.
        let plain = serde_json::json!({ "width": 1920, "height": 1080 });
        assert_eq!(output_of(&plain).map(|output| output.scale), Some(1.0));
        assert_eq!(output_of(&plain).map(|output| output.width), Some(1920));
    }

    /// The Display page's rows come out of `hyprctl -j monitors`, and the modes it
    /// offers are the monitor's own - with the `Hz` stripped, because that is the
    /// spelling `hl.monitor` takes.
    #[test]
    fn a_display_reports_what_the_monitor_is_doing() {
        let monitor = serde_json::json!({
            "name": "eDP-1",
            "width": 2736,
            "height": 1824,
            "refreshRate": 59.959,
            "x": 1903,
            "y": 1440,
            "scale": 2,
            "transform": 0,
            "mirrorOf": "none",
            "disabled": false,
            "focused": true,
            "availableModes": ["2736x1824@59.96Hz", "1368x912@60.00Hz"],
        });
        let display = display_of(&monitor).unwrap();
        assert_eq!(display.name, "eDP-1");
        assert_eq!(display.mode(), "2736x1824@59.96");
        assert_eq!(display.position(), "1903x1440");
        assert_eq!(display.summary(), "2736 \u{d7} 1824 \u{b7} 59.96 Hz");
        assert!(display.focused);
        assert_eq!(display.value("scale").as_deref(), Some("2"));
        assert_eq!(display.value("transform").as_deref(), Some("0"));
        assert_eq!(display.value("mirror").as_deref(), Some("none"));
        assert_eq!(display.value("disabled").as_deref(), Some("false"));
        assert_eq!(display.value("nonsense"), None);
        // The picture of the layout is drawn in *logical* pixels: a 2x panel is
        // 1368x912 there, not 2736x1824.
        assert_eq!(display.logical_size(), (1368, 912));
        assert_eq!(
            display.modes,
            vec!["2736x1824@59.96".to_string(), "1368x912@60.00".to_string()]
        );
    }

    /// A quarter-turned output takes the other shape in the layout, because that
    /// is how it sits on the desk.
    #[test]
    fn a_quarter_turned_output_swaps_its_layout_size() {
        let monitor = serde_json::json!({
            "name": "DP-2",
            "width": 1920,
            "height": 1080,
            "scale": 1,
            "transform": 3,
        });
        let display = display_of(&monitor).unwrap();
        assert_eq!(display.logical_size(), (1080, 1920));
    }

    /// A monitor with no `availableModes` (or no name) is not a row: the Display
    /// page would have nothing to offer for it.
    #[test]
    fn a_nameless_monitor_is_no_display() {
        assert_eq!(display_of(&serde_json::json!({ "width": 1920 })), None);
        let bare = display_of(&serde_json::json!({ "name": "DP-9" })).unwrap();
        assert!(bare.modes.is_empty());
        assert_eq!(bare.scale, 1.0);
        // A monitor nobody has focused is not marked as the focused one.
        assert!(!bare.focused);
    }

    /// The primary display is a connector, and everything else - an absent key, an
    /// empty value, the word for "follow focus" - means no choice at all.
    #[test]
    fn the_primary_display_is_a_connector_or_nothing() {
        assert_eq!(primary_name(Some("eDP-1")), Some("eDP-1".to_string()));
        // A file's value is trimmed, like every other text setting's.
        assert_eq!(primary_name(Some("  DP-1 ")), Some("DP-1".to_string()));
        assert_eq!(primary_name(None), None);
        assert_eq!(primary_name(Some("")), None);
        assert_eq!(primary_name(Some("focused")), None);
    }
}

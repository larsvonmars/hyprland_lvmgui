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
}

//! Which output an OSD belongs on.
//!
//! A layer-shell surface can be pinned to one monitor, but GDK has no notion of
//! "the focused monitor" - that is a compositor concept. Hyprland exposes it as
//! the monitor of the active workspace, i.e. the output whose windows are
//! focused, which is exactly where the user pressed the volume key.

use gtk::gdk;
use gtk::prelude::*;

use crate::hyprctl;

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

/// The first monitor, used when the focused one cannot be identified.
pub fn first() -> Option<gdk::Monitor> {
    all().into_iter().next()
}

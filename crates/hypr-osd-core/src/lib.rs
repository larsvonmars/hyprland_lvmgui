//! Shared plumbing for the Hyprland UI elements in this collection.
//!
//! Every element is a small GTK4 app that owns exactly one borderless
//! layer-shell surface: a transparent frame with a single *card* in it, drawn
//! in the bar's design language. What lives here is everything the elements
//! would otherwise each have to invent:
//!
//! * [`css`] - the palette (read from waybar's stylesheet) plus the card
//!   recipe and typography, in GTK4 CSS.
//! * [`monitors`] - which output the OSD should appear on (the focused one).
//! * [`follow`] - run a command that prints one line per event and hand those
//!   lines to the main loop, for elements that react to something happening
//!   (a new track, a device appearing) instead of to a key press.
//! * [`hyprctl`] - asking Hyprland itself (the focused output, the window list,
//!   a dispatch) the same way audio goes through `wpctl`.
//! * [`Osd`] - the layer surface, its card and the auto-hide timer.
//! * [`run`] - the application shell: GTK4 + `GApplication`, single instance,
//!   command line forwarding, so `element up` in a keybinding talks to the
//!   already-running instance and never opens a second window.
//! * [`Config`] - the tiny `~/.config/hypr-osd/<element>.conf` reader.
//!
//! Conventions an element should follow are in the README ("Adding an
//! element"): one binary, one app id under `com.schells2.osd.*`, one namespace
//! prefix `hypr-osd`, verbs on the command line.

pub mod config;
pub mod css;
pub mod follow;
pub mod hyprctl;
pub mod monitors;
mod osd;

pub use config::Config;
pub use osd::{
    run, Build, Handle, Keyboard, Opts, Osd, Placement, CARD_PAD_X, CARD_PAD_Y, SHADOW_PAD,
};

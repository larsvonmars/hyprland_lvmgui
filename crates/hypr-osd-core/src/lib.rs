//! Shared plumbing for the Hyprland UI elements in this collection.
//!
//! Every element is a small GTK4 app that owns exactly one borderless
//! layer-shell surface: a transparent frame with a single *card* in it, drawn
//! in the bar's design language. What lives here is everything the elements
//! would otherwise each have to invent:
//!
//! * [`css`] - the palette (read from `~/.config/hypr-osd/theme.css`) plus the
//!   card, bar and typography recipes, in GTK4 CSS.
//! * [`monitors`] - which output the OSD should appear on (the focused one), and
//!   how big it is in the pixels GTK draws in.
//! * [`follow`] - run a command that prints one line per event and hand those
//!   lines to the main loop, for elements that react to something happening
//!   (a new track, a device appearing) instead of to a key press.
//! * [`output`] - the same for a command that answers *once* with bytes: how the
//!   overview captures windows without freezing while it does.
//! * [`hypripc`] - Hyprland's own sockets: a request/answer socket and the event
//!   stream, for the elements that have to keep up with the compositor (the bar,
//!   most of all) instead of asking it one question at a time.
//! * [`hardware`] - the machine's own state, as far as a pill needs it: the sink's
//!   volume, the battery, the wireless link.
//! * [`system`] - the same idea for the machine's *load*: CPU, memory,
//!   temperature and pending updates, shared by the bar's system pill and the
//!   system popup.
//! * [`mpris`] - what is playing, through `playerctl`: the shared half of the
//!   media card, the bar's media pill and the island popup.
//! * [`windows`] - Hyprland's windows and workspaces, as far as a card needs
//!   them, shared by the two elements that draw windows.
//! * [`icons`] - a window's class turned into the application's icon.
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
pub mod hardware;
pub mod hover;
pub mod hyprctl;
pub mod hypripc;
pub mod icons;
pub mod monitors;
pub mod mpris;
mod osd;
pub mod output;
pub mod state;
pub mod system;
pub mod text;
pub mod windows;

pub use config::Config;
pub use osd::{
    run, Build, Content, Handle, Keyboard, Opts, Osd, Placement, CARD_PAD_X, CARD_PAD_Y, SHADOW_PAD,
};

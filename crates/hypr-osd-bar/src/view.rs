//! The bar itself: one row, three sections, and the pills in them.
//!
//! Left: the button that opens the applications panel, then the workspaces, then
//! the focused window's title. Centre: the player - the cover and the track, the
//! one thing in the bar that is not about this machine, and the handle for the
//! island popup (hover it and the panel unfolds; see the `hypr-osd-island`
//! element). Right: the tray and the windows put away, the combined status pill
//! (link, sound, battery), the power button - and at the far end the clock, which
//! is a reading and nothing more.
//!
//! The view owns no state of its own beyond what the last render put on screen.
//! Everything it shows is handed to it by `main` (a snapshot from Hyprland, a
//! reading from `sources`, a track from MPRIS), so there is exactly one place
//! that knows how to draw each pill, and one place that decides *when* to read
//! the thing it draws.
//!
//! The one exception is the workspace row: its accent capsule remembers where it
//! was, because switching workspaces is a *movement* between two paints - the
//! capsule glides from the pill you were on to the one you went to (see
//! [`WorkspaceRow`]). Nothing else on the bar carries state from one paint to the
//! next.
//!
//! A finger changes two things here, both in the handlers rather than in the
//! paint: a *tap* on one of the three handles (the applications button, the
//! player, the status pill) asks its panel to open pinned - `tap` instead of
//! `open`, because there is no pointer to leave and so no hover to close it -
//! and the pointer's arrival no longer asks at all on a tablet, because GDK
//! turns a tap into an emulated pointer that enters a pill and never leaves it.
//! The pills' marks are drawn a few pixels larger while the machine is a tablet;
//! the class that grows the rest is the shell's (`window.osd.touch`).
//!
//! A `gtk::CenterBox` is what keeps the player in the middle: its centre child is
//! centred on the *bar*, not on the space left over between the two sides, which
//! is what an ordinary box with three children would give.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use hypr_osd_core::art;
use hypr_osd_core::hardware::{self as sources, Battery, Charging, Network, Volume};
use hypr_osd_core::icons::{self, names};
use hypr_osd_core::mpris::Track;
use hypr_osd_core::text;
use hypr_osd_core::touch;
use hypr_osd_core::windows::Window;

use crate::hypr::{Snapshot, Workspace};
use crate::minimise::{self, Minimised};
use crate::{Settings, Shows, TOUCH_ICON_EXTRA};

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

/// How large a pill's marks are drawn.
///
/// The bar is the tightest place in the collection, so most of them sit inside
/// 13px text: the workspace row is a step down from that (its dots live in a
/// capsule beside 12px numbers), the applications button and the power button
/// at the two ends a step up, because those two are aimed at rather than read.
/// A tablet draws every one of them [`TOUCH_ICON_EXTRA`] larger - the pill is
/// still found by the same gesture, but it is now looked at from further away
/// and hit with a fingertip.
const ICON: i32 = 14;
const WS_ICON: i32 = 11;
const APPS_ICON: i32 = 15;
const POWER_ICON: i32 = 16;

/// The cover in the player pill is a picture, not a mark: two steps up from the
/// text beside it, which is what makes it read as artwork rather than an icon.
const MEDIA_ART: i32 = 22;
/// The play/pause mark at the *end* of the player pill - smaller than a pill's
/// own mark, because it is a note about the track rather than a reading of the
/// machine.
const MEDIA_HINT: i32 = 12;

/// What separates the parts of a pill's face. A word of its own rather than a
/// character inside one of the readings, so it reads as a separator and not as
/// another number; Pango's `<small>` is what keeps it quiet.
const SEPARATOR: &str = "<small>·</small>";

/// Battery, empty to full, and the charging mark - the one state that is not a
/// level. Four steps where the font had five: Lucide draws four batteries, and
/// the top two of the old ladder were two pixels apart anyway.
const BATTERY_STEPS: [&str; 4] = [
    names::BATTERY_EMPTY,
    names::BATTERY_LOW,
    names::BATTERY_MEDIUM,
    names::BATTERY_FULL,
];

/// Signal strength, weak to strong - the wifi arcs, not Lucide's `signal-*` bar
/// chart: that family puts its ink in the lower left of the box, which at the
/// size a pill draws it is a dot in a corner (measured - see the `icon-sheet`
/// example in `hypr-osd-core`).
const NETWORK_STEPS: [&str; 4] = [
    names::WIFI_ZERO,
    names::WIFI_LOW,
    names::WIFI_HIGH,
    names::WIFI,
];

/// A player's own mark, the way the bar this replaces chose one.
///
/// It is the *kind* of thing a player plays rather than the player itself, and
/// that is forced by the icon set rather than chosen here: Lucide dropped brand
/// marks on purpose, so Firefox cannot be Firefox. A browser is a compass and a
/// browser on Chromium a globe, a video player a clapperboard, a streamer the
/// audio lines of one - and unknown players get the note, which is at least
/// true.
fn player_icon(player: &str) -> &'static str {
    match player.to_ascii_lowercase().as_str() {
        "vlc" => names::PLAY,
        "mpv" | "celluloid" => names::CLAPPERBOARD,
        "spotify" => names::AUDIO_LINES,
        "firefox" | "firefoxdeveloperedition" => names::COMPASS,
        "chromium" | "chrome" | "google-chrome" | "brave" => names::GLOBE,
        _ => names::MUSIC,
    }
}

// ---------------------------------------------------------------------------
// Pills
// ---------------------------------------------------------------------------

/// One part of a pill's face: a mark and the reading it belongs to.
///
/// A pill used to be a single label with the glyph baked into its text, which no
/// longer works: GTK *paints* an icon, it cannot parse one out of a string. So a
/// face is built from parts instead, and the parts are what a repaint compares.
struct Part {
    /// The mark that says what the reading is.
    icon: &'static str,
    /// The reading, in Pango markup, because the one number that is a *task*
    /// rather than a reading (the pending updates) is bold. Empty for a part
    /// that is only a mark - and then no label is made at all, since an empty
    /// `GtkLabel` still draws a stray mark of its own.
    text: String,
}

impl Part {
    fn new(icon: &'static str, text: impl Into<String>) -> Self {
        Part {
            icon,
            text: text.into(),
        }
    }
}

/// One pill: a flat button with an icon and its readings in it.
///
/// A button rather than a plain label so that hover, click and the
/// press-and-hold state come from GTK (and from the stylesheet's `:hover`),
/// instead of being re-invented with gesture controllers; `can_focus` is off
/// because the bar has no keyboard to walk a focus chain with.
struct Pill {
    button: gtk::Button,
    /// The icon-and-reading row inside the button.
    face: gtk::Box,
    /// What is on the face now, so a repaint that changes nothing does not make
    /// GTK rebuild and re-measure it. The markup and the icons beside it mean the
    /// face cannot be asked: `label.text()` is the *parsed* text, not what was
    /// set, and a box has no text at all.
    shown: RefCell<String>,
    /// How large this pill draws its marks.
    icon_size: i32,
}

impl Pill {
    fn new(class: &str, icon_size: i32) -> Self {
        let button = gtk::Button::new();
        button.add_css_class("module");
        button.add_css_class(class);
        button.set_has_frame(false);
        button.set_focus_on_click(false);
        button.set_can_focus(false);
        let face = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        face.add_css_class("face");
        // A pill is often wider than its face: `min-width` gives the workspace
        // pills one width whether they hold a dot, a ring or a number. A box
        // packs its children from the start, so without this the leftover room
        // would all land on the right and the mark would sit left of the
        // capsule's centre. Centring the face itself is what the padding
        // promises; for a pill whose face is already its natural width this
        // changes nothing.
        face.set_halign(gtk::Align::Center);
        face.set_valign(gtk::Align::Center);
        button.set_child(Some(&face));
        Pill {
            button,
            face,
            shown: RefCell::new(String::new()),
            icon_size,
        }
    }

    /// A left click. This is the action that matters, so it gets the plain
    /// "clicked" signal and works with the keyboard/accessibility story too.
    fn on_click(&self, action: impl Fn() + 'static) -> &Self {
        self.button.connect_clicked(move |_| action());
        self
    }

    /// Any other mouse button. GTK's `clicked` is button 1 only, so the rest
    /// arrive as gestures.
    fn on_button(&self, button: u32, action: impl Fn() + 'static) -> &Self {
        let gesture = gtk::GestureClick::new();
        gesture.set_button(button);
        gesture.connect_pressed(move |_, _, _, _| action());
        self.button.add_controller(gesture);
        self
    }

    /// The scroll wheel: `+1` up, `-1` down.
    fn on_scroll(&self, action: impl Fn(i32) + 'static) -> &Self {
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll(move |_, _dx, dy| {
            // Only the direction is a wheel: the amount is how much the desktop
            // scrolled, which for a discrete wheel is 1 or -1 and for a touchpad
            // is a fraction of that.
            action(if dy < 0.0 { 1 } else { -1 });
            glib::Propagation::Stop
        });
        self.button.add_controller(scroll);
        self
    }

    /// Show a different face without rebuilding the button.
    ///
    /// One part per reading, each with the mark that says what the reading is,
    /// and the bar's separator between them.
    fn set(&self, parts: &[Part], states: &[(&str, bool)]) {
        self.paint(parts, states);
    }

    /// The same, for a pill whose whole face is one part.
    fn set_one(&self, icon: &'static str, text: &str, states: &[(&str, bool)]) {
        self.set(&[Part::new(icon, text)], states);
    }

    /// A face that is *only* a mark: the two buttons at the ends of the bar
    /// (the applications panel and the session card) have no reading to show.
    fn set_icon(&self, icon: &'static str) {
        self.set(&[Part::new(icon, "")], &[]);
    }

    fn paint(&self, parts: &[Part], states: &[(&str, bool)]) {
        // The signature is what the face *shows* - the marks and the readings -
        // so a repaint that changes nothing (the volume ticking over every
        // second, the same workspaces arriving again) leaves the widgets alone.
        let signature = parts
            .iter()
            .map(|part| format!("{}|{}", part.icon, part.text))
            .collect::<Vec<_>>()
            .join("\u{1f}");
        {
            let mut shown = self.shown.borrow_mut();
            if *shown != signature {
                *shown = signature;
                while let Some(child) = self.face.first_child() {
                    self.face.remove(&child);
                }
                for (index, part) in parts.iter().enumerate() {
                    if index > 0 {
                        let separator = gtk::Label::new(None);
                        separator.set_markup(SEPARATOR);
                        separator.add_css_class("separator");
                        self.face.append(&separator);
                    }
                    // An empty name means the part is its reading alone: the
                    // active workspace pill is a number with no dot in front of
                    // it, and `lucide("")` would ask the icon theme for nothing.
                    if !part.icon.is_empty() {
                        self.face.append(&icons::lucide(part.icon, self.icon_size));
                    }
                    if !part.text.is_empty() {
                        let label = gtk::Label::new(None);
                        label.set_markup(&part.text);
                        self.face.append(&label);
                    }
                }
            }
        }
        for (class, on) in states {
            set_class(&self.button, class, *on);
        }
    }

    fn tooltip(&self, text: Option<&str>) {
        self.button.set_tooltip_text(text);
    }

    fn set_visible(&self, visible: bool) {
        self.button.set_visible(visible);
    }
}

// ---------------------------------------------------------------------------
// The clock
// ---------------------------------------------------------------------------

/// The clock at the right end of the bar: the time over the small date.
///
/// It is not a [`Pill`]: a pill's face is a mark and a reading side by side, and
/// this is two strings *stacked* - which is the point (the rice this bar follows
/// closes on the time with the date under it, not on another capsule). The
/// button around them is still a `.module`, so it hovers and clicks like every
/// other target.
struct Clock {
    button: gtk::Button,
    time: gtk::Label,
    date: gtk::Label,
}

impl Clock {
    fn new() -> Self {
        let button = gtk::Button::new();
        button.add_css_class("module");
        button.add_css_class("clock");
        button.set_has_frame(false);
        button.set_focus_on_click(false);
        button.set_can_focus(false);

        // A vertical box rather than one label with a line break: the two lines
        // carry different sizes and weights (see `bar.css`), which a single label
        // cannot do - markup would set the whole stack to the same style.
        let face = gtk::Box::new(gtk::Orientation::Vertical, 0);
        face.set_halign(gtk::Align::Center);
        face.set_valign(gtk::Align::Center);

        let time = gtk::Label::new(None);
        time.add_css_class("clock-time");
        let date = gtk::Label::new(None);
        date.add_css_class("clock-date");
        face.append(&time);
        face.append(&date);
        button.set_child(Some(&face));

        Clock { button, time, date }
    }

    /// Both lines, already formatted by the caller (`main` is the only place that
    /// knows the configured formats). An empty date takes its line out rather
    /// than leaving a blank one, so a config that asks for no date gets a clock of
    /// one line.
    fn render(&self, time: &str, date: &str) {
        self.time.set_text(time);
        self.date.set_text(date);
        self.date.set_visible(!date.is_empty());
        let tooltip = match date.is_empty() {
            true => time.to_string(),
            false => format!("{time} · {date}"),
        };
        self.button.set_tooltip_text(Some(&tooltip));
    }

    fn set_visible(&self, visible: bool) {
        self.button.set_visible(visible);
    }
}

// ---------------------------------------------------------------------------
// The player pill
// ---------------------------------------------------------------------------

/// The player, in the middle of the bar: the cover (or the player's own mark when
/// it has no artwork) and the track, in one rounded capsule.
///
/// The cover is loaded from `mpris:artUrl` through `core::art`, which never
/// blocks: the mark stands in until the picture arrives, and a track that changes
/// while a slow cover is still being fetched drops whatever comes back (the `url`
/// cell is the "is this still the track I asked about?" answer).
struct MediaPill {
    button: gtk::Button,
    /// The left slot of the face: the artwork when there is some, the mark when
    /// there is not. A `gtk::Stack` rather than a child that is swapped out, so
    /// the two never change the pill's size and the picture is kept while the
    /// slot is showing the mark.
    slot: gtk::Stack,
    art: gtk::Image,
    /// The box holding the player's mark, rebuilt when the mark changes (a
    /// different player, or play/pause) without touching the artwork.
    mark: gtk::Box,
    /// The play/pause mark at the end of the pill: the same note the rice's
    /// player carries, and the only thing that says "paused" once a cover is
    /// showing (the mark in the slot is the *player*, not the state).
    hint: gtk::Box,
    label: gtk::Label,
    /// What the face shows now, so a repaint that changes nothing (the same track
    /// arriving again on the heartbeat) leaves the widgets alone.
    shown: RefCell<String>,
    /// The art URL the slot is showing, or waiting for.
    url: Rc<RefCell<String>>,
    icon_size: i32,
    hint_size: i32,
}

impl MediaPill {
    fn new(icon_size: i32, art_size: i32, hint_size: i32, width: i32) -> Self {
        let button = gtk::Button::new();
        button.add_css_class("module");
        button.add_css_class("media");
        button.set_has_frame(false);
        button.set_focus_on_click(false);
        button.set_can_focus(false);

        let face = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        face.set_halign(gtk::Align::Center);
        face.set_valign(gtk::Align::Center);

        let art = gtk::Image::new();
        art.add_css_class("media-art");
        art.set_pixel_size(art_size);

        let mark = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        mark.set_valign(gtk::Align::Center);

        let hint = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        hint.add_css_class("media-hint");
        hint.set_valign(gtk::Align::Center);

        let slot = gtk::Stack::new();
        slot.set_hhomogeneous(false);
        slot.set_vhomogeneous(false);
        slot.add_named(&art, Some("art"));
        slot.add_named(&mark, Some("mark"));
        slot.set_visible_child_name("mark");

        let label = gtk::Label::new(None);
        label.add_css_class("media-title");
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_xalign(0.0);
        // An ellipsised label still asks for the width of its full text; the cap
        // is what stops one long track from deciding how much room the rest of
        // the bar gets.
        text::cap_width(&label, width);

        face.append(&slot);
        face.append(&label);
        face.append(&hint);
        button.set_child(Some(&face));

        MediaPill {
            button,
            slot,
            art,
            mark,
            hint,
            label,
            shown: RefCell::new(String::new()),
            url: Rc::new(RefCell::new(String::new())),
            icon_size,
            hint_size,
        }
    }

    fn render(&self, track: Option<&Track>, playing: bool) {
        let Some(track) = track.filter(|track| !track.is_empty()) else {
            self.button.set_visible(false);
            return;
        };
        let icon = player_icon(&track.player);
        let state = if playing {
            names::AUDIO_LINES
        } else {
            names::PAUSE
        };
        // The artist is dropped before the title is: on a bar, "what is this" is
        // more useful than "who is this".
        let summary = if track.artist.is_empty() {
            track.title.clone()
        } else {
            format!("{} · {}", track.title, track.artist)
        };

        let signature = format!("{icon}|{state}|{summary}");
        {
            let mut shown = self.shown.borrow_mut();
            if *shown != signature {
                *shown = signature;
                while let Some(child) = self.mark.first_child() {
                    self.mark.remove(&child);
                }
                self.mark.append(&icons::lucide(icon, self.icon_size));
                while let Some(child) = self.hint.first_child() {
                    self.hint.remove(&child);
                }
                self.hint.append(&icons::lucide(state, self.hint_size));
                self.label.set_text(&summary);
            }
        }
        self.set_art(&track.art);
        set_class(&self.button, "playing", playing);
        set_class(&self.hint, "playing", playing);
        self.button.set_tooltip_text(Some(&format!(
            "{summary}\n{}",
            if playing { "playing" } else { "paused" }
        )));
        self.button.set_visible(true);
    }

    /// Point the cover slot at `url`, through `core::art` (which decodes a local
    /// file or fetches a remote one, never on the main loop).
    fn set_art(&self, url: &str) {
        if *self.url.borrow() == url {
            return;
        }
        *self.url.borrow_mut() = url.to_string();
        if url.is_empty() {
            self.art.set_paintable(None::<&gtk::gdk::Texture>);
            self.slot.set_visible_child_name("mark");
            return;
        }
        // The mark stands in until the cover is decoded, so a slow fetch never
        // leaves a hole in the pill.
        self.slot.set_visible_child_name("mark");
        let wanted = url.to_string();
        let cell = self.url.clone();
        let slot = self.slot.clone();
        let art = self.art.clone();
        art::load(url, move |texture| {
            // The track changed while this one was being read: whatever arrived
            // belongs to a pill that is no longer showing that track.
            if *cell.borrow() != wanted {
                return;
            }
            match texture {
                Some(texture) => {
                    art.set_paintable(Some(&texture));
                    slot.set_visible_child_name("art");
                }
                None => slot.set_visible_child_name("mark"),
            }
        });
    }

    fn set_visible(&self, visible: bool) {
        self.button.set_visible(visible);
    }
}

// ---------------------------------------------------------------------------
// The workspace row
// ---------------------------------------------------------------------------

/// How long the accent capsule takes to glide from the workspace you were on to
/// the one you switched to.
///
/// The pills' own colour transitions are 180ms (`bar.css`), so the glide is the
/// same size of gesture: long enough to read as movement, over before it becomes
/// something to wait for.
const GLIDE: Duration = Duration::from_millis(180);

/// A rectangle in the workspace row's own coordinates.
///
/// Kept in floats because a glide interpolates it on every frame; the capsule is
/// only ever *drawn* at whole pixels (see [`WorkspaceRow::put`]), but rounding
/// each step of a long ease would stutter at the slow end.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

/// How far a glide that started at `started` has got by `now`, as 0..=1.
///
/// The frame clock's times are microseconds and monotonic, so a glide is a
/// subtraction and a division; the clamps are what keep a late frame (or a clock
/// that went backwards) from drawing the capsule outside its two pills.
fn progress(now: i64, started: i64, span: Duration) -> f64 {
    let span = span.as_micros().max(1) as f64;
    ((now - started).max(0) as f64 / span).clamp(0.0, 1.0)
}

/// The eased fraction of a glide: it leaves at once and settles at the end.
///
/// Ease-out cubic - the shape the CSS transitions on the pills already have. A
/// capsule that started slowly would read as dragged rather than sent, and one
/// that stopped at full speed would look like it had missed the pill.
fn ease_out_cubic(part: f64) -> f64 {
    let part = part.clamp(0.0, 1.0);
    1.0 - (1.0 - part).powi(3)
}

/// The rectangle the capsule draws `eased` of the way from `from` to `to`.
fn between(from: Rect, to: Rect, eased: f64) -> Rect {
    let at = |start: f32, end: f32| start + (end - start) * eased as f32;
    Rect {
        x: at(from.x, to.x),
        y: at(from.y, to.y),
        width: at(from.width, to.width),
        height: at(from.height, to.height),
    }
}

/// Where the row has been asked to put the capsule, and whether it may glide
/// there.
///
/// A move waits when the row has not been laid out: nothing has an allocation
/// before the bar is on screen, and a rebuilt row's buttons do not have one
/// until the next frame.
#[derive(Clone, Copy)]
struct PendingMove {
    workspace: i32,
    animate: bool,
}

/// One glide of the accent capsule.
struct Travel {
    /// Where the capsule started and where it is going, in row coordinates.
    from: Rect,
    to: Rect,
    /// The workspace it is gliding to: the number inside changes half way, where
    /// the movement hides the change.
    to_id: i32,
    /// The frame time of the first frame, which is where the glide's clock
    /// starts (the tick that starts it has no time of its own yet).
    started: Option<i64>,
}

/// What the row was last drawn from, and what its capsule is doing about it.
#[derive(Default)]
struct RowState {
    /// The workspace ids the buttons were made for, in row order.
    ids: Vec<i32>,
    /// One pill per id. Kept between renders so that a switch restyles the
    /// buttons instead of replacing them - a replaced button is what used to make
    /// the row snap.
    pills: Vec<Pill>,
    /// The signature of the snapshot the row was last drawn for.
    signature: String,
    /// The rectangle the capsule is at, or `None` before it has been placed.
    rect: Option<Rect>,
    /// The workspace the number in the capsule names.
    label: Option<i32>,
    /// The workspace the capsule is at, or gliding to.
    place: Option<i32>,
    /// A move that is waiting for the row to be laid out (see [`PendingMove`]).
    pending: Option<PendingMove>,
    /// The glide in progress, if any.
    travel: Option<Travel>,
    /// The tick callback that is running the glide, so a switch arriving
    /// mid-glide can take it over from where the capsule has got to.
    glide_tick: Option<gtk::TickCallbackId>,
    /// Whether a tick is waiting for the layout to catch up (see
    /// [`WorkspaceRow::settle`]).
    settling: bool,
}

impl RowState {
    /// Stop the glide where it is: a retarget starts a new one from there.
    fn cancel_glide(&mut self) {
        if let Some(tick) = self.glide_tick.take() {
            tick.remove();
        }
        self.travel = None;
    }
}

/// The row of workspace pills, and the accent capsule above them that says which
/// one you are on.
///
/// The capsule is a widget of its own rather than the active button's
/// background, because it is the thing that *moves*, and a background cannot
/// slide from one pill to another. Its position is not something a stylesheet
/// can be told: the distance between two pills is only known once the row has
/// been laid out, and a `GtkOverlay` places an overlay child by its alignment
/// alone, ignoring its margins - so the row stacks a `GtkFixed` over the pills
/// and moves the capsule inside it, frame by frame, from `add_tick_callback`
/// (see [`WorkspaceRow::glide`]).
///
/// The buttons are rebuilt only when the *set* of workspaces changes; a switch
/// restyles them in place. That split is what the glide needs: its journey starts
/// from wherever the capsule was on screen, which only exists as long as the row
/// is not thrown away.
struct WorkspaceRow {
    /// The row as a widget: the pills, with the capsule on top.
    root: gtk::Overlay,
    /// The buttons, in workspace order.
    row: gtk::Box,
    /// The absolute-position layer the capsule lives on.
    track: gtk::Fixed,
    /// The accent pill: the workspace you are on, as its number.
    capsule: gtk::Label,
    /// How large a workspace mark is drawn (see [`WS_ICON`]), kept for the
    /// buttons a rebuild makes.
    ws_icon: i32,
    state: RefCell<RowState>,
}

impl WorkspaceRow {
    /// Build the empty row. It fills in on the first [`WorkspaceRow::render`].
    fn new(ws_icon: i32) -> Rc<Self> {
        let root = gtk::Overlay::new();
        root.add_css_class("workspaces");

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        root.set_child(Some(&row));

        // The capsule's layer. A fixed container is the only widget that takes an
        // absolute position, and neither it nor the capsule may eat the clicks
        // and hovers the pills underneath are there for.
        let track = gtk::Fixed::new();
        track.set_can_target(false);
        // A fixed clips what it holds to its own box, which would cut the
        // capsule's glow off at the row's edges - the glow is what separates it
        // from the bar behind it, so it has to be able to spill.
        track.set_overflow(gtk::Overflow::Visible);
        root.add_overlay(&track);

        let capsule = gtk::Label::new(None);
        capsule.add_css_class("ws-capsule");
        capsule.set_can_target(false);
        capsule.set_visible(false);
        track.put(&capsule, 0.0, 0.0);

        let this = Rc::new(WorkspaceRow {
            root,
            row,
            track,
            capsule,
            ws_icon,
            state: RefCell::new(RowState::default()),
        });

        // The first render runs before the bar is on screen (so nothing has an
        // allocation then), and a bar that was hidden and shown again has a layout
        // that may have moved underneath the capsule. Both are moments to place it
        // again - a frame later, when the row can be measured (see `settle`).
        let me = Rc::downgrade(&this);
        this.root.connect_map(move |_| {
            let Some(row) = me.upgrade() else {
                return;
            };
            {
                let mut state = row.state.borrow_mut();
                if let Some(place) = state.place {
                    state.pending = Some(PendingMove {
                        workspace: place,
                        animate: false,
                    });
                }
            }
            row.ensure_settle();
        });

        this
    }

    /// Draw the row from one Hyprland snapshot.
    fn render(self: &Rc<Self>, workspaces: &[Workspace]) {
        // The signature is the whole picture the row draws - which workspaces
        // exist, what is in them, and which one you are on. A repaint that changes
        // none of it (the clock's heartbeat, a volume tick) must not touch a
        // widget, let alone restart a glide.
        let signature = workspaces
            .iter()
            .map(|workspace| {
                format!(
                    "{}:{}:{}:{}",
                    workspace.id,
                    workspace.windows,
                    u8::from(workspace.active),
                    u8::from(workspace.visible)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        {
            let mut state = self.state.borrow_mut();
            if state.signature == signature {
                return;
            }
            state.signature = signature;
        }

        self.rebuild(workspaces);
        self.paint(workspaces);
        self.aim(
            workspaces
                .iter()
                .find(|workspace| workspace.active)
                .map(|workspace| workspace.id),
        );
    }

    /// Make one button per workspace when the list of workspaces changes.
    ///
    /// Not on a switch: the capsule needs the old buttons' rectangles to glide
    /// from, and a workspace that has just come into existence appends a pill at
    /// the end without moving the ones before it.
    fn rebuild(self: &Rc<Self>, workspaces: &[Workspace]) {
        let ids: Vec<i32> = workspaces.iter().map(|workspace| workspace.id).collect();
        if self.state.borrow().ids == ids {
            return;
        }

        while let Some(child) = self.row.first_child() {
            self.row.remove(&child);
        }
        let mut pills = Vec::with_capacity(workspaces.len());
        for workspace in workspaces {
            let pill = Pill::new("ws", self.ws_icon);
            // The number lives in the capsule rather than in this button (see
            // `paint`), so the name a screen reader would otherwise have found
            // only in the label is given to the button itself.
            pill.button
                .update_property(&[gtk::accessible::Property::Label(&format!(
                    "Workspace {}",
                    workspace.id
                ))]);
            {
                let id = workspace.id.to_string();
                pill.on_click(move || {
                    hypr_osd_core::hypripc::request(&format!(
                        "dispatch hl.dsp.focus({{ workspace = {id} }})"
                    ));
                });
            }
            pill.on_scroll(move |direction| {
                // `e+1`/`e-1` are Hyprland's own "the next existing workspace"
                // relative forms, so the wheel skips the gaps the way the
                // workspace swipe does.
                let step = if direction > 0 { "e+1" } else { "e-1" };
                hypr_osd_core::hypripc::request(&format!(
                    "dispatch hl.dsp.focus({{ workspace = \"{step}\" }})"
                ));
            });
            self.row.append(&pill.button);
            pills.push(pill);
        }

        let mut state = self.state.borrow_mut();
        // A glide belongs to the old row; the next one starts from wherever the
        // capsule is now (`rect`), so the number cannot arrive on a pill that the
        // rebuild has just moved away from under it.
        state.cancel_glide();
        state.ids = ids;
        state.pills = pills;
        // The buttons are new, so the capsule's place among them has to be found
        // again (the row has no allocations for them until the next frame).
        state.place = None;
    }

    /// Restyle the buttons for one snapshot, without replacing any of them.
    fn paint(&self, workspaces: &[Workspace]) {
        let state = self.state.borrow();
        for (pill, workspace) in state.pills.iter().zip(workspaces) {
            // An idle workspace is a dot, and the *shape* carries its state:
            // filled when it holds a window, hollow when it is empty. That leaves
            // the colour free for the one thing shape cannot say - the accent,
            // for a workspace that is up on another monitor.
            //
            // The workspace you are on carries no mark here at all: the capsule
            // above the row draws its number, and this button is only the target
            // a click lands on.
            pill.set_one(
                if workspace.active {
                    ""
                } else if workspace.windows > 0 {
                    names::WORKSPACE_ACTIVE
                } else {
                    names::WORKSPACE_IDLE
                },
                "",
                &[
                    ("active", workspace.active),
                    ("visible", workspace.visible),
                    ("occupied", workspace.windows > 0),
                    ("empty", workspace.windows == 0),
                ],
            );
            pill.tooltip(Some(&workspace_tooltip(workspace)));
        }
    }

    /// Put the capsule where the focused workspace is: glide there if it is
    /// already on screen, land on it if it is not.
    fn aim(self: &Rc<Self>, wanted: Option<i32>) {
        let Some(wanted) = wanted else {
            // Nothing the row draws has the focus (a special workspace has no
            // pill): there is nowhere for the capsule to stand, so it goes away.
            {
                let mut state = self.state.borrow_mut();
                state.pending = None;
                state.place = None;
                state.cancel_glide();
            }
            self.capsule.set_visible(false);
            return;
        };
        // A workspace the row has no pill for cannot be glided to. That should
        // not happen - the row is built from the same snapshot the focus comes
        // from - but the two IPC reads behind that snapshot are separate, so a
        // workspace created between them can leave the focus ahead of the row for
        // a moment. The capsule is left where it is and the next snapshot aims it;
        // waiting here would leave the settle tick retrying for a pill that is not
        // there yet, for ever.
        if !self.state.borrow().ids.contains(&wanted) {
            {
                let mut state = self.state.borrow_mut();
                state.pending = None;
                state.cancel_glide();
                state.place = None;
            }
            return;
        }
        let animate = self.capsule.is_visible();
        // A glide already on its way to this workspace is not disturbed by a
        // repaint that changed only some other field (a window count, another
        // monitor's `visible`): restarting the clock would stretch the movement.
        if self
            .state
            .borrow()
            .travel
            .as_ref()
            .is_some_and(|travel| travel.to_id == wanted)
        {
            return;
        }
        {
            let state = self.state.borrow();
            let settled =
                state.place == Some(wanted) && state.travel.is_none() && state.pending.is_none();
            if settled && animate {
                return;
            }
            if state
                .pending
                .is_some_and(|pending| pending.workspace == wanted)
            {
                return;
            }
        }
        self.move_to(wanted, animate);
    }

    /// Put the capsule on `workspace`, gliding from where it is when `animate`
    /// is set and the row has been laid out enough to measure the pill.
    fn move_to(self: &Rc<Self>, workspace: i32, animate: bool) {
        let Some(to) = self.bounds_of(workspace) else {
            // The bar is not on screen yet, or a rebuilt row's buttons have no
            // allocation until the next frame: wait for the layout.
            self.state.borrow_mut().pending = Some(PendingMove { workspace, animate });
            self.ensure_settle();
            return;
        };
        let from = self.state.borrow().rect;
        match from {
            Some(from) if animate && animations_enabled() && from != to => {
                self.glide(from, to, workspace)
            }
            _ => self.land(to, workspace),
        }
    }

    /// Start a glide: every frame moves the capsule a little further, and the
    /// number in it changes at the half-way point (the movement is what hides the
    /// change). A switch that arrives mid-glide takes the glide over from where
    /// the capsule has got to instead of starting again.
    fn glide(self: &Rc<Self>, from: Rect, to: Rect, workspace: i32) {
        {
            let mut state = self.state.borrow_mut();
            state.cancel_glide();
            state.pending = None;
            state.place = Some(workspace);
            state.travel = Some(Travel {
                from,
                to,
                to_id: workspace,
                started: None,
            });
        }
        let me = Rc::downgrade(self);
        let tick = self.root.add_tick_callback(move |_, clock| {
            let Some(row) = me.upgrade() else {
                return glib::ControlFlow::Break;
            };
            row.step(clock.frame_time())
        });
        self.state.borrow_mut().glide_tick = Some(tick);
    }

    /// One frame of the glide. Returns whether there is another one.
    fn step(self: &Rc<Self>, now: i64) -> glib::ControlFlow {
        let mut state = self.state.borrow_mut();
        if state.travel.is_none() {
            // Nobody took the callback away, but there is nothing left to move.
            state.glide_tick = None;
            return glib::ControlFlow::Break;
        }
        let (started, from, to, to_id) = {
            let travel = state.travel.as_mut().expect("checked above");
            (
                *travel.started.get_or_insert(now),
                travel.from,
                travel.to,
                travel.to_id,
            )
        };
        let part = progress(now, started, GLIDE);
        let finished = part >= 1.0;
        let rect = if finished {
            to
        } else {
            between(from, to, ease_out_cubic(part))
        };
        // The half-way point: by now the capsule is moving fast enough to hide
        // the number changing under the eye.
        if part >= 0.5 && state.label != Some(to_id) {
            state.label = Some(to_id);
            self.capsule.set_text(&to_id.to_string());
        }
        if finished {
            state.travel = None;
            state.glide_tick = None;
        }
        state.rect = Some(rect);
        state.place = Some(to_id);
        drop(state);
        self.put(rect);
        if finished {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    }

    /// Put the capsule down without a glide: the first paint (there is nothing to
    /// glide from yet), or a desktop that has asked GTK for no animations. A
    /// rebuilt row is not one of those cases - it cancels a running glide and the
    /// next one starts from wherever the capsule has got to (see `rebuild`).
    fn land(self: &Rc<Self>, rect: Rect, workspace: i32) {
        {
            let mut state = self.state.borrow_mut();
            state.cancel_glide();
            state.pending = None;
            state.place = Some(workspace);
            state.rect = Some(rect);
            state.label = Some(workspace);
        }
        self.capsule.set_text(&workspace.to_string());
        self.put(rect);
        self.capsule.set_visible(true);
    }

    /// Draw the capsule at `rect`.
    ///
    /// Its size is a size *request*: a fixed allocates its children at their
    /// *minimum*, which is where the request lands (the label's own minimum - the
    /// widest number it could show - is smaller than the pill). Its position is
    /// the child's transform. Those are the two numbers this code animates, and
    /// both are rounded to whole pixels: a bar of hairlines and 12px text is not
    /// improved by a capsule that sits on half a pixel.
    fn put(&self, rect: Rect) {
        self.capsule.set_size_request(
            rect.width.round().max(1.0) as i32,
            rect.height.round().max(1.0) as i32,
        );
        self.track.move_(
            &self.capsule,
            f64::from(rect.x.round()),
            f64::from(rect.y.round()),
        );
    }

    /// Where a workspace's pill is inside the row, in the row's coordinates.
    ///
    /// `compute_bounds` is GTK's own answer, so it accounts for the row's layout
    /// rather than for anything guessed here. `None` before the bar is on screen
    /// (or before a rebuilt row has been laid out) is what makes a move wait.
    ///
    /// The guard is not merely "is the rectangle non-empty": a button that has
    /// not been allocated *still* answers `compute_bounds` with a degenerate
    /// rectangle instead of nothing (measured: `-8,-1 16x2` on the first paint).
    /// Trusting that placed the capsule from a layout that had not happened yet,
    /// and nothing aimed it again until the workspace changed - so the number sat
    /// off its pill until you switched. A real pill is at least a mark wide and
    /// tall and lies inside the row, which is what these two tests check. */
    fn bounds_of(&self, workspace: i32) -> Option<Rect> {
        let state = self.state.borrow();
        let index = state.ids.iter().position(|id| *id == workspace)?;
        let pill = state.pills.get(index)?;
        let bounds = pill.button.compute_bounds(&self.root)?;
        let rect = Rect {
            x: bounds.x(),
            y: bounds.y(),
            width: bounds.width(),
            height: bounds.height(),
        };
        let inside = rect.x >= -1.0
            && rect.y >= -1.0
            && rect.x + rect.width <= self.root.width() as f32 + 1.0
            && rect.y + rect.height <= self.root.height() as f32 + 1.0;
        (rect.width >= 8.0 && rect.height >= 8.0 && inside).then_some(rect)
    }

    /// Ask for a frame in which to carry out a move that had to wait for the
    /// layout. Only one wait runs at a time, and a wait that still has nothing to
    /// measure keeps the same tick rather than adding another.
    fn ensure_settle(self: &Rc<Self>) {
        if self.state.borrow().settling {
            return;
        }
        self.state.borrow_mut().settling = true;
        let me = Rc::downgrade(self);
        self.root.add_tick_callback(move |_, _| {
            let Some(row) = me.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if row.settle() {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// One frame of the wait: carry out the pending move if the row can be
    /// measured now. Returns whether the wait is over.
    fn settle(self: &Rc<Self>) -> bool {
        let pending = self.state.borrow_mut().pending.take();
        if let Some(pending) = pending {
            self.move_to(pending.workspace, pending.animate);
        }
        let done = self.state.borrow().pending.is_none();
        if done {
            self.state.borrow_mut().settling = false;
        }
        done
    }

    /// Whether the row is on the bar at all (`show_workspaces = false` turns it
    /// off, like any other part).
    fn set_visible(&self, visible: bool) {
        self.root.set_visible(visible);
    }
}

/// Whether the desktop asks GTK to animate at all.
///
/// GTK's own setting is the accessibility switch for motion ("Enable
/// animations"): a desktop that has turned it off gets the landing without the
/// glide, which is the whole point of the switch. The row is as readable either
/// way; it is only still.
fn animations_enabled() -> bool {
    gtk::Settings::default().is_none_or(|settings| settings.is_gtk_enable_animations())
}

// ---------------------------------------------------------------------------
// The bar
// ---------------------------------------------------------------------------

pub struct BarView {
    /// The output this bar is on (`eDP-1`, `DP-2`, ...). The bars are held under
    /// it by `main`, and it is what the bar tells its two popups: they are drawn
    /// on this screen and measured against it.
    connector: String,
    pub root: gtk::CenterBox,
    /// The button that opens the applications panel, at the far left.
    apps: Pill,
    /// The workspace row: one button per workspace, with the accent capsule that
    /// glides between them when you switch.
    workspaces: Rc<WorkspaceRow>,
    title: gtk::Label,
    clock: Clock,
    media: MediaPill,
    /// The combined status pill: the link, the sound and the battery, and the
    /// handle the system popup unfolds from.
    status: Pill,
    tray: gtk::Box,
    /// The windows the bar has put away, drawn at the left of the tray's row.
    minimised: gtk::Box,
    power: Pill,
    /// The three readings behind the status pill, kept together because the pill
    /// is drawn from all of them at once (see [`Status`]).
    readings: RefCell<Status>,
    /// The same for the put-away icons, which are buttons too - and one the
    /// pointer may well be resting on (see [`BarView::render_minimised`]).
    last_minimised: RefCell<String>,
    /// How large a tray icon is drawn, in layout pixels. The put-away icons are
    /// the same size as the application indicators beside them: they are the same
    /// gesture. Larger while the machine is a tablet - the icon *is* the target,
    /// so it grows with the finger.
    icon_size: i32,
    /// Which parts of the bar the config asks for (see [`Shows`]). Kept rather
    /// than read once, because four of the pills show and hide themselves as
    /// their data arrives and the switch has to win over that.
    shows: Shows,
}

/// The last reading of each part of the status pill.
///
/// The three arrive on different clocks - the volume every second, the link
/// every five seconds, the battery every thirty - so the pill is painted from all
/// three of them rather than each one repainting a third of a pill. Without that
/// it would spend most of its life showing one number and two blanks, which is
/// exactly what the three separate pills had to be told not to do (by each of
/// them hiding itself).
#[derive(Default)]
struct Status {
    volume: Option<Volume>,
    /// `Err` is "this machine has no wireless card, or no `iw` to ask", which is
    /// a different answer from "there is a card and nothing is joined" - the
    /// distinction [`sources::network`] makes. `None` here means "nothing read
    /// yet", which draws the same as "no card": saying nothing about a link is
    /// the only honest answer before the first reading.
    network: Option<Result<Option<Network>, ()>>,
    battery: Option<Battery>,
}

impl BarView {
    /// Build one monitor's bar. `connector` is the output it is on, which it
    /// needs to tell its two popups where they belong: they are drawn on the
    /// bar's screen and measured against it, and the bar is the only one who
    /// knows which screen that is. `tablet` is the layout the bar's surface was
    /// made for - the marks grow with it, and a rebuild after a fold builds the
    /// pills again at the new size.
    pub fn new(settings: &Rc<Settings>, connector: &str, tablet: bool) -> Self {
        let icon = if tablet {
            ICON + TOUCH_ICON_EXTRA
        } else {
            ICON
        };
        let ws_icon = if tablet {
            WS_ICON + TOUCH_ICON_EXTRA
        } else {
            WS_ICON
        };
        let apps_icon = if tablet {
            APPS_ICON + TOUCH_ICON_EXTRA
        } else {
            APPS_ICON
        };
        let power_icon = if tablet {
            POWER_ICON + TOUCH_ICON_EXTRA
        } else {
            POWER_ICON
        };
        let media_art = if tablet {
            MEDIA_ART + TOUCH_ICON_EXTRA
        } else {
            MEDIA_ART
        };
        let media_hint = if tablet {
            MEDIA_HINT + TOUCH_ICON_EXTRA
        } else {
            MEDIA_HINT
        };

        // --- left: the workspaces and the focused window -------------------
        let workspaces = WorkspaceRow::new(ws_icon);

        let title = gtk::Label::new(None);
        title.add_css_class("title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_xalign(0.0);
        // An ellipsised label still asks for its full width, so the title is
        // capped in characters; without this one long title would decide how
        // much room the right half of the bar gets.
        text::cap_width(&title, settings.title_width);

        // The applications button is the first thing in the bar, at the far left -
        // the corner every desktop keeps the thing that opens everything else.
        // It unfolds the panel like the clock unfolds the island: the pointer
        // arriving (or a click) *asks* the element to open it, and the element
        // takes it away again when the pointer has left both the button and the
        // panel. The handler is wired after the row exists, because the request
        // measures the button and needs the tree it ends up in.
        let apps = Pill::new("apps", apps_icon);

        let left = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        left.add_css_class("section");
        // Which screen edge this tab is welded to. The two outer tabs run into
        // the corners of the screen, so their outer corners are square and only
        // the one facing the bar is rounded (see `edge-left` in bar.css).
        left.add_css_class("edge-left");
        left.append(&apps.button);
        left.append(&workspaces.root);
        left.append(&title);

        // --- centre: the player --------------------------------------------
        let media = MediaPill::new(icon, media_art, media_hint, settings.media_width);
        // The pointer's left click is play/pause, but a *finger's* tap unfolds the
        // island instead - the panel is where the finger-sized transport lives.
        // That handler is wired after the row is built, because the request
        // measures the pill (see below).
        {
            // The previous track is on the middle button; GTK's `clicked` is
            // button 1 only, so it arrives as a gesture.
            let gesture = gtk::GestureClick::new();
            gesture.set_button(gtk::gdk::BUTTON_MIDDLE);
            gesture.connect_pressed(|_, _, _, _| {
                sources::launch("playerctl", &["previous"]);
            });
            media.button.add_controller(gesture);
        }
        {
            // Scrolling seeks: `5+` is "five seconds further in".
            let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
            scroll.connect_scroll(|_, _dx, dy| {
                let spec = if dy < 0.0 { "5+" } else { "5-" };
                sources::launch("playerctl", &["position", spec]);
                glib::Propagation::Stop
            });
            media.button.add_controller(scroll);
        }
        // Next track had the right button. A finger reaches it with a long press
        // instead (the same binding the whole bar uses); the middle click for the
        // *previous* track stays mouse-only, because a finger has no third
        // gesture to spare and the play button is right beside it.
        touch::on_secondary(&media.button, || {
            sources::launch("playerctl", &["next"]);
        });

        // --- the clock, at the right end ------------------------------------
        let clock = Clock::new();
        // The clock is a reading now, not the island's handle - the player in the
        // middle unfolds it. Its clicks still run the notification centre and
        // do-not-disturb, which is what they always did; a long press is the same
        // gesture on a touchscreen (see `touch::on_secondary`).
        {
            let notifications = settings.notifications_command.clone();
            touch::on_click(&clock.button, move |_| {
                sources::launch(&notifications, &["-t", "-sw"]);
            });
        }
        {
            let notifications = settings.notifications_command.clone();
            touch::on_secondary(&clock.button, move || {
                sources::launch(&notifications, &["-d", "-sw"]);
            });
        }

        // --- right: tray, status, power, clock ------------------------------
        let tray = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        tray.add_css_class("tray");

        // The windows the bar has put away are drawn in the same row as the tray's
        // icons, at its left end - they are the tray's other half (see `minimise`).
        // The box is a sibling of the tray rather than a child of it, so the two
        // can be switched off independently by the settings window; every bar
        // draws these, while the application indicators are one session-wide D-Bus
        // object that only one bar can carry. Putting a window away on one screen
        // therefore shows its icon on both.
        let minimised = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        minimised.add_css_class("minimised");

        // --- the combined status pill --------------------------------------
        //
        // One pill for the three things about the machine you glance at - the
        // wireless link, the sound, the battery - where there used to be three.
        // They belong together because they are one question ("is this machine
        // alright?") and because they are all *handles*: this pill unfolds the
        // system popup, which is where the detail lives now.
        let status = Pill::new("status", icon);
        {
            let stats = settings.stats_command.clone();
            // The left click is the popup's other handle, but it is wired
            // further down with the hover: both send the same request, and that
            // request has to measure the pill - which needs the widget tree the
            // pill ends up in (see below).
            status
                // The microphone is not part of the combined reading (it is
                // about the source, not the sink), so it keeps the corner it had
                // on the volume pill: one click away, out of the way.
                .on_button(gtk::gdk::BUTTON_MIDDLE, || {
                    sources::launch("wpctl", &["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"]);
                })
                // The wheel is the volume, as it was on the volume pill: the
                // element that owns the step does it, so the level shows up in
                // the pill a moment later and the card appears over it.
                .on_scroll({
                    let settings = settings.clone();
                    move |direction| {
                        let verb = if direction > 0 { "up" } else { "down" };
                        let fallback = if direction > 0 { "5%+" } else { "5%-" };
                        sources::nudge_volume(
                            &settings.volume_command,
                            verb,
                            &["wpctl", "set-volume", sources::SINK, fallback],
                        );
                    }
                });
            // The wireless radio on/off had the right button. The *element* owns
            // that switch (it reads `rfkill` and knows what it means), so the
            // pill asks it rather than guessing with `nmcli` - and the touchscreen
            // reaches the same verb with a long press.
            touch::on_secondary(&status.button, move || {
                sources::launch(&stats, &["wifi"]);
            });
        }

        // The power button closes the row: between the status pill and the
        // clock, which keeps the dangerous button off the corner a pointer flies
        // into.
        let power = Pill::new("power", power_icon);
        {
            let session = settings.session_command.clone();
            power.on_click(move || {
                sources::launch(&session, &["toggle"]);
            });
        }

        let right = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        right.add_css_class("section");
        right.add_css_class("edge-right");
        right.append(&minimised);
        right.append(&tray);
        // The status pill sits to the right of the tray, where the rice puts its
        // network, sound and battery readings - and where the system popup's
        // own geometry already expects it (right-aligned with the bar).
        right.append(&status.button);
        // The power button sits between the status pill and the clock, which
        // keeps the dangerous button off the corner a pointer flies into while
        // leaving the clock as the last thing in the row.
        right.append(&power.button);
        right.append(&clock.button);

        // The three pills reach the surface's top edge: the bar paints nothing
        // behind them, so their shapes are all that is on screen - and a tab that
        // floated a few pixels down would be a pill again, not a tab. The height
        // is the one the surface was made with (a fold makes it taller), which is
        // why it is asked here rather than written in CSS.
        let pill_height = settings.bar_height(tablet);
        for section in [&left, &right] {
            section.set_size_request(-1, pill_height);
        }
        media.button.set_size_request(-1, pill_height);

        // --- the row -------------------------------------------------------
        let root = gtk::CenterBox::new();
        root.set_start_widget(Some(&left));
        root.set_center_widget(Some(&media.button));
        root.set_end_widget(Some(&right));

        // The player is the island popup's handle: the pointer arriving on it
        // *asks* the island to unfold, and a finger's tap opens it pinned - the
        // same pair of gestures the clock carried when it was the bar's centre
        // child. The island decides when to go away again, because leaving a
        // popup is the pointer moving somewhere the bar cannot see (onto the
        // popup itself).
        //
        // The bar sends the pill's own rectangle, measured inside the bar's card,
        // because the player is as wide as its track: a fixed strip would be
        // either narrower than the pill or wide enough to reach the workspaces.
        // Wired here rather than with the transport bindings, because measuring
        // the pill needs the widget tree it ends up in.
        {
            let island = settings.island_command.clone();
            let pill = media.button.clone();
            let root = root.clone();
            let connector = connector.to_owned();
            let ask = Rc::new(move |verb: &str| {
                let mut args: Vec<String> = vec![verb.to_string(), connector.clone()];
                if let Some(card) = root.parent() {
                    if let Some(bounds) = bounds_in(&pill, &card) {
                        args.extend(bounds.iter().map(|value| format!("{value:.0}")));
                    }
                }
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                sources::launch(&island, &refs);
            });
            {
                let ask = ask.clone();
                let motion = gtk::EventControllerMotion::new();
                motion.connect_enter(move |motion, _, _| {
                    // A finger has no hover, and GDK turns its tap into an
                    // emulated pointer that enters the pill and never leaves.
                    if touch::tablet() || from_finger(motion) {
                        return;
                    }
                    ask("open");
                });
                media.button.add_controller(motion);
            }
            // A pointer's click is play/pause; a finger's tap is the island,
            // whose transport is sized for it. The binding is a
            // `touch::on_click` so it can tell the two apart, and so the long
            // press beside it (next track) takes precedence.
            touch::on_click(&media.button, move |from_finger| {
                if from_finger {
                    ask("tap");
                } else {
                    sources::launch("playerctl", &["play-pause"]);
                }
            });
        }

        // The applications button's handles: the pointer arriving on it, and a
        // press. Both ask the panel to open - with the connector the bar is on
        // and the button's own rectangle - but with a different *verb*: a pointer
        // gets `open`, and the panel watches the pointer and closes itself when
        // it leaves; a finger gets `tap`, because there is no pointer to leave
        // and the panel therefore opens pinned with a tap catcher around it.
        // Either way the request is `open`-shaped rather than `toggle`: a
        // `toggle` would put the panel into its *pinned* state, which is not
        // watched at all. Wired here rather than with the pill's other handlers,
        // because the request has to *measure* the button - for which it needs
        // the widget tree the button lives in.
        {
            let command = settings.apps_command.clone();
            let pill = apps.button.clone();
            let root = root.clone();
            let connector = connector.to_owned();
            let ask = Rc::new(move |verb: &str| {
                // Two things the panel cannot work out for itself: which monitor
                // the bar is on (the panel is drawn on that screen and measured
                // against it), and where the button is. The rectangle is in the
                // bar's *card*, because the panel knows the bar's margins and the
                // monitor's position but not the button's own width.
                let mut args: Vec<String> = vec![verb.to_string(), connector.clone()];
                if let Some(card) = root.parent() {
                    if let Some(bounds) = bounds_in(&pill, &card) {
                        args.extend(bounds.iter().map(|value| format!("{value:.0}")));
                    }
                }
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                sources::launch(&command, &refs);
            });
            {
                let ask = ask.clone();
                let motion = gtk::EventControllerMotion::new();
                motion.connect_enter(move |motion, _, _| {
                    // The emulated pointer a tap produces enters and never
                    // leaves; only a real pointer's arrival means "watch me".
                    if touch::tablet() || from_finger(motion) {
                        return;
                    }
                    ask("open");
                });
                apps.button.add_controller(motion);
            }
            touch::on_click(&apps.button, move |from_finger| {
                ask(if from_finger { "tap" } else { "open" });
            });
        }

        // The two handles that unfold the system popup: the pointer arriving on
        // the status pill, and a press. Both measure the pill and send the same
        // two things - which monitor the bar is on, and where the pill is - and
        // both send an `open`-shaped request rather than a `toggle`.
        //
        // The *verb* differs with who is asking: a pointer gets `open`, and the
        // panel watches it and closes itself when it leaves; a finger gets `tap`,
        // which opens the panel pinned with a tap catcher around it. (What a
        // click used to send is worth remembering, because it is the bug this
        // shape exists to prevent: `toggle` puts the panel into the machine's
        // *pinned* state, which is not watched at all, so a single click left it
        // up for good, glowing, with the pointer long gone.)
        //
        // Wired here rather than with the pill's other handlers, because the
        // request has to *measure* the pill - for which it needs the widget tree
        // the pill lives in.
        {
            let stats = settings.stats_command.clone();
            let pill = status.button.clone();
            let root = root.clone();
            let connector = connector.to_owned();
            let ask = Rc::new(move |verb: &str| {
                // Two things the panel cannot work out for itself, so the bar
                // says them: which monitor the bar is on (the panel is drawn on
                // that screen and measured against it, and it has no way of
                // knowing which of the bars the pointer came from), and where the
                // pill is. The tray and the power button sit to the pill's right,
                // so its hot zone cannot be derived from the bar's edge either -
                // and the rectangle below is measured inside the bar's card,
                // because the panel knows the bar's margins and the monitor's
                // position but not this.
                let mut args: Vec<String> = vec![verb.to_string(), connector.clone()];
                if let Some(card) = root.parent() {
                    if let Some(bounds) = bounds_in(&pill, &card) {
                        args.extend(bounds.iter().map(|value| format!("{value:.0}")));
                    }
                }
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                sources::launch(&stats, &refs);
            });
            {
                let ask = ask.clone();
                let motion = gtk::EventControllerMotion::new();
                motion.connect_enter(move |motion, _, _| {
                    if touch::tablet() || from_finger(motion) {
                        return;
                    }
                    ask("open");
                });
                status.button.add_controller(motion);
            }
            touch::on_click(&status.button, move |from_finger| {
                ask(if from_finger { "tap" } else { "open" });
            });
        }

        let this = BarView {
            connector: connector.to_owned(),
            root,
            apps,
            workspaces,
            title,
            clock,
            media,
            status,
            tray,
            minimised,
            power,
            readings: RefCell::new(Status::default()),
            last_minimised: RefCell::new(String::new()),
            icon_size: settings.tray_icon_size(tablet),
            shows: settings.shows,
        };
        // A first paint, so the bar that appears is already correct rather than
        // appearing empty and filling in over the next second.
        this.power.set_icon(names::POWER);
        this.power
            .tooltip(Some("Session: lock, suspend, log out, reboot, shut down"));
        this.apps.set_icon(names::GRID);
        this.apps
            .tooltip(Some("Applications: everything installed, in drawers"));
        this.media.set_visible(false);
        // And the status pill waits for the first readings of all three of its
        // parts, which is the first `refresh`.
        this.status.set_visible(false);

        // What the config switched off. The four pills above decide their own
        // visibility as their data arrives, so those flags are applied *there*
        // instead (see `render_media`, `render_stats`, `render_status` and
        // `render_title`); these are the parts that are always up.
        this.apps.set_visible(this.shows.apps);
        this.workspaces.set_visible(this.shows.workspaces);
        this.clock.set_visible(this.shows.clock);
        this.minimised.set_visible(this.shows.minimised);
        this.tray.set_visible(this.shows.tray);
        this.power.set_visible(this.shows.power);
        this
    }

    /// The tray's container, for the StatusNotifier host to fill.
    pub fn tray_container(&self) -> &gtk::Box {
        &self.tray
    }

    /// The output this bar is on.
    pub fn connector(&self) -> &str {
        &self.connector
    }

    /// The workspace row and the window title, from one Hyprland snapshot.
    pub fn render_hypr(&self, snapshot: &Snapshot) {
        self.render_workspaces(&snapshot.workspaces);
        self.render_title(snapshot);
    }

    /// The windows that are put away: one icon per window, at the left of the
    /// tray's row. A click brings that window back.
    ///
    /// The icons are rebuilt only when what they show changes. Rebuilding on
    /// every repaint would replace the button under the pointer - which loses a
    /// hover and, worse, can swallow the click that was about to land on it.
    pub fn render_minimised(&self, entries: &[Minimised]) {
        let signature = entries
            .iter()
            .map(|entry| {
                format!(
                    "{}:{}:{}:{}",
                    entry.window.address,
                    entry.window.class,
                    entry.window.title,
                    entry.home.as_deref().unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        {
            let mut last = self.last_minimised.borrow_mut();
            if *last == signature {
                return;
            }
            *last = signature;
        }

        while let Some(child) = self.minimised.first_child() {
            self.minimised.remove(&child);
        }
        for entry in entries {
            let button = gtk::Button::new();
            button.add_css_class("minimised-item");
            button.set_has_frame(false);
            button.set_focus_on_click(false);
            button.set_can_focus(false);
            button.set_child(Some(&picture(&entry.window, self.icon_size)));
            button.set_tooltip_text(Some(&entry.tooltip()));
            // What a click does is the entry's own business, and the entry is
            // cloned into the handler: the *list* it came from is thrown away at
            // the next repaint, and the icons are rebuilt with it.
            let entry = entry.clone();
            button.connect_clicked(move |_| {
                if let Err(error) = minimise::bring_back(&entry) {
                    eprintln!("hypr-osd-bar: {error}");
                }
            });
            self.minimised.append(&button);
        }
    }

    /// The workspace row: one button per workspace and the accent capsule over
    /// them, which glides to the workspace you switch to (see [`WorkspaceRow`]).
    fn render_workspaces(&self, workspaces: &[Workspace]) {
        self.workspaces.render(workspaces);
    }

    fn render_title(&self, snapshot: &Snapshot) {
        let title = snapshot.title.trim();
        self.title.set_text(title);
        // An empty title takes the label (and its gap) out of the row rather
        // than leaving a hole where a window title would be - and a title the
        // config switched off stays out however full the row is.
        self.title
            .set_visible(self.shows.title && !title.is_empty());
        let tooltip = match (title.is_empty(), snapshot.class.trim().is_empty()) {
            (true, _) => None,
            (false, true) => Some(title.to_string()),
            (false, false) => Some(format!("{title}\n{}", snapshot.class)),
        };
        self.title.set_tooltip_text(tooltip.as_deref());
    }

    /// The clock, every second. `time` and `date` are already formatted by the
    /// caller, which is the only place that knows the configured formats.
    pub fn render_clock(&self, time: &str, date: &str) {
        self.clock.render(time, date);
    }

    /// Whether the island popup is on screen, so the player pill can light up
    /// while the panel is unfolded - the pill and its window read as one object.
    pub fn set_island_open(&self, open: bool) {
        set_class(&self.media.button, "panel-open", open);
    }

    /// The same for the system popup and the status pill. Two flags, because the
    /// two panels are two processes and either can be up on its own.
    pub fn set_status_open(&self, open: bool) {
        set_class(&self.status.button, "panel-open", open);
    }

    /// And the same for the applications panel, whose flag the panel itself keeps
    /// (`state::apps_panel`) - the button that opened it wears the accent while it
    /// is up, so the two read as one object.
    pub fn set_apps_open(&self, open: bool) {
        set_class(&self.apps.button, "panel-open", open);
    }

    /// The sound level. One of the three parts of the status pill, so it is
    /// stored and the whole pill is repainted - see [`Status`].
    pub fn set_volume(&self, volume: Option<Volume>) {
        self.readings.borrow_mut().volume = volume;
        self.paint_status();
    }

    /// The wireless link. `Err` is "no wireless card (or no `iw`)" and draws
    /// nothing for this part; `Ok(None)` is a card with nothing joined.
    pub fn set_network(&self, network: Result<Option<Network>, ()>) {
        self.readings.borrow_mut().network = Some(network);
        self.paint_status();
    }

    pub fn set_battery(&self, battery: Option<Battery>) {
        self.readings.borrow_mut().battery = battery;
        self.paint_status();
    }

    /// Draw the combined pill from the last reading of each of its three parts.
    ///
    /// A part the machine does not have is simply left out - no battery in a
    /// desktop, no wireless card - and a machine with *none* of the three gets no
    /// pill at all, rather than an empty capsule that would read as a reading.
    fn paint_status(&self) {
        let status = self.readings.borrow();
        let mut parts: Vec<Part> = Vec::new();
        let mut states: Vec<(&str, bool)> = Vec::new();
        let mut tooltip: Vec<String> = Vec::new();

        match &status.network {
            // Nothing read yet, or no wireless card: this machine has nothing to
            // say about a link, so the pill says nothing about one either.
            None | Some(Err(())) => {}
            Some(Ok(None)) => {
                parts.push(Part::new(names::WIFI_OFF, "off"));
                states.push(("disconnected", true));
                tooltip.push("Not connected".to_string());
            }
            Some(Ok(Some(link))) => {
                // Four steps, as the three separate pills drew them.
                let step = match link.signal {
                    ..=25 => 0,
                    26..=50 => 1,
                    51..=75 => 2,
                    _ => 3,
                };
                parts.push(Part::new(NETWORK_STEPS[step], format!("{}%", link.signal)));
                tooltip.push(format!(
                    "{} · {} dBm · {}% signal",
                    link.ssid, link.dbm, link.signal
                ));
            }
        }

        if let Some(volume) = status.volume {
            let icon = if volume.muted || volume.percent == 0 {
                names::VOLUME_OFF
            } else if volume.percent < 50 {
                names::VOLUME_LOW
            } else {
                names::VOLUME_HIGH
            };
            parts.push(Part::new(icon, format!("{}%", volume.percent)));
            states.push(("muted", volume.muted));
            tooltip.push(if volume.muted {
                format!("Sound: {}% (muted)", volume.percent)
            } else {
                format!("Sound: {}%", volume.percent)
            });
        }

        // The battery's colour is the pill's colour: it is the one part of the
        // three that ever means "do something now".
        let (mut warning, mut critical) = (false, false);
        if let Some(battery) = status.battery {
            let icon = match battery.charging {
                Charging::Yes | Charging::Plugged => names::BATTERY_CHARGING,
                _ => {
                    let step = match battery.percent {
                        ..=10 => 0,
                        11..=40 => 1,
                        41..=70 => 2,
                        _ => 3,
                    };
                    BATTERY_STEPS[step]
                }
            };
            parts.push(Part::new(icon, format!("{}%", battery.percent)));
            warning = battery.warning();
            critical = battery.critical();
            tooltip.push(match battery.charging {
                Charging::Yes => format!("Battery: {}% - charging", battery.percent),
                Charging::Full => format!("Battery: {}% - full", battery.percent),
                Charging::Plugged => {
                    format!("Battery: {}% - plugged in, not charging", battery.percent)
                }
                Charging::No => format!("Battery: {}% on battery", battery.percent),
            });
        }

        if parts.is_empty() {
            self.status.set_visible(false);
            return;
        }
        // Colour precedence, worst news first: a muted sink or a dying battery is
        // red whichever else is also true, then the amber band, then a link that
        // is not there at all (which is amber - it is worth a look, not an
        // emergency).
        let disconnected = matches!(status.network, Some(Ok(None)));
        let muted = status.volume.is_some_and(|volume| volume.muted);
        states.push(("critical", critical || muted));
        states.push(("warning", warning || disconnected));

        self.status.set(&parts, &states);
        tooltip.push("Left click: the system panel".to_string());
        tooltip.push("Right click: wifi on/off · middle click: mute the microphone".to_string());
        tooltip.push("Scroll: the volume - and hovering shows the same panel".to_string());
        self.status.tooltip(Some(&tooltip.join("\n")));
        self.status.set_visible(self.shows.status);
    }

    pub fn render_media(&self, track: Option<&Track>, playing: bool) {
        if !self.shows.media {
            self.media.set_visible(false);
            return;
        }
        self.media.render(track, playing);
    }
}

/// What a put-away window is drawn as: the application's icon, or its first
/// letter when the theme has nothing for that class - the same stand-in the two
/// window cards fall back to, so a window looks like itself wherever this desktop
/// draws it.
fn picture(window: &Window, size: i32) -> gtk::Widget {
    if let Some(paintable) = icons::paintable(&window.class, size) {
        let image = gtk::Image::new();
        image.set_pixel_size(size);
        image.set_paintable(Some(&paintable));
        return image.upcast::<gtk::Widget>();
    }
    let letter = gtk::Label::new(Some(&text::initial(&window.class)));
    letter.add_css_class("initial");
    letter.set_size_request(size, size);
    letter.upcast::<gtk::Widget>()
}

/// Where a widget sits inside `target`, in whole pixels.
///
/// `compute_bounds` is GTK's own answer - it walks the layout, so it is right
/// even while a resize is in flight - and it is the only way to measure a widget
/// from outside the tree it lives in. What the popup needs is the status pill's
/// rectangle inside the bar's *card*: the tray and the power button sit to its
/// right, so its hot zone cannot be derived from the bar's edge, and the card is
/// the one origin both programs can agree on - the panel knows the bar's margins
/// and the monitor's position, and adds them to whatever this says.
/// Whether the event that just reached a controller came from a finger or a pen
/// rather than the pointer.
///
/// The hover handlers are not gestures, so `touch::is_touch` cannot be asked
/// here - but the event the controller is handling can. That is what keeps a
/// *laptop* session usable with a touchscreen: with the Type Cover attached the
/// machine is not a tablet (so the pill is pointer-sized and `tablet()` is
/// false), and a finger touching the player still must not be mistaken for a
/// pointer *hovering* it - the panel would open on the dwell and then stay up,
/// because the emulated pointer never leaves.
fn from_finger(controller: &impl IsA<gtk::EventController>) -> bool {
    let Some(event) = controller.current_event() else {
        return false;
    };
    if event.is_pointer_emulated() {
        return true;
    }
    event.device().is_some_and(|device| {
        matches!(
            device.source(),
            gtk::gdk::InputSource::Touchscreen | gtk::gdk::InputSource::Pen
        )
    })
}

fn bounds_in(widget: &impl IsA<gtk::Widget>, target: &impl IsA<gtk::Widget>) -> Option<[f64; 4]> {
    let bounds = widget.compute_bounds(target)?;
    Some([
        f64::from(bounds.x()),
        f64::from(bounds.y()),
        f64::from(bounds.width()),
        f64::from(bounds.height()),
    ])
}

/// Turn one CSS class on or off. GTK has no `set_css_class` in this binding, and
/// the pair is used often enough here (every pill's every state) to deserve a
/// name.
fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

/// What a workspace pill says on hover: which one it is, and whether it is worth
/// going there.
fn workspace_tooltip(workspace: &Workspace) -> String {
    let label = if workspace.name == workspace.id.to_string() {
        format!("Workspace {}", workspace.id)
    } else {
        format!("Workspace {} ({})", workspace.name, workspace.id)
    };
    let count = if workspace.windows == 1 {
        "1 window".to_string()
    } else {
        format!("{} windows", workspace.windows)
    };
    let state = if workspace.active {
        " - you are here"
    } else if workspace.visible {
        " - on another monitor"
    } else {
        ""
    };
    format!("{label}{state}\n{count}\nLeft click: focus · wheel: previous/next")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tooltip_says_which_workspace_and_how_full_it_is() {
        let workspace = |id, windows, active, visible| Workspace {
            id,
            name: id.to_string(),
            windows,
            active,
            visible,
        };
        let text = workspace_tooltip(&workspace(3, 0, false, false));
        assert!(text.starts_with("Workspace 3\n0 windows"), "{text}");
        let text = workspace_tooltip(&workspace(3, 1, true, false));
        assert!(
            text.starts_with("Workspace 3 - you are here\n1 window"),
            "{text}"
        );
        let text = workspace_tooltip(&workspace(2, 4, false, true));
        assert!(text.contains("on another monitor"), "{text}");
        // A renamed workspace still says which number it really is.
        let renamed = Workspace {
            name: "code".to_string(),
            ..workspace(7, 2, false, false)
        };
        assert!(workspace_tooltip(&renamed).starts_with("Workspace code (7)"));
    }

    #[test]
    fn a_players_mark_is_its_own_when_we_know_it() {
        assert_eq!(player_icon("vlc"), names::PLAY);
        assert_eq!(player_icon("mpv"), names::CLAPPERBOARD);
        // Unknown (and empty) players fall back to the note.
        assert_eq!(player_icon("some-new-player"), names::MUSIC);
    }

    #[test]
    fn the_glide_eases_out_and_never_overshoots_its_pill() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        // Ease-out: most of the distance is covered in the first half of the
        // time, and the rest is the settling.
        assert!(ease_out_cubic(0.5) > 0.75);
        let mut last = -1.0;
        for step in 0..=20 {
            let eased = ease_out_cubic(f64::from(step) / 20.0);
            assert!(eased >= last, "the ease went backwards at step {step}");
            assert!((0.0..=1.0).contains(&eased), "the ease left 0..=1");
            last = eased;
        }
    }

    #[test]
    fn the_capsule_travels_from_one_pill_to_the_other() {
        let from = Rect {
            x: 10.0,
            y: 5.0,
            width: 30.0,
            height: 30.0,
        };
        let to = Rect {
            x: 74.0,
            y: 5.0,
            width: 30.0,
            height: 30.0,
        };
        // The ends are the pills themselves, which is what makes the capsule
        // land on exactly the button its number names.
        assert_eq!(between(from, to, 0.0), from);
        assert_eq!(between(from, to, 1.0), to);
        let half = between(from, to, 0.5);
        assert_eq!(half.x, 42.0);
        assert_eq!(half.y, 5.0);
        assert_eq!(half.width, 30.0);
        assert_eq!(half.height, 30.0);
    }

    #[test]
    fn a_glide_is_measured_from_its_first_frame() {
        let span = Duration::from_millis(180);
        let started = 1_000_000;
        assert_eq!(progress(started, started, span), 0.0);
        assert_eq!(progress(started + 90_000, started, span), 0.5);
        assert_eq!(progress(started + 180_000, started, span), 1.0);
        // A clock that went backwards, and a frame that is late, both clamp: the
        // capsule is never drawn outside its two pills.
        assert_eq!(progress(started - 5, started, span), 0.0);
        assert_eq!(progress(started + 999_999, started, span), 1.0);
    }
}

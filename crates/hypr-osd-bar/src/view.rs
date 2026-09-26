//! The bar itself: one row, three sections, and the pills in them.
//!
//! Left: the workspaces, then the focused window's title. Centre: the clock,
//! which is also the handle for the island popup (hover it and the panel
//! unfolds; see the `hypr-osd-island` element). Right: the media pill, the
//! network, the system info pill (CPU, memory, temperature, pending updates),
//! the volume, the battery, the tray - and the power button at the far end.
//!
//! The view owns no state of its own beyond what the last render put on screen.
//! Everything it shows is handed to it by `main` (a snapshot from Hyprland, a
//! reading from `sources`, a track from MPRIS), so there is exactly one place
//! that knows how to draw each pill, and one place that decides *when* to read
//! the thing it draws.
//!
//! A `gtk::CenterBox` is what keeps the clock in the middle: its centre child is
//! centred on the *bar*, not on the space left over between the two sides, which
//! is what an ordinary box with three children would give.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use hypr_osd_core::hardware::{self as sources, Battery, Charging, Network, Volume};
use hypr_osd_core::mpris::Track;
use hypr_osd_core::system::Reading;
use hypr_osd_core::text;

use crate::hypr::{Snapshot, Workspace};
use crate::Settings;

// ---------------------------------------------------------------------------
// Glyphs
// ---------------------------------------------------------------------------

/// Workspaces: a filled circle for the one you are on, a hollow one otherwise -
/// the same pair the bar this replaces used, so the row reads the same way.
const WS_ACTIVE: &str = "\u{f111}";
const WS_IDLE: &str = "\u{f10c}";

const CLOCK: &str = "\u{f017}";
const MEDIA_NOTE: &str = "\u{f001}";
const MEDIA_PAUSE: &str = "\u{f04c}";
const POWER: &str = "\u{f011}";

/// The three system info glyphs: processor, memory, thermometer - the same ones
/// the bar this replaces used for them.
const STATS_CPU: &str = "\u{f2db}";
const STATS_MEMORY: &str = "\u{f538}";
const STATS_TEMP: &str = "\u{f2c7}";

/// What separates the parts of a pill's face. Pango's `<small>` is what makes it
/// read as a separator rather than as another reading.
const SEPARATOR: &str = " <small>·</small> ";

/// Volume, at three levels - the same three the bars and the volume card use.
const VOLUME_MUTED: &str = "\u{f026}";
const VOLUME_LOW: &str = "\u{f027}";
const VOLUME_HIGH: &str = "\u{f028}";

/// Battery, empty to full, and the charging bolt.
const BATTERY_STEPS: [&str; 5] = ["\u{f244}", "\u{f243}", "\u{f242}", "\u{f241}", "\u{f240}"];
const BATTERY_CHARGING: &str = "\u{f0e7}";

/// Signal strength, weak to strong.
const NETWORK_STEPS: [&str; 4] = ["\u{f092}", "\u{f091}", "\u{f090}", "\u{f1eb}"];
const NETWORK_OFF: &str = "\u{f127}";

/// A player's own glyph, the way the bar this replaces chose one. Unknown
/// players get the note, which is at least true.
fn player_glyph(player: &str) -> &'static str {
    match player.to_ascii_lowercase().as_str() {
        "vlc" => "\u{f04b}",
        "mpv" | "celluloid" => "\u{f03d}",
        "spotify" => "\u{f1bc}",
        "firefox" | "firefoxdeveloperedition" => "\u{f269}",
        "chromium" | "chrome" | "google-chrome" | "brave" => "\u{f268}",
        _ => MEDIA_NOTE,
    }
}

// ---------------------------------------------------------------------------
// Pills
// ---------------------------------------------------------------------------

/// One pill: a flat button with a label in it.
///
/// A button rather than a plain label so that hover, click and the
/// press-and-hold state come from GTK (and from the stylesheet's `:hover`),
/// instead of being re-invented with gesture controllers; `can_focus` is off
/// because the bar has no keyboard to walk a focus chain with.
struct Pill {
    button: gtk::Button,
    label: gtk::Label,
    /// What is on the label now, so a repaint that changes nothing does not make
    /// GTK re-measure it. The markup beside it means the label cannot be asked:
    /// `label.text()` is the *parsed* text, not what was set.
    face: RefCell<String>,
}

impl Pill {
    fn new(class: &str) -> Self {
        let button = gtk::Button::new();
        button.add_css_class("module");
        button.add_css_class(class);
        button.set_has_frame(false);
        button.set_focus_on_click(false);
        button.set_can_focus(false);
        let label = gtk::Label::new(None);
        button.set_child(Some(&label));
        Pill {
            button,
            label,
            face: RefCell::new(String::new()),
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

    /// Show a different face without rebuilding the widget.
    fn set(&self, text: &str, states: &[(&str, bool)]) {
        self.paint(text, states, false);
    }

    /// The same, with Pango markup - which is how the system info pill keeps its
    /// hierarchy (a bold update count, small separators) without hard-coding a
    /// colour, exactly as the bar this replaces did.
    fn set_markup(&self, markup: &str, states: &[(&str, bool)]) {
        self.paint(markup, states, true);
    }

    fn paint(&self, face: &str, states: &[(&str, bool)], markup: bool) {
        {
            let mut current = self.face.borrow_mut();
            if *current != face {
                if markup {
                    self.label.set_markup(face);
                } else {
                    self.label.set_text(face);
                }
                *current = face.to_string();
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
// The bar
// ---------------------------------------------------------------------------

pub struct BarView {
    /// The output this bar is on (`eDP-1`, `DP-2`, ...). The bars are held under
    /// it by `main`, and it is what the bar tells its two popups: they are drawn
    /// on this screen and measured against it.
    connector: String,
    pub root: gtk::CenterBox,
    workspaces: gtk::Box,
    title: gtk::Label,
    clock: Pill,
    media: Pill,
    /// The system info pill: what the machine is *doing*.
    stats: Pill,
    /// The combined status pill: the link, the sound and the battery, and the
    /// handle the system popup unfolds from.
    status: Pill,
    tray: gtk::Box,
    power: Pill,
    /// The three readings behind the status pill, kept together because the pill
    /// is drawn from all of them at once (see [`Status`]).
    readings: RefCell<Status>,
    /// What the workspace row was last drawn from, so an event that changes
    /// nothing does not rebuild a row of buttons.
    last_workspaces: RefCell<String>,
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
    /// knows which screen that is.
    pub fn new(settings: &Rc<Settings>, connector: &str) -> Self {
        // --- left: the workspaces and the focused window -------------------
        let workspaces = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        workspaces.add_css_class("workspaces");

        let title = gtk::Label::new(None);
        title.add_css_class("title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_xalign(0.0);
        // An ellipsised label still asks for its full width, so the title is
        // capped in characters; without this one long title would decide how
        // much room the right half of the bar gets.
        text::cap_width(&title, settings.title_width);

        let left = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        left.add_css_class("section");
        left.append(&workspaces);
        left.append(&title);

        // --- centre: the clock, and the island popup's handle ---------------
        let clock = Pill::new("clock");
        // Hovering the clock unfolds the island popup. The bar only *asks*: the
        // island decides when to go away again, because leaving a popup is the
        // pointer moving somewhere the bar cannot see (onto the popup itself).
        let motion = gtk::EventControllerMotion::new();
        {
            let island = settings.island_command.clone();
            let connector = connector.to_owned();
            motion.connect_enter(move |_, _, _| {
                // The island's hot zone is the middle of the bar, so the panel
                // only needs to be told *which* bar - i.e. which monitor. It
                // cannot look that up itself: the focused output is not
                // necessarily the one this bar is on.
                sources::launch(&island, &["open", &connector]);
            });
        }
        clock.button.add_controller(motion);
        {
            let notifications = settings.notifications_command.clone();
            clock
                .on_click(move || {
                    sources::launch(&notifications, &["-t", "-sw"]);
                })
                .on_button(gtk::gdk::BUTTON_SECONDARY, {
                    let notifications = settings.notifications_command.clone();
                    move || {
                        sources::launch(&notifications, &["-d", "-sw"]);
                    }
                });
        }

        // --- right: media, tray, status, power -----------------------------
        let media = Pill::new("media");
        media
            .on_click(move || {
                sources::launch("playerctl", &["play-pause"]);
            })
            .on_button(gtk::gdk::BUTTON_SECONDARY, || {
                sources::launch("playerctl", &["next"]);
            })
            .on_button(gtk::gdk::BUTTON_MIDDLE, || {
                sources::launch("playerctl", &["previous"]);
            })
            .on_scroll(|direction| {
                // Scrolling seeks: `5+` is "five seconds further in".
                let spec = if direction > 0 { "5+" } else { "5-" };
                sources::launch("playerctl", &["position", spec]);
            });

        let tray = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        tray.add_css_class("tray");

        // --- the combined status pill --------------------------------------
        //
        // One pill for the three things about the machine you glance at - the
        // wireless link, the sound, the battery - where there used to be three.
        // They belong together because they are one question ("is this machine
        // alright?") and because they are all *handles*: this pill unfolds the
        // system popup, which is where the detail lives now.
        let status = Pill::new("status");
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
                // The wireless radio on/off, which the network pill had here.
                // The *element* owns that switch (it reads `rfkill` and knows
                // what it means), so the pill asks it rather than guessing with
                // `nmcli`.
                .on_button(gtk::gdk::BUTTON_SECONDARY, {
                    let stats = stats.clone();
                    move || {
                        sources::launch(&stats, &["wifi"]);
                    }
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
        }

        // The system info pill sits to the left of the status pill: what the
        // machine is doing first, then what it is connected to and holding.
        let stats = Pill::new("stats");
        if !settings.updates_command.is_empty() {
            // The count is the one part of this pill with something behind it, and
            // the old pill's tooltip promised exactly this: the full list, in a
            // terminal (where it can be read at leisure and scrolls).
            let terminal = settings.terminal_command.clone();
            let command = settings.updates_command.clone();
            stats.on_click(move || {
                sources::launch(
                    &terminal,
                    &[
                        "-e",
                        "sh",
                        "-c",
                        &format!("{command}; printf '\\n-- press enter to close --\\n'; read _"),
                    ],
                );
            });
        }

        let power = Pill::new("power");
        {
            let session = settings.session_command.clone();
            power.on_click(move || {
                sources::launch(&session, &["toggle"]);
            });
        }

        let right = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        right.add_css_class("section");
        for pill in [&media, &stats, &status] {
            right.append(&pill.button);
        }
        right.append(&tray);
        // The power button sits at the far right, where a mis-aimed click is
        // least likely to land on it.
        right.append(&power.button);

        // --- the row -------------------------------------------------------
        let root = gtk::CenterBox::new();
        root.set_start_widget(Some(&left));
        root.set_center_widget(Some(&clock.button));
        root.set_end_widget(Some(&right));

        // The two handles that unfold the system popup: the pointer arriving on
        // the status pill, and a left click on it. Both send the *same* thing,
        // and that thing is only ever `open` - the request to start watching,
        // which is all the bar ever says to either popup (the clock's hover sends
        // the island the same verb). The panel then decides for itself when it has
        // had enough of the pointer, exactly as the island does: the bar asks, the
        // panel disposes.
        //
        // Wired here rather than with the pill's other handlers, because the
        // request has to *measure* the pill - for which it needs the widget tree
        // the pill lives in.
        {
            let stats = settings.stats_command.clone();
            let pill = status.button.clone();
            let root = root.clone();
            let connector = connector.to_owned();
            // What the click used to run instead is worth knowing about, because
            // it is the bug this shape exists to prevent: `toggle`, which puts the
            // panel into the machine's *pinned* state - and a pinned panel is not
            // watched at all, so a single click on the pill left it up for good,
            // glowing, with the pointer long gone. Nothing but another click (or
            // `hypr-osd-stats close`) could take it away. The island has no such
            // state, and the bar must not be able to put either panel into one.
            let ask = Rc::new(move || {
                // Two things the panel cannot work out for itself, so the bar
                // says them: which monitor the bar is on (the panel is drawn on
                // that screen and measured against it, and it has no way of
                // knowing which of the bars the pointer came from), and where the
                // pill is. The tray and the power button sit to the pill's right,
                // so its hot zone cannot be derived from the bar's edge either -
                // and the rectangle below is measured inside the bar's card,
                // because the panel knows the bar's margins and the monitor's
                // position but not this.
                let mut args: Vec<String> = vec!["open".to_string(), connector.clone()];
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
                motion.connect_enter(move |_, _, _| ask());
                status.button.add_controller(motion);
            }
            status.on_click(move || ask());
        }

        let this = BarView {
            connector: connector.to_owned(),
            root,
            workspaces,
            title,
            clock,
            media,
            stats,
            status,
            tray,
            power,
            readings: RefCell::new(Status::default()),
            last_workspaces: RefCell::new(String::new()),
        };
        // A first paint, so the bar that appears is already correct rather than
        // appearing empty and filling in over the next second.
        this.power.set(POWER, &[]);
        this.power
            .tooltip(Some("Session: lock, suspend, log out, reboot, shut down"));
        this.media.set_visible(false);
        // The system info pill has nothing to show until the second CPU sample
        // a second from now; it is filled in by the first real reading.
        this.stats.set_visible(false);
        // And the status pill waits for the first readings of all three of its
        // parts, which is the first `refresh`.
        this.status.set_visible(false);
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

    fn render_workspaces(&self, workspaces: &[Workspace]) {
        // Rebuilding the row on every event would replace buttons under the
        // pointer (and lose a hover); the signature is what says whether
        // anything visible actually changed.
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
            let mut last = self.last_workspaces.borrow_mut();
            if *last == signature {
                return;
            }
            *last = signature;
        }

        while let Some(child) = self.workspaces.first_child() {
            self.workspaces.remove(&child);
        }
        for workspace in workspaces {
            let pill = Pill::new("ws");
            pill.set(
                if workspace.active { WS_ACTIVE } else { WS_IDLE },
                &[
                    ("active", workspace.active),
                    ("visible", workspace.visible),
                    ("occupied", workspace.windows > 0),
                    ("empty", workspace.windows == 0),
                ],
            );
            pill.tooltip(Some(&workspace_tooltip(workspace)));
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
            self.workspaces.append(&pill.button);
        }
    }

    fn render_title(&self, snapshot: &Snapshot) {
        let title = snapshot.title.trim();
        self.title.set_text(title);
        // An empty title takes the label (and its gap) out of the row rather
        // than leaving a hole where a window title would be.
        self.title.set_visible(!title.is_empty());
        let tooltip = match (title.is_empty(), snapshot.class.trim().is_empty()) {
            (true, _) => None,
            (false, true) => Some(title.to_string()),
            (false, false) => Some(format!("{title}\n{}", snapshot.class)),
        };
        self.title.set_tooltip_text(tooltip.as_deref());
    }

    /// The clock, every second. `time` and `date` are already formatted by the
    /// caller, which is the only place that knows the configured formats.
    pub fn render_clock(&self, time: &str) {
        self.clock.set(&format!("{CLOCK} {time}"), &[]);
        self.clock.tooltip(Some(time));
    }

    /// Whether the island popup is on screen, so the clock can light up while
    /// the panel is unfolded - the same feedback the pill and its window shared
    /// before.
    pub fn set_island_open(&self, open: bool) {
        set_class(&self.clock.button, "panel-open", open);
    }

    /// The same for the system popup and the status pill. Two flags, because the
    /// two panels are two processes and either can be up on its own.
    pub fn set_status_open(&self, open: bool) {
        set_class(&self.status.button, "panel-open", open);
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
        let mut parts: Vec<String> = Vec::new();
        let mut states: Vec<(&str, bool)> = Vec::new();
        let mut tooltip: Vec<String> = Vec::new();

        match &status.network {
            // Nothing read yet, or no wireless card: this machine has nothing to
            // say about a link, so the pill says nothing about one either.
            None | Some(Err(())) => {}
            Some(Ok(None)) => {
                parts.push(format!("{NETWORK_OFF} off"));
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
                parts.push(format!("{} {}%", NETWORK_STEPS[step], link.signal));
                tooltip.push(format!(
                    "{} · {} dBm · {}% signal",
                    link.ssid, link.dbm, link.signal
                ));
            }
        }

        if let Some(volume) = status.volume {
            let glyph = if volume.muted || volume.percent == 0 {
                VOLUME_MUTED
            } else if volume.percent < 50 {
                VOLUME_LOW
            } else {
                VOLUME_HIGH
            };
            parts.push(format!("{glyph} {}%", volume.percent));
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
            let glyph = match battery.charging {
                Charging::Yes | Charging::Plugged => BATTERY_CHARGING,
                _ => {
                    let step = match battery.percent {
                        ..=10 => 0,
                        11..=35 => 1,
                        36..=60 => 2,
                        61..=85 => 3,
                        _ => 4,
                    };
                    BATTERY_STEPS[step]
                }
            };
            parts.push(format!("{glyph} {}%", battery.percent));
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

        self.status.set_markup(&parts.join(SEPARATOR), &states);
        tooltip.push("Left click: the system panel".to_string());
        tooltip.push("Right click: wifi on/off · middle click: mute the microphone".to_string());
        tooltip.push("Scroll: the volume - and hovering shows the same panel".to_string());
        self.status.tooltip(Some(&tooltip.join("\n")));
        self.status.set_visible(true);
    }

    pub fn render_media(&self, track: Option<&Track>, playing: bool) {
        let Some(track) = track.filter(|track| !track.is_empty()) else {
            self.media.set_visible(false);
            return;
        };
        let glyph = if playing {
            player_glyph(&track.player)
        } else {
            MEDIA_PAUSE
        };
        // The artist is dropped before the title is: on a bar, "what is this" is
        // more useful than "who is this".
        let summary = if track.artist.is_empty() {
            track.title.clone()
        } else {
            format!("{} · {}", track.title, track.artist)
        };
        self.media
            .set(&format!("{glyph} {summary}"), &[("playing", playing)]);
        self.media.tooltip(Some(&format!(
            "{}\n{}",
            summary,
            if playing { "playing" } else { "paused" }
        )));
        self.media.set_visible(true);
    }

    /// The system info pill: CPU, memory, temperature, and how many updates are
    /// waiting.
    ///
    /// The readings and the update count are one pill on purpose (that is how the
    /// bar this replaces had them): a machine that is busy *and* has 40 updates
    /// waiting is one story about the machine, and the state colour is the same
    /// answer either way - amber for "have a look", red for "look now".
    pub fn render_stats(&self, reading: &Reading, updates: i32) {
        let mut parts = vec![format!("{STATS_CPU} {:.0}%", reading.cpu)];
        parts.push(format!("{STATS_MEMORY} {:.0}%", reading.memory));
        // A machine with no such sensor says nothing rather than 0°.
        if let Some(temperature) = reading.temperature {
            parts.push(format!("{STATS_TEMP} {temperature:.0}°"));
        }
        let mut face = parts.join(SEPARATOR);
        if updates > 0 {
            // Bold, the way the update count stood out before: it is the one
            // number here that is a *task* rather than a reading.
            face.push_str(&format!("{SEPARATOR}<b>{updates}</b>"));
        }
        self.stats.set_markup(
            &face,
            &[
                ("warning", reading.warning()),
                ("critical", reading.critical()),
                ("updates", updates > 0),
            ],
        );

        let mut tooltip = vec![format!("CPU: {:.0}%", reading.cpu)];
        tooltip.push(format!("Memory: {:.0}%", reading.memory));
        tooltip.push(match reading.temperature {
            Some(temperature) => format!("Temperature: {temperature:.0}°C"),
            None => "Temperature: no sensor".to_string(),
        });
        tooltip.push(match updates {
            0 => "No updates pending".to_string(),
            1 => "1 pending update".to_string(),
            count => format!("{count} pending updates"),
        });
        tooltip.push("Left click: the list, in a terminal".to_string());
        self.stats.tooltip(Some(&tooltip.join("\n")));
        self.stats.set_visible(true);
    }
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
    fn a_players_glyph_is_its_own_when_we_know_it() {
        assert_eq!(player_glyph("vlc"), "\u{f04b}");
        assert_eq!(player_glyph("mpv"), "\u{f03d}");
        // Unknown (and empty) players fall back to the note.
        assert_eq!(player_glyph("some-new-player"), MEDIA_NOTE);
    }
}

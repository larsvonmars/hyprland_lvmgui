//! The system popup: what the machine is doing, and the controls for it.
//!
//! Two columns of tiles in the card that unfolds from the bar's status pill:
//!
//! ```text
//!  ┌───────────────────────────────────────────────┐
//!  │                                        ▬▬       │
//!  │ SYSTEM               [15 updates] [bluetooth on]│
//!  │ ─────────────────────────────────────────────   │
//!  │ ┌─ resources ──────┐ ┌─ controls ────────────┐ │
//!  │ │  CPU ▂▂▂▂▂   11% │ │ BLUETOOTH On · 1 conn │ │
//!  │ │  MEMORY ▂▂▂▂  63%│ │ [Open] [Turn off]     │ │
//!  │ │  TEMP ▂▂▂▂   52°C│ │ NETWORK   home-wifi   │ │
//!  │ │  [btop]          │ │ [Open] [Wi-Fi off]    │ │
//!  │ └──────────────────┘ │ SOUND  42%            │ │
//!  │ ┌─ updates ────────┐ │ [Mixer] [Mute]        │ │
//!  │ │ 15 pending       │ │ POWER      [balanced] │ │
//!  │ │ base              │ │ PRESENTATION   [off]  │ │
//!  │ │ linux             │ │ ☀ ▁▁▁▁▁▁▁▁▁▁▁▁▁▁ 100% │ │
//!  │ │ + 13 more        │ │ LAYOUT            DE  │ │
//!  │ │ [Install]        │ │                       │ │
//!  │ └──────────────────┘ └───────────────────────┘ │
//!  └───────────────────────────────────────────────┘
//! ```
//!
//! The view paints what it is handed and runs the commands its buttons mean. It
//! does not read anything itself: `main` owns the readings and the clock, so
//! there is one place that decides *when* to read and one that decides how to
//! draw. The one piece of state it does keep is what the last reading said -
//! because a button like "Turn off" has to know whether bluetooth is on to say
//! what it does.
//!
//! The card itself - the fill, the border, the shadow, the 16px radius - is the
//! shell's (`box.card` in `base.css`), and the chips, tiles and grip are the
//! collection's shared recipes; what is here is only what goes inside one.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use hypr_osd_core::hardware::{self, Network, Volume};
use hypr_osd_core::system::{Reading, TEMP_CRIT, TEMP_WARN};

use crate::sources::{self, Bluetooth, Inhibitor};
use crate::Settings;

// ---------------------------------------------------------------------------
// Glyphs
// ---------------------------------------------------------------------------

const CPU: &str = "\u{f2db}";
const MEMORY: &str = "\u{f538}";
const TEMPERATURE: &str = "\u{f2c7}";
const UPDATES: &str = "\u{f019}";
const CONTROLS: &str = "\u{f1de}";
const BLUETOOTH: &str = "\u{f293}";
const NETWORK: &str = "\u{f1eb}";
const VOLUME: &str = "\u{f028}";
const POWER: &str = "\u{f0e7}";
const PRESENTATION: &str = "\u{f03d}";
const BRIGHTNESS: &str = "\u{f185}";
const LAYOUT: &str = "\u{f11c}";

/// What the last reading said - what the buttons act on.
///
/// A toggle button has to know the state to toggle *from* ("Turn off" vs "Turn
/// on"), and asking the system again at click time would mean a blocking read in
/// the click path. The reading is at most a few seconds old, and every one of
/// these toggles is a no-op in the worst case.
#[derive(Default)]
struct Last {
    bluetooth: Bluetooth,
    /// Whether the wireless radio is on at all - not the same thing as being
    /// connected to something.
    radio: bool,
    /// Whether the machine has a wireless card in the first place. With none,
    /// the row and the chip about it have nothing to say.
    present: bool,
    profile: Option<String>,
    volume: Option<Volume>,
    updates: i32,
}

pub struct StatsView {
    /// The card's content: the shell's card widget is its parent.
    pub root: gtk::Box,
    /// The state chips in the header, rebuilt whenever something changes them.
    chips: gtk::Box,
    bars: [gtk::ProgressBar; 3],
    values: [gtk::Label; 3],
    updates_state: gtk::Label,
    updates_list: gtk::Label,
    install: gtk::Button,
    bluetooth_state: gtk::Label,
    bluetooth_toggle: gtk::Button,
    network_state: gtk::Label,
    radio_toggle: gtk::Button,
    sound_state: gtk::Label,
    mute_toggle: gtk::Button,
    profile: gtk::Button,
    presentation: gtk::Button,
    brightness_scale: gtk::Scale,
    brightness_value: gtk::Label,
    layout_value: gtk::Label,
    last: RefCell<Last>,
    inhibitor: RefCell<Inhibitor>,
    /// Set while the brightness slider is being moved *by the program*: a
    /// `set_value` during a refresh is indistinguishable from a drag at the
    /// signal, and would write the backlight back to where it already is.
    scale_guard: Cell<bool>,
    /// What to run shortly after an action, so the row shows the new state
    /// rather than the old one until the next heartbeat. `main` installs it.
    on_refresh: RefCell<Option<Rc<dyn Fn()>>>,
    /// This view, weakly: the buttons are wired after it exists, and every one
    /// of them needs to reach back into it (see `wire`).
    me: Weak<StatsView>,
}

impl StatsView {
    pub fn new(settings: &Rc<Settings>) -> Rc<StatsView> {
        // ---- header ---------------------------------------------------------
        let title = gtk::Label::new(Some("SYSTEM"));
        title.add_css_class("popup-title");
        title.set_xalign(0.0);
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chips.add_css_class("chips");
        chips.set_valign(gtk::Align::Center);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        header.add_css_class("popup-header");
        header.append(&title);
        header.append(&chips);

        // ---- resources ------------------------------------------------------
        let resources = tile("resources", CPU);
        let mut bars = Vec::new();
        let mut values = Vec::new();
        for (glyph, name) in [(CPU, "CPU"), (MEMORY, "MEMORY"), (TEMPERATURE, "TEMP")] {
            let (row, bar, value) = resource_row(glyph, name);
            resources.append(&row);
            bars.push(bar);
            values.push(value);
        }
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.set_halign(gtk::Align::Center);
        let btop = chip("btop", {
            let terminal = settings.terminal_command.clone();
            move || sources::spawn(&terminal, &["-e", "btop"])
        });
        btop.set_tooltip_text(Some("The full picture, in btop"));
        actions.append(&btop);
        resources.append(&actions);

        // ---- updates --------------------------------------------------------
        let updates = tile("updates", UPDATES);
        let updates_state = gtk::Label::new(Some("System up to date"));
        updates_state.add_css_class("sub");
        updates_state.set_xalign(0.0);
        // A few package names, so the count is a thing rather than a number.
        let updates_list = gtk::Label::new(None);
        updates_list.add_css_class("upd-list");
        updates_list.set_xalign(0.0);
        updates_list.set_visible(false);
        let install_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        install_actions.set_halign(gtk::Align::Center);
        let install = chip("Install", {
            let terminal = settings.terminal_command.clone();
            move || sources::spawn(&terminal, &["-e", "sudo", "pacman", "-Syu"])
        });
        install.set_tooltip_text(Some("Update now, in a terminal"));
        install_actions.append(&install);
        updates.append(&updates_state);
        updates.append(&updates_list);
        updates.append(&install_actions);

        let left = gtk::Box::new(gtk::Orientation::Vertical, 8);
        left.add_css_class("column");
        left.set_size_request(settings.left_width, -1);
        left.append(&resources);
        left.append(&updates);

        // ---- controls -------------------------------------------------------
        let controls = tile("controls", CONTROLS);

        // BLUETOOTH: the state on the value side, its two actions under it -
        // the one control that needs a second row.
        let bluetooth_state = gtk::Label::new(Some("Off"));
        bluetooth_state.add_css_class("value");
        bluetooth_state.set_hexpand(true);
        bluetooth_state.set_xalign(1.0);
        controls.append(&control_row(BLUETOOTH, "BLUETOOTH", &bluetooth_state));
        let bluetooth_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let bluetooth_open = chip("Open", {
            let terminal = settings.terminal_command.clone();
            move || sources::spawn(&terminal, &["-e", "bluetoothctl"])
        });
        let bluetooth_toggle = chip("Turn on", || {});
        bluetooth_actions.append(&bluetooth_open);
        bluetooth_actions.append(&bluetooth_toggle);
        controls.append(&bluetooth_actions);

        // NETWORK: which access point, and the two things you might want to do
        // about it. The pill's own wifi toggle lives here too - this is where
        // managing the link belongs now that the pill is one handle.
        let network_state = gtk::Label::new(Some("Not connected"));
        network_state.add_css_class("value");
        network_state.set_hexpand(true);
        network_state.set_xalign(1.0);
        network_state.set_ellipsize(gtk::pango::EllipsizeMode::End);
        network_state.set_max_width_chars(14);
        controls.append(&control_row(NETWORK, "NETWORK", &network_state));
        let network_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let network_open = chip("Open", {
            let terminal = settings.terminal_command.clone();
            move || sources::spawn(&terminal, &["-e", "nmtui"])
        });
        let radio_toggle = chip("Wi-Fi off", || {});
        network_actions.append(&network_open);
        network_actions.append(&radio_toggle);
        controls.append(&network_actions);

        // SOUND: the sink's level, and the two things the volume pill used to
        // do with a right and a middle click.
        let sound_state = gtk::Label::new(Some("—"));
        sound_state.add_css_class("value");
        sound_state.set_hexpand(true);
        sound_state.set_xalign(1.0);
        controls.append(&control_row(VOLUME, "SOUND", &sound_state));
        let sound_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let mixer = chip("Mixer", || sources::spawn("pavucontrol", &[]));
        mixer.set_tooltip_text(Some("Per-application levels, in pavucontrol"));
        let mute_toggle = chip("Mute", || {});
        sound_actions.append(&mixer);
        sound_actions.append(&mute_toggle);
        controls.append(&sound_actions);

        // POWER: the chip *is* the value, and clicking it cycles the profile.
        let profile = chip("—", || {});
        profile.set_tooltip_text(Some("Click to cycle: power-saver → balanced → performance"));
        profile.set_hexpand(true);
        profile.set_halign(gtk::Align::End);
        controls.append(&control_row(POWER, "POWER", &profile));

        // PRESENTATION: hold an idle/sleep block for as long as it is on.
        let presentation = chip("off", || {});
        presentation.set_tooltip_text(Some(
            "Hold a systemd inhibitor (idle and sleep) - what the old \
             idle_inhibitor module did, with a switch that goes away when this \
             element does",
        ));
        presentation.set_hexpand(true);
        presentation.set_halign(gtk::Align::End);
        controls.append(&control_row(PRESENTATION, "PRESENTATION", &presentation));

        // BRIGHTNESS: glyph, slider, percent.
        let brightness_value = gtk::Label::new(Some("—"));
        brightness_value.add_css_class("value");
        brightness_value.set_xalign(1.0);
        brightness_value.set_size_request(38, -1);
        let sun = gtk::Label::new(Some(BRIGHTNESS));
        sun.add_css_class("section-glyph");
        let brightness_scale =
            gtk::Scale::with_range(gtk::Orientation::Horizontal, 1.0, 100.0, 1.0);
        brightness_scale.set_draw_value(false);
        brightness_scale.set_size_request(80, -1);
        brightness_scale.set_hexpand(true);
        brightness_scale.set_valign(gtk::Align::Center);
        let brightness_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        brightness_row.add_css_class("bright-row");
        brightness_row.append(&sun);
        brightness_row.append(&brightness_scale);
        brightness_row.append(&brightness_value);
        controls.append(&brightness_row);

        // LAYOUT: the bare code on the value side.
        let layout_value = gtk::Label::new(Some("—"));
        layout_value.add_css_class("value");
        layout_value.set_hexpand(true);
        layout_value.set_xalign(1.0);
        controls.append(&control_row(LAYOUT, "LAYOUT", &layout_value));

        let right = gtk::Box::new(gtk::Orientation::Vertical, 8);
        right.add_css_class("column");
        right.set_size_request(settings.right_width, -1);
        right.append(&controls);

        // ---- the panel ------------------------------------------------------
        // The little capsule in the corner: the same grip every drawer in this
        // theme wears, at the *right* end because that is the end of the bar
        // this unfolded from.
        let handle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        handle.add_css_class("handle");
        handle.set_halign(gtk::Align::End);
        handle.set_size_request(46, 4);

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        body.add_css_class("body");
        body.append(&left);
        body.append(&right);

        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.add_css_class("sep");

        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("stats");
        root.append(&handle);
        root.append(&header);
        root.append(&separator);
        root.append(&body);

        let view = Rc::new_cyclic(|me| StatsView {
            root,
            chips,
            bars: bars.try_into().expect("three resource rows"),
            values: values.try_into().expect("three resource rows"),
            updates_state,
            updates_list,
            install,
            bluetooth_state,
            bluetooth_toggle,
            network_state,
            radio_toggle,
            sound_state,
            mute_toggle,
            profile,
            presentation,
            brightness_scale,
            brightness_value,
            layout_value,
            last: RefCell::new(Last::default()),
            inhibitor: RefCell::new(Inhibitor::default()),
            scale_guard: Cell::new(true),
            on_refresh: RefCell::new(None),
            me: me.clone(),
        });

        // The three buttons that only open a window have nothing to decide, so
        // they are wired here; everything else goes through `wire`, which reads
        // the last reading to know what it is toggling.
        for button in [&bluetooth_open, &network_open, &mixer] {
            let view = view.clone();
            button.connect_clicked(move |_| view.refresh_soon(600));
        }
        view.wire();
        view
    }

    /// Install what to run shortly after an action changed something. `main`
    /// owns the readings, so only `main` can re-read them.
    pub fn on_refresh(&self, hook: Rc<dyn Fn()>) {
        *self.on_refresh.borrow_mut() = Some(hook);
    }

    /// Ask for a re-read soon - the daemon behind a button (bluetooth,
    /// brightness, the power profile) needs a moment to catch up with it.
    fn refresh_soon(&self, delay_ms: u64) {
        let Some(hook) = self.on_refresh.borrow().clone() else {
            return;
        };
        glib::timeout_add_local_once(Duration::from_millis(delay_ms), move || hook());
    }

    /// Wire every button that has a state to read.
    fn wire(self: &Rc<Self>) {
        connect_chip(&self.bluetooth_toggle, self.me(), move |view| {
            sources::set_bluetooth(!view.last.borrow().bluetooth.powered);
            view.refresh_soon(600);
        });

        connect_chip(&self.radio_toggle, self.me(), move |view| {
            sources::set_radio(!view.last.borrow().radio);
            view.refresh_soon(600);
        });

        connect_chip(&self.mute_toggle, self.me(), move |view| {
            // Straight at the sink: the *volume* element owns the step, not the
            // mute, and a mute is one command either way.
            hardware::run("wpctl", &["set-mute", hardware::SINK, "toggle"]);
            view.refresh_soon(300);
        });

        connect_chip(&self.profile, self.me(), move |view| {
            let current = view.last.borrow().profile.clone();
            sources::set_power_profile(sources::next_profile(current.as_deref()));
            view.refresh_soon(600);
        });

        connect_chip(&self.presentation, self.me(), move |view| {
            view.inhibitor.borrow_mut().toggle();
            // Ours to report: nothing outside this process knows about the
            // switch, so it does not have to wait for a re-read.
            let active = view.inhibitor.borrow().active();
            view.render_presentation(active);
            view.render_chips();
            view.refresh_soon(300);
        });

        // The install chip's own work is in its constructor (a terminal); what
        // is left is the re-read, because the count changes once it is done.
        connect_chip(&self.install, self.me(), move |view| {
            view.refresh_soon(1500)
        });

        // The slider writes the backlight as it is dragged. Every write goes
        // through `brightnessctl`, which is exactly why the guard exists: a
        // refresh pushing the current value into the slider must not bounce back
        // out as a write of the value it already has.
        let view = self.clone();
        self.brightness_scale.connect_value_changed(move |scale| {
            if view.scale_guard.get() {
                return;
            }
            let percent = (scale.value().round() as i32).clamp(1, 100);
            view.brightness_value.set_text(&format!("{percent}%"));
            sources::set_brightness(percent);
            view.refresh_soon(400);
        });
    }

    /// This view as an `Rc`, for the callbacks that have to hold on to it. The
    /// buttons live inside the view they act on, so they reach it through the
    /// weak reference it was built with.
    fn me(&self) -> Rc<StatsView> {
        self.me
            .upgrade()
            .expect("the view outlives its own buttons")
    }

    // ---- rendering ------------------------------------------------------

    /// CPU, memory and temperature: three bars, three values.
    pub fn render_resources(&self, reading: &Reading) {
        let rows = [
            (0, reading.cpu, format!("{:.0}%", reading.cpu), false, false),
            (
                1,
                reading.memory,
                format!("{:.0}%", reading.memory),
                false,
                false,
            ),
            (
                2,
                reading.temperature.unwrap_or(0.0),
                match reading.temperature {
                    Some(temperature) => format!("{temperature:.0}°C"),
                    // A machine with no such sensor says so rather than showing
                    // a 0°C that would look like a reading.
                    None => "—".to_string(),
                },
                reading.temperature.is_some_and(|t| t >= TEMP_WARN),
                reading.temperature.is_some_and(|t| t >= TEMP_CRIT),
            ),
        ];
        for (index, fraction, text, warn, crit) in rows {
            self.bars[index].set_fraction(f64::from(fraction / 100.0).clamp(0.0, 1.0));
            self.values[index].set_text(&text);
            set_class(&self.values[index], "warn", warn);
            set_class(&self.values[index], "crit", crit);
        }
    }

    /// The pending updates: the count, a few names, and whether installing is
    /// worth offering.
    pub fn render_updates(&self, count: i32, names: &[String]) {
        self.last.borrow_mut().updates = count;
        if count == 0 {
            self.updates_state.set_text("System up to date");
            self.updates_list.set_visible(false);
        } else {
            self.updates_state.set_text(&match count {
                1 => "1 package waiting".to_string(),
                count => format!("{count} packages waiting"),
            });
            // Four fit the tile; the rest is a number, which is what the count
            // above already says.
            let mut text = names.iter().take(4).cloned().collect::<Vec<_>>().join("\n");
            if names.len() > 4 {
                text.push_str(&format!("\n+ {} more", names.len() - 4));
            }
            self.updates_list.set_text(&text);
            self.updates_list.set_visible(!names.is_empty());
        }
        self.install.set_sensitive(count > 0);
    }

    pub fn render_bluetooth(&self, bluetooth: Bluetooth) {
        self.last.borrow_mut().bluetooth = bluetooth;
        self.bluetooth_state.set_text(&bluetooth.state());
        label_of(&self.bluetooth_toggle).set_text(bluetooth.action());
    }

    /// The wireless link and the radio behind it. `present` is whether the
    /// machine has a wireless interface at all - with none, the row says exactly
    /// that rather than a permanent "off" on hardware that is not there.
    pub fn render_network(&self, network: Option<&Network>, present: bool, radio: bool) {
        {
            let mut last = self.last.borrow_mut();
            last.radio = radio;
            last.present = present;
        }
        label_of(&self.radio_toggle).set_text(if radio { "Wi-Fi off" } else { "Wi-Fi on" });
        if !present {
            self.network_state.set_text("no wireless card");
            self.radio_toggle.set_sensitive(false);
            return;
        }
        self.radio_toggle.set_sensitive(true);
        self.network_state.set_text(&match (radio, network) {
            (false, _) => "radio off".to_string(),
            (true, Some(link)) => link.ssid.clone(),
            (true, None) => "Not connected".to_string(),
        });
        let tooltip = match (radio, network) {
            (false, _) => Some("The wifi radio is switched off".to_string()),
            (true, Some(link)) => Some(format!(
                "{} · {} dBm · {}%",
                link.ssid, link.dbm, link.signal
            )),
            (true, None) => Some("Nothing joined".to_string()),
        };
        self.network_state.set_tooltip_text(tooltip.as_deref());
    }

    pub fn render_sound(&self, volume: Option<Volume>) {
        self.last.borrow_mut().volume = volume;
        match volume {
            Some(volume) => {
                self.sound_state.set_text(&format!("{}%", volume.percent));
                set_class(&self.sound_state, "warn", volume.muted);
                label_of(&self.mute_toggle).set_text(if volume.muted { "Unmute" } else { "Mute" });
                self.mute_toggle.set_sensitive(true);
            }
            // No sink to talk to: the row says so instead of offering a button
            // that would do nothing.
            None => {
                self.sound_state.set_text("no sink");
                self.mute_toggle.set_sensitive(false);
            }
        }
    }

    pub fn render_profile(&self, profile: Option<&str>) {
        self.last.borrow_mut().profile = profile.map(str::to_owned);
        match profile {
            Some(profile) => {
                label_of(&self.profile).set_text(profile);
                self.profile.set_sensitive(true);
            }
            // A machine without the daemon: the profile is not "unknown", it is
            // unavailable, and the button says so and stops taking clicks.
            None => {
                label_of(&self.profile).set_text("n/a");
                self.profile.set_sensitive(false);
            }
        }
    }

    pub fn render_presentation(&self, on: bool) {
        label_of(&self.presentation).set_text(if on { "on" } else { "off" });
        set_class(&self.presentation, "on", on);
    }

    /// The slider and its percentage. `None` on a machine whose backlight cannot
    /// be read: the row goes away, because a slider that does nothing is worse
    /// than no slider.
    pub fn render_brightness(&self, percent: Option<i32>) {
        let Some(percent) = percent else {
            self.brightness_scale.set_visible(false);
            self.brightness_value.set_visible(false);
            return;
        };
        self.brightness_scale.set_visible(true);
        self.brightness_value.set_visible(true);
        let percent = percent.clamp(1, 100);
        self.scale_guard.set(true);
        self.brightness_scale.set_value(f64::from(percent));
        self.scale_guard.set(false);
        self.brightness_value.set_text(&format!("{percent}%"));
    }

    pub fn render_layout(&self, code: &str) {
        self.layout_value.set_text(code);
    }

    /// The state chips: everything that is worth knowing before reading a row.
    pub fn render_chips(&self) {
        while let Some(child) = self.chips.first_child() {
            self.chips.remove(&child);
        }
        let last = self.last.borrow();
        let mut chips: Vec<(String, &str)> = Vec::new();
        if last.updates > 0 {
            chips.push((format!("{} updates", last.updates), "warn"));
        }
        if self.inhibitor.borrow().active() {
            chips.push(("presentation".to_string(), "warn"));
        }
        if !last.radio && last.present {
            chips.push(("wifi off".to_string(), "warn"));
        }
        chips.push((
            format!(
                "bluetooth {}",
                if last.bluetooth.powered { "on" } else { "off" }
            ),
            if last.bluetooth.powered {
                "accent"
            } else {
                "dim"
            },
        ));
        for (text, style) in chips {
            let chip = gtk::Label::new(Some(&text));
            chip.add_css_class("chip");
            chip.add_css_class(style);
            self.chips.append(&chip);
        }
        self.chips.set_visible(true);
    }

    /// Whether the presentation switch is holding a block - what `status`
    /// prints, and what the header chip is drawn from.
    pub fn presenting(&self) -> bool {
        self.inhibitor.borrow().active()
    }

    /// What the last bluetooth read said - `status` reports it without a card in
    /// the way.
    pub fn bluetooth(&self) -> Bluetooth {
        self.last.borrow().bluetooth
    }

    /// Let go of the inhibitor, on the way out. A block nobody can see is worse
    /// than no block: it would keep the machine awake for the rest of the
    /// session after this element is gone.
    pub fn release(&self) {
        self.inhibitor.borrow_mut().release();
    }
}

// ---------------------------------------------------------------------------
// Small building blocks
// ---------------------------------------------------------------------------

/// A tile: the nested surface inside the card, with its section heading.
fn tile(title: &str, glyph: &str) -> gtk::Box {
    let heading = gtk::Label::new(Some(glyph));
    heading.add_css_class("section-glyph");
    let name = gtk::Label::new(Some(title));
    name.add_css_class("section");
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    line.add_css_class("section-row");
    line.append(&heading);
    line.append(&name);

    let tile = gtk::Box::new(gtk::Orientation::Vertical, 6);
    tile.add_css_class("tile");
    tile.append(&line);
    tile
}

/// One resource readout: glyph, name, bar, value. The bar takes the slack, which
/// is what pushes the name to the left edge and the value to the right one -
/// two clean columns without a single hard-coded width.
fn resource_row(glyph: &str, name: &str) -> (gtk::Box, gtk::ProgressBar, gtk::Label) {
    let icon = gtk::Label::new(Some(glyph));
    icon.add_css_class("section-glyph");
    let label = gtk::Label::new(Some(name));
    label.add_css_class("section");
    let bar = gtk::ProgressBar::new();
    bar.set_show_text(false);
    bar.set_hexpand(true);
    bar.set_valign(gtk::Align::Center);
    let value = gtk::Label::new(Some("—"));
    value.add_css_class("value");
    value.set_xalign(1.0);
    // A fixed column for the reading, so a value that changes width ("9%" to
    // "11%", "52°C" to "100°C") does not shuffle the bar beside it.
    value.set_size_request(46, -1);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("res-row");
    row.append(&icon);
    row.append(&label);
    row.append(&bar);
    row.append(&value);
    (row, bar, value)
}

/// A name on the left and the live value - or the control itself - on the right
/// of the same row: the scheme every control row follows, so the tile reads as
/// one aligned table rather than as a ragged pile of widgets. The value carries
/// the expand, which is what separates the two columns.
fn control_row(glyph: &str, name: &str, value: &impl IsA<gtk::Widget>) -> gtk::Box {
    let icon = gtk::Label::new(Some(glyph));
    icon.add_css_class("section-glyph");
    let label = gtk::Label::new(Some(name));
    label.add_css_class("section");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("control-row");
    row.append(&icon);
    row.append(&label);
    row.append(value);
    row
}

/// A button with a word in it - the popup's only kind of button. The word is
/// what changes ("Turn off" → "Turn on"), so it is reachable through
/// [`label_of`].
fn chip(text: &str, action: impl Fn() + 'static) -> gtk::Button {
    let label = gtk::Label::new(Some(text));
    let button = gtk::Button::new();
    button.add_css_class("chipbtn");
    button.set_child(Some(&label));
    button.set_focus_on_click(false);
    button.set_can_focus(false);
    button.connect_clicked(move |_| action());
    button
}

/// The word inside a [`chip`].
fn label_of(button: &gtk::Button) -> gtk::Label {
    button
        .child()
        .and_downcast::<gtk::Label>()
        .expect("a chip button's child is its label")
}

fn connect_chip<F>(button: &gtk::Button, view: Rc<StatsView>, action: F)
where
    F: Fn(&Rc<StatsView>) + 'static,
{
    button.connect_clicked(move |_| action(&view));
}

fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

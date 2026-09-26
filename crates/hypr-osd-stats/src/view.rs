//! The system popup: what the machine is doing, and the controls for it.
//!
//! A card of tiles that unfolds from the bar's status pill:
//!
//! ```text
//!  ┌─────────────────────────────────────────────────────────────────────┐
//!  │                                                  ▬▬▬               │
//!  │  SYSTEM                [15 updates] [wifi off] [bluetooth on]      │
//!  │ ─────────────────────────────────────────────────────────────────── │
//!  │ ┌─ resources ────────────────────┐ ┌─ controls ───────────────────┐ │
//!  │ │  ⌗ CPU       ▤ MEM     ▒ TEMP  │ │ ☁ NETWORK        home-wifi   │ │
//!  │ │    11%         63%       52°C  │ │   [nmtui] [Wi-Fi off]        │ │
//!  │ │    ▬▬▬▬        ▬▬▬▬      ▬▬▬▬  │ │ ♪ SOUND             42%      │ │
//!  │ │                        [btop]  │ │   [Mixer] [Mute]             │ │
//!  │ └────────────────────────────────┘ │ ⚡ POWER                     │ │
//!  │ ┌─ bluetooth ────────────────────┐ │   [saver|balanced|perf]      │ │
//!  │ │ On · 1 connected    [Visible]⏻│ │ ▷ PRESENTATION      [off]    │ │
//!  │ │ ▍◆ WH-1000XM4  ▂▄▆  82%   [↯] │ │ ☀ ▬▬▬▬▬▬▬▬▬▬▬ 100%           │ │
//!  │ │ ▍⌨ K380        ▂▄         [↗] │ │ ⌨ LAYOUT              DE     │ │
//!  │ │ [Scan] [bluetoothctl]          │ └──────────────────────────────┘ │
//!  │ └────────────────────────────────┘ ┌─ updates ────────────────────┐ │
//!  │                                    │ 15 packages waiting          │ │
//!  │                                    │ base, linux, firefox [Install]│ │
//!  │                                    └──────────────────────────────┘ │
//!  └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Two columns: the machine's *load* on the left (three gauges, then the
//! bluetooth radios with the devices they know), the machine's *controls* on the
//! right (one row per thing, the name on the left, the value or the control on
//! the right), with the pending updates under them. The control rows are the
//! table: every row has the same shape, so the eye can run down the values.
//!
//! Everything here is *direct*: the bluetooth controller has a power button and
//! a visibility switch, each device row carries the one action that makes sense
//! for that device (connect, disconnect, or pair what a scan found) plus a
//! forget button on what is paired, the power profile is a segmented switch
//! rather than a button that cycles through it, and the brightness is a slider.
//! What a control does is readable from the control itself - no "click and see".
//!
//! The view paints what it is handed and runs the commands its buttons mean. It
//! does not read anything itself: `main` owns the readings and the clock, so
//! there is one place that decides *when* to read and one that decides how to
//! draw. The one piece of state it does keep is what the last reading said -
//! because a button like "Disconnect" has to know the device is connected to say
//! what it does.
//!
//! The card itself - the fill, the border, the shadow, the 16px radius - is the
//! shell's (`box.card` in `base.css`), and the tiles, chips and grip are the
//! collection's shared recipes; what is here is only what goes inside one.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;

use hypr_osd_core::hardware::{self, Network, Volume};
use hypr_osd_core::system::{Reading, TEMP_CRIT, TEMP_WARN};

use crate::sources::{self, Bluetooth, Device, DeviceAction, Inhibitor};
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
const EYE: &str = "\u{f06e}";
const EYE_SLASH: &str = "\u{f070}";
const BATTERY: &str = "\u{f240}";
/// The four partial blocks a link-quality meter fills from the left: the
/// filled ones read in the dim colour, the rest fainter, which is the meter
/// without drawing a trough for it.
const SIGNAL_BARS: &str = "\u{2582}\u{2584}\u{2586}\u{2588}";
const NETWORK: &str = "\u{f1eb}";
const VOLUME: &str = "\u{f028}";
const MUTE: &str = "\u{f026}";
const POWER: &str = "\u{f0e7}";
const POWER_OFF: &str = "\u{f011}";
const PRESENTATION: &str = "\u{f03d}";
const BRIGHTNESS: &str = "\u{f185}";
const LAYOUT: &str = "\u{f11c}";
const TERMINAL: &str = "\u{f120}";
const MIXER: &str = "\u{f1de}";
const SEARCH: &str = "\u{f002}";
const INSTALL: &str = "\u{f019}";

/// How long a row's button spins after it was clicked. Long enough to cover
/// `bluetoothctl` talking to a device (a connect is a second or three), short
/// enough that a spinner is never stuck turning for good.
const ACTION_TIMEOUT: Duration = Duration::from_secs(8);

/// What a pairing attempt may hold its row's button for.
///
/// Pairing is not one command but a conversation - pair, trust, connect - and
/// `bluetoothctl` is given `sources::PAIR_TIMEOUT_SECONDS` for the middle of it,
/// so this is that with room for the rest. It is only the backstop: the chain
/// itself says when it is over, and that answer (`pair_result`) is much sooner.
const PAIR_PATIENCE: Duration = Duration::from_secs(45);

/// How long the note under the device list stays up. Long enough to be read,
/// short enough that it is gone before it turns into furniture.
const NOTE_TIME: Duration = Duration::from_secs(8);

/// How long this action may hold its row's button before the answer is clearly
/// not coming. See [`ACTION_TIMEOUT`] and [`PAIR_PATIENCE`].
fn patience(action: DeviceAction) -> Duration {
    match action {
        DeviceAction::Pair => PAIR_PATIENCE,
        _ => ACTION_TIMEOUT,
    }
}

/// What the last reading said - what the buttons act on.
///
/// A toggle button has to know the state to toggle *from* ("Turn off" vs "Turn
/// on"), and asking the system again at click time would mean a blocking read in
/// the click path. The reading is at most a few seconds old, and every one of
/// these toggles is a no-op in the worst case.
#[derive(Default)]
struct Last {
    bluetooth: Bluetooth,
    /// The devices of the last read, in the order they were listed: what the
    /// rows are rebuilt from when something else changes (a click, the scan
    /// ending) without waiting for the next read.
    devices: Vec<Device>,
    /// Whether a scan is running, which the Scan button and the header chip say.
    scanning: bool,
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

/// A device whose button was just clicked and whose answer has not arrived yet.
struct Pending {
    address: String,
    action: DeviceAction,
    /// When to stop believing in it, whatever happened: a spinner that turns
    /// for the rest of the session is worse than a button that admits nothing
    /// happened.
    until: Instant,
}

pub struct StatsView {
    /// The card's content: the shell's card widget is its parent.
    pub root: gtk::Box,
    /// The state chips in the header, rebuilt whenever something changes them.
    chips: gtk::Box,
    /// The three gauges: one meter and one reading each.
    bars: [gtk::ProgressBar; 3],
    values: [gtk::Label; 3],
    updates_state: gtk::Label,
    updates_list: gtk::Label,
    install: gtk::Button,
    bluetooth_state: gtk::Label,
    bluetooth_power: gtk::Button,
    /// The "Visible"/"Hidden" chip next to the power button: whether the
    /// controller answers pairing requests.
    bluetooth_visible: gtk::Button,
    /// The Scan button's normal content (glyph + word), kept so the spinner
    /// that replaces it while a scan runs can be swapped back out.
    scan_idle: gtk::Box,
    /// Where the device rows are built. Rebuilt wholesale on every read, because
    /// a list of devices is a list of widgets that come and go.
    device_list: gtk::Box,
    /// What the list says when it has nothing to list ("Bluetooth is off", the
    /// dashed "no devices yet" box).
    device_hint: gtk::Label,
    device_more: gtk::Label,
    /// One line about what just happened to a device - today only a pairing that
    /// did not work, because that is the one thing a row cannot say by itself.
    /// It clears itself again after a few seconds.
    device_note: gtk::Label,
    scan: gtk::Button,
    network_state: gtk::Label,
    radio_toggle: gtk::Button,
    sound_state: gtk::Label,
    mute_toggle: gtk::Button,
    /// The power profile as one button per profile, in [`sources::PROFILES`]
    /// order: the lit one is the profile in force.
    profile: [gtk::Button; 3],
    presentation: gtk::Button,
    brightness_scale: gtk::Scale,
    brightness_value: gtk::Label,
    layout_value: gtk::Label,
    last: RefCell<Last>,
    inhibitor: RefCell<Inhibitor>,
    /// The device whose button is waiting for an answer, if any.
    pending: RefCell<Option<Pending>>,
    /// Set while the brightness slider is being moved *by the program*: a
    /// `set_value` during a refresh is indistinguishable from a drag at the
    /// signal, and would write the backlight back to where it already is.
    scale_guard: Cell<bool>,
    /// How many device rows the list shows - the rest is a count. `main` reads
    /// the same number when it asks for the devices, so no details are read for
    /// rows that will not be drawn.
    max_devices: usize,
    /// What to run shortly after an action, so the row shows the new state
    /// rather than the old one until the next heartbeat. `main` installs it.
    on_refresh: RefCell<Option<Rc<dyn Fn()>>>,
    /// What the Scan button runs. `main` installs it, because the scan is a
    /// process with a lifetime, and the element - not the view - is what keeps
    /// the clock that notices it ending.
    on_scan: RefCell<Option<Rc<dyn Fn()>>>,
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
        header.append(&stretch());
        header.append(&chips);

        // ---- resources: three gauges in a row -------------------------------
        let (resources, resources_head) = tile("resources", CPU);
        let metrics = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        metrics.add_css_class("metrics");
        let mut bars = Vec::new();
        let mut values = Vec::new();
        for (glyph, name) in [(CPU, "CPU"), (MEMORY, "MEM"), (TEMPERATURE, "TEMP")] {
            let (metric, bar, value) = gauge(glyph, name);
            metrics.append(&metric);
            bars.push(bar);
            values.push(value);
        }
        resources.append(&metrics);
        let btop = glyph_chip(TERMINAL, "btop", {
            let terminal = settings.terminal_command.clone();
            move || sources::spawn(&terminal, &["-e", "btop"])
        });
        btop.set_tooltip_text(Some("The full picture, in btop"));
        resources_head.append(&stretch());
        resources_head.append(&btop);

        // ---- bluetooth: the controller, and the devices it knows ------------
        // A tile of its own, because it is the one control with *content*: the
        // state of the radio, the visibility of the controller, and then a row
        // per device with the one action that makes sense for that device.
        let (bluetooth_tile, _) = tile("bluetooth", BLUETOOTH);
        let bluetooth_state = gtk::Label::new(Some("Off"));
        bluetooth_state.add_css_class("value");
        bluetooth_state.set_xalign(0.0);
        // The two halves of "is the controller approachable": whether it is on,
        // and whether it answers pairing requests while it is.
        let bluetooth_visible = glyph_chip(EYE_SLASH, "Hidden", || {});
        bluetooth_visible.set_tooltip_text(Some(
            "Answer pairing requests - make the controller visible to new devices",
        ));
        let bluetooth_power = icon_button(POWER_OFF, "Turn bluetooth on");
        let bluetooth_head_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bluetooth_head_row.add_css_class("bt-state");
        bluetooth_head_row.append(&bluetooth_state);
        bluetooth_head_row.append(&stretch());
        bluetooth_head_row.append(&bluetooth_visible);
        bluetooth_head_row.append(&bluetooth_power);
        bluetooth_tile.append(&bluetooth_head_row);

        let device_list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        device_list.add_css_class("dev-list");
        let device_hint = gtk::Label::new(None);
        device_hint.add_css_class("dev-hint");
        device_hint.set_xalign(0.5);
        device_hint.set_visible(false);
        let device_more = gtk::Label::new(None);
        device_more.add_css_class("dev-more");
        device_more.set_xalign(0.0);
        device_more.set_visible(false);
        // The one line a device row cannot say for itself: that asking for it
        // failed. A pairing that goes back to saying "Pair" without a word is
        // what a broken button looks like.
        let device_note = gtk::Label::new(None);
        device_note.add_css_class("dev-note");
        device_note.set_xalign(0.0);
        device_note.set_wrap(true);
        device_note.set_visible(false);
        bluetooth_tile.append(&device_list);
        bluetooth_tile.append(&device_hint);
        bluetooth_tile.append(&device_more);
        bluetooth_tile.append(&device_note);

        let scan = glyph_chip(SEARCH, "Scan", || {});
        scan.set_tooltip_text(Some("Look for devices nearby for a few seconds"));
        // The idle content is kept so the spinner that replaces it while a scan
        // runs can be swapped back out when the controller stops looking.
        let scan_idle = scan
            .child()
            .and_downcast::<gtk::Box>()
            .expect("a chip's child is its glyph-and-word row");
        let bluetooth_open = glyph_chip(TERMINAL, "bluetoothctl", {
            let terminal = settings.terminal_command.clone();
            move || sources::spawn(&terminal, &["-e", "bluetoothctl"])
        });
        bluetooth_open.set_tooltip_text(Some("Everything else bluetooth, in a terminal"));
        bluetooth_tile.append(&action_row(&[&scan, &bluetooth_open]));

        // ---- updates: what is waiting, and the one way to act on it ---------
        let (updates, updates_head) = tile("updates", UPDATES);
        let updates_state = gtk::Label::new(Some("System up to date"));
        updates_state.add_css_class("value-lg");
        updates_state.set_xalign(0.0);
        // A few package names, so the count is a thing rather than a number.
        let updates_list = gtk::Label::new(None);
        updates_list.add_css_class("upd-list");
        updates_list.set_xalign(0.0);
        updates_list.set_visible(false);
        updates.append(&updates_state);
        updates.append(&updates_list);
        let install = glyph_chip(INSTALL, "Install", {
            let terminal = settings.terminal_command.clone();
            move || sources::spawn(&terminal, &["-e", "sudo", "pacman", "-Syu"])
        });
        install.set_tooltip_text(Some("Update now, in a terminal"));
        updates_head.append(&stretch());
        updates_head.append(&install);

        let left = gtk::Box::new(gtk::Orientation::Vertical, 8);
        left.add_css_class("column");
        left.set_size_request(settings.left_width, -1);
        left.append(&resources);
        left.append(&bluetooth_tile);

        // ---- controls -------------------------------------------------------
        let (controls, _) = tile("controls", CONTROLS);

        // NETWORK: which access point, and the two things you might want to do
        // about it. The pill's own wifi toggle lives here too - this is where
        // managing the link belongs now that the pill is one handle.
        let network_state = gtk::Label::new(Some("Not connected"));
        network_state.add_css_class("value");
        network_state.set_ellipsize(gtk::pango::EllipsizeMode::End);
        network_state.set_max_width_chars(16);
        controls.append(&control_row(NETWORK, "NETWORK", &network_state));
        let network_open = glyph_chip(TERMINAL, "nmtui", {
            let terminal = settings.terminal_command.clone();
            move || sources::spawn(&terminal, &["-e", "nmtui"])
        });
        let radio_toggle = glyph_chip(NETWORK, "Wi-Fi off", || {});
        controls.append(&action_row(&[&network_open, &radio_toggle]));

        // SOUND: the sink's level, and the two things the volume pill used to
        // do with a right and a middle click.
        let sound_state = gtk::Label::new(Some("—"));
        sound_state.add_css_class("value");
        controls.append(&control_row(VOLUME, "SOUND", &sound_state));
        let mixer = glyph_chip(MIXER, "Mixer", || sources::spawn("pavucontrol", &[]));
        mixer.set_tooltip_text(Some("Per-application levels, in pavucontrol"));
        let mute_toggle = glyph_chip(MUTE, "Mute", || {});
        controls.append(&action_row(&[&mixer, &mute_toggle]));

        // POWER: one button per profile, the lit one being the profile in force.
        // Direct, with no guessing what the next click would cycle to.
        let mut profile = Vec::new();
        let profile_row = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        profile_row.add_css_class("segmented");
        for (name, label) in [
            ("power-saver", "saver"),
            ("balanced", "balanced"),
            ("performance", "perf"),
        ] {
            let button = gtk::Button::new();
            button.add_css_class("seg");
            button.set_child(Some(&gtk::Label::new(Some(label))));
            button.set_focus_on_click(false);
            button.set_can_focus(false);
            button.set_tooltip_text(Some(name));
            profile_row.append(&button);
            profile.push(button);
        }
        controls.append(&control_row(POWER, "POWER", &profile_row));

        // PRESENTATION: hold an idle/sleep block for as long as it is on.
        let presentation = glyph_chip(PRESENTATION, "off", || {});
        presentation.set_tooltip_text(Some(
            "Hold a systemd inhibitor (idle and sleep) - what the old \
             idle_inhibitor module did, with a switch that goes away when this \
             element does",
        ));
        controls.append(&control_row(PRESENTATION, "PRESENTATION", &presentation));

        // BRIGHTNESS: glyph, slider, percent. This row carries no name: the
        // slider is the widest thing in the column, and the glyph says the rest.
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
        layout_value.set_size_request(30, -1);
        layout_value.set_xalign(1.0);
        controls.append(&control_row(LAYOUT, "LAYOUT", &layout_value));

        let right = gtk::Box::new(gtk::Orientation::Vertical, 8);
        right.add_css_class("column");
        right.set_size_request(settings.right_width, -1);
        right.append(&controls);
        right.append(&updates);

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
            bars: bars.try_into().expect("three gauges"),
            values: values.try_into().expect("three gauges"),
            updates_state,
            updates_list,
            install,
            bluetooth_state,
            bluetooth_power,
            bluetooth_visible,
            scan_idle,
            device_list,
            device_hint,
            device_more,
            device_note,
            scan,
            network_state,
            radio_toggle,
            sound_state,
            mute_toggle,
            profile: profile.try_into().expect("three profiles"),
            presentation,
            brightness_scale,
            brightness_value,
            layout_value,
            last: RefCell::new(Last::default()),
            inhibitor: RefCell::new(Inhibitor::default()),
            pending: RefCell::new(None),
            scale_guard: Cell::new(true),
            max_devices: settings.max_devices,
            on_refresh: RefCell::new(None),
            on_scan: RefCell::new(None),
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

    /// Install what the Scan button runs. `main` owns it, because a scan is a
    /// process with a lifetime and the element is what holds the clock that
    /// notices it ending.
    pub fn on_scan(&self, hook: Rc<dyn Fn()>) {
        *self.on_scan.borrow_mut() = Some(hook);
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
        connect_chip(&self.bluetooth_power, self.me(), move |view| {
            sources::set_bluetooth(!view.last.borrow().bluetooth.powered);
            view.refresh_soon(700);
        });

        connect_chip(&self.bluetooth_visible, self.me(), move |view| {
            sources::set_discoverable(!view.last.borrow().bluetooth.discoverable);
            view.refresh_soon(700);
        });

        connect_chip(&self.scan, self.me(), move |view| {
            // Cloned out of the cell first, so the hook runs with no borrow held
            // on the hook itself: it repaints the panel, and a repaint of the
            // panel is exactly the kind of re-entry a live borrow would panic on.
            let hook = view.on_scan.borrow().clone();
            if let Some(hook) = hook {
                hook();
            }
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

        // One button per profile, and a click sets *that* profile: the index is
        // the profile, so nothing has to work out what "next" would mean.
        for (index, button) in self.profile.iter().enumerate() {
            let view = self.me();
            button.connect_clicked(move |_| {
                sources::set_power_profile(sources::PROFILES[index]);
                view.refresh_soon(600);
            });
        }

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
            // The colour goes on both the number and its meter: a red reading
            // over an accent-coloured bar would read as two different opinions
            // about the same sensor.
            set_class(&self.values[index], "warn", warn);
            set_class(&self.values[index], "crit", crit);
            set_class(&self.bars[index], "warn", warn);
            set_class(&self.bars[index], "crit", crit);
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

    /// The controller, the devices it knows, and whether a scan is running.
    ///
    /// All three arrive together, because they come out of one read: the power
    /// state is the *header*, the device list is the body, and the scan is what
    /// the button under it is doing.
    pub fn render_bluetooth(&self, bluetooth: Bluetooth, devices: Vec<Device>, scanning: bool) {
        {
            let mut last = self.last.borrow_mut();
            last.bluetooth = bluetooth;
            last.devices = devices;
            last.scanning = scanning;
        }
        self.bluetooth_state.set_text(&bluetooth.state());
        set_class(&self.bluetooth_state, "dim", !bluetooth.powered);

        // The power button keeps its glyph whatever the state and says the rest
        // with *colour* and its tooltip: a button whose symbol changes under the
        // pointer is a button you have to read twice, and the state is written
        // out next to it anyway ("On · 2 connected" / "Off").
        self.bluetooth_power
            .set_tooltip_text(Some(&format!("Bluetooth: {}", bluetooth.action())));
        set_class(&self.bluetooth_power, "on", bluetooth.powered);

        // The visibility chip says what the controller is doing ("Visible" /
        // "Hidden") and lights in the warning colour while it is holding itself
        // open to pairing - the same "switch holding something back" the
        // presentation row uses, not the plain "on" accent.
        word_of(&self.bluetooth_visible).set_text(bluetooth.visibility());
        glyph_of(&self.bluetooth_visible).set_text(if bluetooth.discoverable {
            EYE
        } else {
            EYE_SLASH
        });
        self.bluetooth_visible
            .set_tooltip_text(Some(if bluetooth.discoverable {
                "Stop answering pairing requests"
            } else {
                "Answer pairing requests - make the controller visible to new devices"
            }));
        set_class(&self.bluetooth_visible, "on", bluetooth.discoverable);
        self.bluetooth_visible.set_sensitive(bluetooth.powered);

        self.render_scan_button(scanning, bluetooth.powered);

        self.rebuild_devices();
    }

    /// What the Scan button shows: its usual glyph-and-word, or a spinner and
    /// "Scanning" while the controller is looking. The word alone could not
    /// say "working", and the spinner can.
    fn render_scan_button(&self, scanning: bool, powered: bool) {
        if scanning {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            row.add_css_class("btn-row");
            let spinner = gtk::Spinner::new();
            spinner.add_css_class("btn-spinner");
            spinner.start();
            let word = gtk::Label::new(Some("Scanning"));
            word.add_css_class("btn-word");
            row.append(&spinner);
            row.append(&word);
            self.scan.set_child(Some(&row));
        } else {
            self.scan.set_child(Some(&self.scan_idle));
        }
        self.scan.set_sensitive(powered && !scanning);
    }

    /// Whether a scan is running, without re-reading anything.
    ///
    /// The scan is the element's own process, so it is the only thing that knows
    /// when one starts and ends - and the button and the header chip have to
    /// follow it in between: at the click, and again when the controller stops
    /// looking.
    pub fn render_scanning(&self, scanning: bool) {
        self.last.borrow_mut().scanning = scanning;
        let powered = self.last.borrow().bluetooth.powered;
        self.render_scan_button(scanning, powered);
        self.rebuild_devices();
        self.render_chips();
    }

    /// Rebuild the rows from the last device list.
    ///
    /// The list is rebuilt whole rather than diffed - devices come and go, and a
    /// handful of labels is cheaper to remake than to reconcile - and it is what
    /// a click re-runs, so a button that was pressed stops looking pressable
    /// without waiting for the next read.
    fn rebuild_devices(&self) {
        self.prune_pending();
        while let Some(child) = self.device_list.first_child() {
            self.device_list.remove(&child);
        }
        let last = self.last.borrow();
        let devices = last.devices.clone();
        let powered = last.bluetooth.powered;

        let hint = match (powered, devices.is_empty()) {
            (false, _) => Some("Bluetooth is off"),
            (true, true) => Some("No devices yet - scan to find some"),
            (true, false) => None,
        };
        self.device_hint.set_visible(hint.is_some());
        if let Some(hint) = hint {
            self.device_hint.set_text(hint);
        }
        // The dashed box is the "the list is empty and could be full" state -
        // it has no business framing the plain "Bluetooth is off" line.
        set_class(&self.device_hint, "empty", powered && devices.is_empty());
        self.device_list.set_visible(powered);

        if powered {
            for device in devices.iter().take(self.max_devices) {
                self.device_list.append(&self.device_row(device));
            }
            let hidden = devices.len().saturating_sub(self.max_devices);
            self.device_more.set_visible(hidden > 0);
            if hidden > 0 {
                // The count, not the names: the list is a list, and the rest of
                // it belongs to `bluetoothctl` (one button away, below).
                self.device_more
                    .set_text(&format!("+{hidden} more - see all in bluetoothctl"));
            }
        } else {
            self.device_more.set_visible(false);
        }
    }

    /// One device: what it is, how its link is doing, its battery, and the
    /// action that follows from its state - plus a way to forget it.
    fn device_row(&self, device: &Device) -> gtk::Box {
        // The mark is what makes a connected row read at a glance: a thin
        // accent bar at the row's edge, stronger than the coloured badge
        // alone. It is always in the row - only its colour comes and goes -
        // so connected and idle rows keep their badges aligned.
        let mark = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        mark.add_css_class("dev-mark");
        mark.set_valign(gtk::Align::Center);
        set_class(&mark, "live", device.connected);

        let glyph = gtk::Label::new(Some(device.icon.glyph()));
        glyph.add_css_class("dev-glyph");
        let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        badge.add_css_class("dev-badge");
        badge.set_valign(gtk::Align::Center);
        badge.append(&glyph);
        // A device that is actually *there* is the one thing worth colouring in
        // this list: everything else is a name in a cache.
        set_class(&badge, "live", device.connected);

        let name = gtk::Label::new(Some(device.label()));
        name.add_css_class("dev-name");
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        // Capped, so a device with a long name cannot set the panel's width -
        // which is also the width the hover zone is measured against (see
        // `main::zones`).
        name.set_max_width_chars(22);
        let detail = gtk::Label::new(Some(&device.detail()));
        detail.add_css_class("dev-sub");
        detail.set_xalign(0.0);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        text.append(&name);
        text.append(&detail);

        // The link quality as bars: filled bars in the dim colour, the rest
        // fainter. The raw dBm moves to the tooltip - a number nobody can
        // calibrate at a glance is worse than a meter.
        let signal = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        signal.add_css_class("dev-sig");
        signal.set_valign(gtk::Align::Center);
        if let Some(level) = device.signal() {
            let filled: String = SIGNAL_BARS.chars().take(level as usize).collect();
            let empty: String = SIGNAL_BARS.chars().skip(level as usize).collect();
            let on = gtk::Label::new(Some(&filled));
            on.add_css_class("sig-on");
            let off = gtk::Label::new(Some(&empty));
            off.add_css_class("sig-off");
            signal.append(&on);
            signal.append(&off);
        }

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("dev");
        row.append(&mark);
        row.append(&badge);
        row.append(&text);
        if device.signal().is_some() {
            row.append(&signal);
        }

        // The battery as a pill of its own, so it lines up across the rows and
        // a low one can colour itself without colouring the whole row.
        if let Some(battery) = device.battery {
            let pill = gtk::Box::new(gtk::Orientation::Horizontal, 3);
            pill.add_css_class("dev-batt");
            pill.set_valign(gtk::Align::Center);
            set_class(&pill, "low", battery < 20);
            pill.set_tooltip_text(Some("Battery, as the device reports it"));
            let icon = gtk::Label::new(Some(BATTERY));
            icon.add_css_class("batt-glyph");
            let value = gtk::Label::new(Some(&format!("{battery}%")));
            value.add_css_class("batt-value");
            pill.append(&icon);
            pill.append(&value);
            row.append(&pill);
        }

        let action = device.action();
        let waiting = self.pending_for(&device.address);
        let button = gtk::Button::new();
        button.add_css_class("iconbtn");
        button.set_focus_on_click(false);
        button.set_can_focus(false);
        if waiting {
            // A spinner rather than a "…": the button is working, and a
            // spinner says so without pretending to be a label.
            let spinner = gtk::Spinner::new();
            spinner.add_css_class("btn-spinner");
            spinner.start();
            button.set_child(Some(&spinner));
            button.set_sensitive(false);
            button.set_tooltip_text(Some("Talking to the device…"));
        } else {
            let glyph = gtk::Label::new(Some(action.glyph()));
            glyph.add_css_class("btn-glyph");
            button.set_child(Some(&glyph));
            button.set_tooltip_text(Some(action.tooltip()));
            set_class(&button, "on", device.connected);
            let view = self.me();
            let address = device.address.clone();
            button.connect_clicked(move |_| view.run_device_action(&address, action));
        }
        row.append(&button);

        // The paired ones a scan no longer sees are exactly the ones worth
        // forgetting - so the forget button lives on a paired, unconnected row,
        // next to its Connect. Connected rows do not carry it: forgetting a
        // live link is never the first thing to want.
        if device.paired && !device.connected && !waiting {
            let remove = icon_button(DeviceAction::Remove.glyph(), DeviceAction::Remove.tooltip());
            remove.add_css_class("danger");
            let view = self.me();
            let address = device.address.clone();
            remove.connect_clicked(move |_| view.run_device_action(&address, DeviceAction::Remove));
            row.append(&remove);
        }

        row.set_tooltip_text(Some(&match device.rssi {
            Some(rssi) => format!("{} · {} dBm", device.address, rssi),
            None => device.address.clone(),
        }));
        row
    }

    /// Run a device's action, and say so until the answer lands.
    ///
    /// The row's button turns into a spinner for at most [`patience`]: the read
    /// that follows the click usually settles it sooner, and this is only the
    /// backstop so that a device which never answers cannot leave a button
    /// spinning for the rest of the session.
    fn run_device_action(&self, address: &str, action: DeviceAction) {
        match action {
            DeviceAction::Connect => sources::connect_device(address),
            DeviceAction::Disconnect => sources::disconnect_device(address),
            // Pairing is the one action that can take half a minute, and the
            // only one that can *fail*: the chain reports when it is over, and
            // that answer is what takes the spinner away and what the tile says
            // when it was not enough.
            DeviceAction::Pair => {
                let view = self.me();
                // The chain outlives this click, so the answer owns its own copy
                // of the address; the call itself only borrows it long enough to
                // start the reads.
                let waiting = address.to_string();
                sources::pair_device(address, move |paired| {
                    view.pair_result(&waiting, paired);
                });
            }
            DeviceAction::Remove => sources::remove_device(address),
        }
        *self.pending.borrow_mut() = Some(Pending {
            address: address.to_string(),
            action,
            until: Instant::now() + patience(action),
        });
        self.rebuild_devices();
        self.refresh_soon(900);
    }

    /// The pairing chain has answered: take the row's spinner away, and say so
    /// when the device did not end up paired.
    ///
    /// The spinner is only cleared while this is still the row that is waiting -
    /// a click on another device in the meantime owns it now, and the re-read
    /// below settles both either way.
    fn pair_result(&self, address: &str, paired: bool) {
        {
            let mut pending = self.pending.borrow_mut();
            let waiting_here = pending.as_ref().is_some_and(|waiting| {
                waiting.address == address && waiting.action == DeviceAction::Pair
            });
            if waiting_here {
                *pending = None;
            }
        }
        if !paired {
            // The panel pairs with a `NoInputNoOutput` agent, so the usual reason
            // is a device that will not pair without a passkey - which only the
            // interactive `bluetoothctl` can ask for.
            self.note("Could not pair - the device may want a passkey (try bluetoothctl)");
        }
        self.rebuild_devices();
        self.refresh_soon(200);
    }

    /// Show the one-line note under the device list, and take it away again by
    /// itself: it reports something that just happened, not a state.
    fn note(&self, text: &str) {
        self.device_note.set_text(text);
        self.device_note.set_visible(true);
        let note = self.device_note.clone();
        glib::timeout_add_local_once(NOTE_TIME, move || note.set_visible(false));
    }

    /// Whether this device's row is waiting for an answer.
    fn pending_for(&self, address: &str) -> bool {
        self.pending
            .borrow()
            .as_ref()
            .is_some_and(|pending| pending.address == address)
    }

    /// Drop the "…" once the device has done what was asked of it, or once it
    /// has been waiting long enough that the answer is clearly not coming.
    fn prune_pending(&self) {
        let mut pending = self.pending.borrow_mut();
        let Some(waiting) = pending.as_ref() else {
            return;
        };
        let device = self
            .last
            .borrow()
            .devices
            .iter()
            .find(|device| device.address == waiting.address)
            .cloned();
        let settled = match (device, waiting.action) {
            (None, _) => true, // the device is not listed any more
            (Some(device), DeviceAction::Connect) => device.connected,
            (Some(device), DeviceAction::Disconnect) => !device.connected,
            (Some(device), DeviceAction::Pair) => device.paired && device.connected,
            // A device the panel asked to forget is settled only once the
            // controller's next read stops listing it.
            (Some(_), DeviceAction::Remove) => false,
        };
        if settled || Instant::now() >= waiting.until {
            *pending = None;
        }
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
        word_of(&self.radio_toggle).set_text(if radio { "Wi-Fi off" } else { "Wi-Fi on" });
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
                word_of(&self.mute_toggle).set_text(if volume.muted { "Unmute" } else { "Mute" });
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

    /// The power profile. The matching button is the lit one, which is the whole
    /// point of the segmented control: where the machine is now and what a click
    /// would do are the same piece of information.
    pub fn render_profile(&self, profile: Option<&str>) {
        self.last.borrow_mut().profile = profile.map(str::to_owned);
        let current = profile
            .and_then(|profile| sources::PROFILES.iter().position(|known| *known == profile));
        for (index, button) in self.profile.iter().enumerate() {
            set_class(button, "active", current == Some(index));
            // A machine without the daemon: the profile is not "unknown", it is
            // unavailable - so the control stops taking clicks rather than
            // offering three settings that would do nothing.
            button.set_sensitive(profile.is_some());
        }
    }

    pub fn render_presentation(&self, on: bool) {
        word_of(&self.presentation).set_text(if on { "on" } else { "off" });
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
        if last.scanning {
            chips.push(("scanning".to_string(), "accent"));
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
///
/// Returns the heading row as well, because a tile's own action (the `btop`
/// button, the `Install` button) belongs at the right end of that heading - and
/// handing it back is cheaper than hunting for it in the widget tree.
fn tile(title: &str, glyph: &str) -> (gtk::Box, gtk::Box) {
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
    (tile, line)
}

/// One resource gauge: glyph and name, the reading, and the meter under it.
///
/// An empty box that takes the slack, so three of them share the tile's width
/// evenly whatever the numbers say - which is what keeps the three readings in
/// a row instead of in a queue.
fn gauge(glyph: &str, name: &str) -> (gtk::Box, gtk::ProgressBar, gtk::Label) {
    let meter = gtk::ProgressBar::new();
    meter.add_css_class("meter");
    meter.set_show_text(false);
    meter.set_valign(gtk::Align::Center);

    let value = gtk::Label::new(Some("—"));
    value.add_css_class("metric-value");
    value.set_xalign(0.0);

    let icon = gtk::Label::new(Some(glyph));
    icon.add_css_class("metric-glyph");
    let label = gtk::Label::new(Some(name));
    label.add_css_class("metric-name");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    head.add_css_class("metric-head");
    head.append(&icon);
    head.append(&label);

    let metric = gtk::Box::new(gtk::Orientation::Vertical, 1);
    metric.add_css_class("metric");
    metric.set_hexpand(true);
    metric.append(&head);
    metric.append(&value);
    metric.append(&meter);
    (metric, meter, value)
}

/// A name on the left and the live value - or the control itself - on the right
/// of the same row: the scheme every control row follows, so the tile reads as
/// one aligned table rather than as a ragged pile of widgets. The stretch
/// between them is what pins the value to the right edge.
fn control_row(glyph: &str, name: &str, value: &impl IsA<gtk::Widget>) -> gtk::Box {
    let icon = gtk::Label::new(Some(glyph));
    icon.add_css_class("section-glyph");
    let label = gtk::Label::new(Some(name));
    label.add_css_class("section");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("control-row");
    row.append(&icon);
    row.append(&label);
    row.append(&stretch());
    row.append(value);
    row
}

/// The buttons of one control, under that control's row and lined up to its
/// right edge - so a row that needs two steps ("nmtui", then the radio) reads as
/// a paragraph rather than as two unrelated rows.
fn action_row(buttons: &[&gtk::Button]) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("actions");
    row.set_halign(gtk::Align::End);
    for button in buttons {
        row.append(*button);
    }
    row
}

/// An empty box that takes the slack in a row: the popup's own `spacer` - the
/// thing that keeps two widgets apart without a hard-coded width.
fn stretch() -> gtk::Box {
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    spacer
}

/// A chip with a leading glyph and a word: the popup's button. The glyph says
/// what kind of thing the button is (a terminal, a magnifier), the word says what
/// it does - and the word is what changes ("Wi-Fi off" → "Wi-Fi on"), so it is
/// reachable through [`word_of`].
fn glyph_chip(glyph: &str, text: &str, action: impl Fn() + 'static) -> gtk::Button {
    let icon = gtk::Label::new(Some(glyph));
    icon.add_css_class("btn-glyph");
    let word = gtk::Label::new(Some(text));
    // The word carries its own class rather than inheriting the button's size:
    // `base.css` sets a font size on `*`, so inheritance never reaches a label.
    word.add_css_class("btn-word");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    row.add_css_class("btn-row");
    row.append(&icon);
    row.append(&word);

    let button = gtk::Button::new();
    button.add_css_class("chipbtn");
    button.set_child(Some(&row));
    button.set_focus_on_click(false);
    button.set_can_focus(false);
    button.set_tooltip_text(Some(text));
    button.connect_clicked(move |_| action());
    button
}

/// A round button whose whole content is one glyph: the popup's *direct*
/// controls - the bluetooth power, the action at the end of a device's row. The
/// tooltip carries the words, because there is nowhere else for them to go.
fn icon_button(glyph: &str, tooltip: &str) -> gtk::Button {
    let label = gtk::Label::new(Some(glyph));
    label.add_css_class("btn-glyph");
    let button = gtk::Button::new();
    button.add_css_class("iconbtn");
    button.set_child(Some(&label));
    button.set_focus_on_click(false);
    button.set_can_focus(false);
    button.set_tooltip_text(Some(tooltip));
    button
}

/// The word inside a [`glyph_chip`] (which is the button's second child).
fn word_of(button: &gtk::Button) -> gtk::Label {
    button
        .child()
        .and_downcast::<gtk::Box>()
        .and_then(|row| row.last_child())
        .and_downcast::<gtk::Label>()
        .expect("a chip's last child is its word")
}

/// The glyph inside a [`glyph_chip`] (which is the button's first child).
fn glyph_of(button: &gtk::Button) -> gtk::Label {
    button
        .child()
        .and_downcast::<gtk::Box>()
        .and_then(|row| row.first_child())
        .and_downcast::<gtk::Label>()
        .expect("a chip's first child is its glyph")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pairing_gets_the_patience_its_own_timeout_needs() {
        // The two numbers live in different modules and *have* to be in this
        // order: a spinner that gives up before `bluetoothctl` does would leave
        // the row looking idle while the attempt is still running.
        assert!(
            patience(DeviceAction::Pair) > Duration::from_secs(sources::PAIR_TIMEOUT_SECONDS),
            "the pairing spinner must outlast the pairing"
        );
        // Everything else answers in a second or three.
        assert_eq!(patience(DeviceAction::Connect), ACTION_TIMEOUT);
        assert_eq!(patience(DeviceAction::Disconnect), ACTION_TIMEOUT);
        assert_eq!(patience(DeviceAction::Remove), ACTION_TIMEOUT);
    }
}

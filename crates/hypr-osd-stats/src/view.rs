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
//!  │ │  ⌗ CPU       ▤ MEM     ▒ TEMP  │ │ ♪ SOUND          42%    ◉ ◉  │ │
//!  │ │    11%         63%       52°C  │ │ ──────────────────────────── │ │
//!  │ │    ▬▬▬▬        ▬▬▬▬      ▬▬▬▬  │ │ ⚡ POWER     [saver|bal|perf] │ │
//!  │ │                        [btop]  │ │ ▷ PRESENTATION         [off] │ │
//!  │ └────────────────────────────────┘ │ ☀ BRIGHTNESS  ▬▬▬▬▬▬▬▬  100% │ │
//!  │ ┌─ network ──────────────────────┐ │ ⌨ LAYOUT                  DE │ │
//!  │ │           [⌕ Scan] [❯ nmtui]   │ └──────────────────────────────┘ │
//!  │ │ Connected · home-wifi     ◉    │ ┌─ updates ────────────────────┐ │
//!  │ │ ▍◆ wlan@home-5G  ▂▄▆█  [⧉] [✕] │ │ 15 packages waiting          │ │
//!  │ │ ▍⊘ PHSG          ▂▄▆█  [⧉] [✕] │ │ base, linux, firefox [Install]│ │
//!  │ │ ▍○ Wlan@home     ▂▄▆·  [⧉]     │ └──────────────────────────────┘ │
//!  │ └────────────────────────────────┘                                  │ │
//!  │ ┌─ bluetooth ────────────────────┐                                  │ │
//!  │ │           [⌕ Scan] [▣ Manage]  │                                  │ │
//!  │ │ On · 1 connected          ◉ ◉  │                                  │ │
//!  │ │ ▍◆ WH-1000XM4  ▂▄▆  82%   [⧗]  │                                  │ │
//!  │ │ ▍⌨ K380        ▂▄  [⧉]  [✕]   │                                  │ │
//!  │ │ ▍○ MX Master   ▂▄        [+]   │                                  │ │
//!  │ └────────────────────────────────┘                                  │ │
//!  └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Two columns: the machine's *load* on the left (three gauges, then the two
//! radios - the wireless link with the networks it can hear, and bluetooth with
//! the devices it knows), the machine's *controls* on the right, with the pending
//! updates under them.
//!
//! The controls are a table, and the table *is* the design: a mark and a name on
//! the left, then whatever that setting is (a reading, a switch, a slider)
//! pinned to the row's right edge. One setting per row, the same shape all the
//! way down, so the eye runs a column instead of re-reading the arrangement of
//! every line. What a row has to *press* sits in a box at its end, and the rows
//! that carry the same pair of buttons put their readings in the same column;
//! the two controls that need room instead - the three-way switch and the
//! slider - take the rest of their row. A hairline divides the pair you *use*
//! from the four you *set*.
//!
//! The two radio tiles are built the same way, one above the other: each is a
//! *heading* that carries the tile's two tools ([Scan] and the tool that manages
//! the rest of it - [nmtui], [Manage] - where the resources and updates
//! tiles keep btop and Install), then the controller's own row (a reading plus
//! its switches), then a list with a fixed slot at the end for its rows' buttons,
//! which is what lines the link meters, the batteries and the actions up from row
//! to row. Because the readings sit against that slot and the buttons at the
//! tile's right edge, a row's tail is a column like any other.
//!
//! The two lists say the same kind of thing about different radios, so they are
//! the same shape on purpose: a network row is a badge (a lock on one that will
//! ask for a password), the network's name, what it is doing, its signal, and the
//! one button that follows from that - join, leave, or (for a network nobody has
//! joined before, where the password has to be typed somewhere) a terminal. A
//! stored profile that is not connected also gets the forget button, because
//! "managing" a connection is mostly letting go of one.
//!
//! Everything here is *direct*: the bluetooth controller has a power switch and
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
use hypr_osd_core::icons::{self, names};
use hypr_osd_core::state;
use hypr_osd_core::system::{Reading, TEMP_CRIT, TEMP_WARN};

use crate::sources::{self, Bluetooth, Device, DeviceAction, Inhibitor, Wifi, WifiAction};
use crate::Settings;

/// The four partial blocks a device's link-quality meter fills from the left:
/// the filled ones read in the dim colour, the rest fainter, which is the meter
/// without drawing a trough for it.
///
/// These stay *text*, unlike everything else in this panel: they are not a mark
/// for a thing but a meter, drawn by glyph composition, and U+2582..U+2588 are
/// ordinary Unicode that every monospace font has - nothing to bundle.
const SIGNAL_BARS: &str = "\u{2582}\u{2584}\u{2586}\u{2588}";

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

/// How large the popup's marks are drawn. Three sizes, because they answer
/// three questions: a heading's or a control row's icon belongs to the 10px word
/// beside it, a gauge's is a label for a number, and a button's has to be
/// aimable.
const SECTION_ICON: i32 = 12;
const METRIC_ICON: i32 = 11;
const BUTTON_ICON: i32 = 12;
const DEVICE_ICON: i32 = 12;
/// The battery in a device's pill: a footnote beside a 9px number.
const BATTERY_ICON: i32 = 10;

/// How long a row's button spins after it was clicked. Long enough to cover
/// `bluetoothctl` talking to a device (a connect is a second or three), short
/// enough that a spinner is never stuck turning for good.
const ACTION_TIMEOUT: Duration = Duration::from_secs(8);

/// The width a device row keeps for its buttons at the end, whatever it has to
/// put there.
///
/// Two buttons is the most a row can carry - the action that follows from its
/// state, and the forget on what is paired but not connected - and holding that
/// width on *every* row is what lines the readings up: they are packed against
/// this box, so without it a row with one button put its battery a button-width
/// further right than a row with two. It has to be the width two buttons
/// really take (measured, not guessed: the theme's button margins are part of
/// it), because a smaller number is only a floor and the box grows past it.
const DEVICE_ACTIONS: i32 = 64;

/// How many characters of an SSID a network row shows before it ellipsises.
///
/// Less than a device name's 22, because an SSID is up to 32 bytes: it is the
/// row that would otherwise decide the width of *both* radio tiles, and with it
/// the card's - which is what the hover zone is derived from. The whole name is
/// still in the row's tooltip.
const WIFI_NAME_CHARS: i32 = 18;

/// What a pairing attempt may hold its row's button for.
///
/// Pairing is not one command but a conversation - pair, trust, connect - and
/// `bluetoothctl` is given `sources::PAIR_TIMEOUT_SECONDS` for the middle of it,
/// so this is that with room for the rest. It is only the backstop: the chain
/// itself says when it is over, and that answer (`pair_result`) is much sooner.
const PAIR_PATIENCE: Duration = Duration::from_secs(45);

/// What joining or leaving a network may hold its row's button for.
///
/// Longer than a bluetooth action because there is a DHCP handshake in it: a
/// join that has to be given an address takes a handful of seconds, and a
/// spinner that gave up in the middle of one would look like a button that did
/// nothing.
const WIFI_PATIENCE: Duration = Duration::from_secs(20);

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

/// The same question for a network's row. Forgetting is a local change to a
/// profile and answers on its own; joining takes a lease.
fn wifi_patience(action: WifiAction) -> Duration {
    match action {
        WifiAction::Connect | WifiAction::Disconnect => WIFI_PATIENCE,
        WifiAction::Join | WifiAction::Forget => ACTION_TIMEOUT,
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
    /// The networks of the last read, in the order they were listed - the same
    /// role for the wireless tile.
    wifi: Vec<Wifi>,
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

/// The same for a network's row, keyed by the SSID the row is drawn for.
struct WifiPending {
    ssid: String,
    action: WifiAction,
    until: Instant,
}

pub struct StatsView {
    /// The card's content: the shell's card widget is its parent.
    pub root: gtk::Box,
    /// The state chips in the header, rebuilt whenever something changes them.
    chips: gtk::Box,
    /// The gear at the header's right end: the settings window's trigger. Kept so
    /// it can wear the accent while that window is up.
    settings_button: gtk::Button,
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
    /// The wireless link's state, and the switch for the radio behind it.
    network_state: gtk::Label,
    radio_toggle: gtk::Button,
    /// The network tile's Scan chip, kept so it can stop taking clicks while
    /// there is no radio to scan with.
    wifi_scan: gtk::Button,
    /// Where the network rows are built. Rebuilt wholesale on every read, for
    /// the same reason as the device list: the list is what came and went.
    wifi_list: gtk::Box,
    /// What that list says when it has nothing to list ("Wifi is off", the
    /// dashed "no networks in reach" box), the hidden-row count, and the one
    /// line about something that just happened.
    wifi_hint: gtk::Label,
    wifi_more: gtk::Label,
    wifi_note: gtk::Label,
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
    /// The network row whose button is waiting for an answer, if any.
    wifi_pending: RefCell<Option<WifiPending>>,
    /// Set while the brightness slider is being moved *by the program*: a
    /// `set_value` during a refresh is indistinguishable from a drag at the
    /// signal, and would write the backlight back to where it already is.
    scale_guard: Cell<bool>,
    /// How many device rows the list shows - the rest is a count. `main` reads
    /// the same number when it asks for the devices, so no details are read for
    /// rows that will not be drawn.
    max_devices: usize,
    /// The same, for the networks in reach: a wifi list can be long, and the tile
    /// is a panel rather than a window.
    max_networks: usize,
    /// What to run shortly after an action, so the row shows the new state
    /// rather than the old one until the next heartbeat. `main` installs it.
    on_refresh: RefCell<Option<Rc<dyn Fn()>>>,
    /// What the Scan button runs. `main` installs it, because the scan is a
    /// process with a lifetime, and the element - not the view - is what keeps
    /// the clock that notices it ending.
    on_scan: RefCell<Option<Rc<dyn Fn()>>>,
    /// What the network tile's Scan chip runs: a read that asks the card for a
    /// fresh scan (see `sources::refresh_wifi`'s `rescan`).
    on_wifi_scan: RefCell<Option<Rc<dyn Fn()>>>,
    /// The terminal the buttons that need a TTY open (btop, nmtui, pacman -
    /// and a join that has to be given a password).
    terminal: String,
    /// The wireless interface, for the one action that names a device rather
    /// than a profile: letting go of the link.
    interface: String,
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

        // The gear: the settings window's trigger, at the header's right end.
        // It runs `settings_command toggle` and nothing else - the settings
        // element is a program of its own, and its first invocation *is* its
        // daemon, so this is a `launch` and never a wait (see
        // `sources::spawn`). While the window is up the gear stays lit, which
        // is what `state`'s flag is for.
        let settings_button = icon_button(names::SETTINGS, "hypr-osd settings");
        {
            let command = settings.settings_command.clone();
            settings_button.connect_clicked(move |_| sources::spawn(&command, &["toggle"]));
        }
        header.append(&settings_button);

        // ---- resources: three gauges in a row -------------------------------
        let (resources, resources_head) = tile("resources", names::CPU);
        let metrics = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        metrics.add_css_class("metrics");
        let mut bars = Vec::new();
        let mut values = Vec::new();
        for (icon, name) in [
            (names::CPU, "CPU"),
            (names::MEMORY, "MEM"),
            (names::TEMPERATURE, "TEMP"),
        ] {
            let (metric, bar, value) = gauge(icon, name);
            metrics.append(&metric);
            bars.push(bar);
            values.push(value);
        }
        resources.append(&metrics);
        let btop = glyph_chip(names::TERMINAL, "btop", {
            let terminal = settings.terminal_command.clone();
            move || sources::spawn(&terminal, &["-e", "btop"])
        });
        btop.set_tooltip_text(Some("The full picture, in btop"));
        resources_head.append(&stretch());
        resources_head.append(&btop);

        // ---- network: the wireless link, and the networks in reach ----------
        // The twin of the bluetooth tile below it, and deliberately the same
        // shape: the same kinds of heading tool, the same controller row, the
        // same list with a fixed slot at its rows' end - because "join this one"
        // and "connect that one" are the same question about two radios.
        let (network_tile, network_head) = tile("network", names::WIFI);

        // The tile's two tools, in its heading where the other tiles keep
        // theirs. Scan asks for a *fresh* scan (a plain re-read trusts
        // NetworkManager's cache - see `sources::refresh_wifi`), and nmtui is
        // the honest escape hatch for everything this tile does not do: a
        // hidden network, an enterprise login, a static address.
        let wifi_scan = glyph_chip(names::SEARCH, "Scan", || {});
        wifi_scan.set_tooltip_text(Some("Ask the wireless card for a fresh scan"));
        let network_open = glyph_chip(names::TERMINAL, "nmtui", {
            let terminal = settings.terminal_command.clone();
            move || sources::spawn(&terminal, &["-e", "nmtui"])
        });
        network_open.set_tooltip_text(Some("Everything else about the link, in a terminal"));
        network_head.append(&stretch());
        network_head.append(&wifi_scan);
        network_head.append(&network_open);

        // The controller's own row: the link's state on the left, the radio's
        // switch in the same box every row of the bluetooth tile ends with - so
        // a switch here is the same shape, at the same edge, as the one below.
        // One switch only: a wireless card has nothing else to switch.
        let network_state = gtk::Label::new(Some("Not connected"));
        network_state.add_css_class("value");
        network_state.set_xalign(0.0);
        network_state.set_ellipsize(gtk::pango::EllipsizeMode::End);
        network_state.set_max_width_chars(22);
        let radio_toggle = icon_button(names::WIFI_OFF, "Turn the wifi radio on");
        let network_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        network_row.add_css_class("radio-state");
        network_row.append(&network_state);
        network_row.append(&stretch());
        let network_actions = actions();
        network_actions.append(&radio_toggle);
        network_row.append(&network_actions);
        network_tile.append(&network_row);

        // The list, its hint, its count and its note: the device list's
        // four-part shape (see below), because they are the same list of the
        // same kind of thing.
        let wifi_list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        wifi_list.add_css_class("dev-list");
        let wifi_hint = gtk::Label::new(None);
        wifi_hint.add_css_class("dev-hint");
        wifi_hint.set_xalign(0.5);
        wifi_hint.set_visible(false);
        let wifi_more = gtk::Label::new(None);
        wifi_more.add_css_class("dev-more");
        wifi_more.set_xalign(0.0);
        wifi_more.set_visible(false);
        // A join that did not take is the one thing a row cannot say by itself:
        // it goes back to offering Connect with nothing to show for the click.
        let wifi_note = gtk::Label::new(None);
        wifi_note.add_css_class("dev-note");
        wifi_note.set_xalign(0.0);
        wifi_note.set_wrap(true);
        wifi_note.set_visible(false);
        network_tile.append(&wifi_list);
        network_tile.append(&wifi_hint);
        network_tile.append(&wifi_more);
        network_tile.append(&wifi_note);

        // ---- bluetooth: the controller, and the devices it knows ------------
        // A tile of its own, because it is the one control with *content*: the
        // state of the radio, the visibility of the controller, and then a row
        // per device with the one action that makes sense for that device.
        let (bluetooth_tile, bluetooth_head) = tile("bluetooth", names::BLUETOOTH);
        // The tile is the left column's tail, so it takes the column's leftover
        // height: a column that ends in a hole reads as an accident.
        bluetooth_tile.set_vexpand(true);

        // The tile's two tools, at the right end of its *heading*, where the
        // other tiles keep theirs (btop, Install) - and no longer floating under
        // the device list, where nothing else in this collection puts an action.
        let scan = glyph_chip(names::SEARCH, "Scan", || {});
        scan.set_tooltip_text(Some("Look for devices nearby for a few seconds"));
        // The idle content is kept so the spinner that replaces it while a scan
        // runs can be swapped back out when the controller stops looking.
        let scan_idle = scan
            .child()
            .and_downcast::<gtk::Box>()
            .expect("a chip's child is its glyph-and-word row");
        // The tile's "manage" tool: a card of this collection, not a terminal.
        // It is the other half of this tile - every device, with room for its
        // battery, its signal and the two things a row can do - and it replaces
        // the `bluetoothctl` session that used to open here.
        let bluetooth_open = glyph_chip(names::BLUETOOTH, "Manage", {
            let command = settings.bluetooth_command.clone();
            move || sources::spawn(&command, &["toggle"])
        });
        bluetooth_open
            .set_tooltip_text(Some("Every bluetooth device, in the collection's manager"));
        bluetooth_head.append(&stretch());
        bluetooth_head.append(&scan);
        bluetooth_head.append(&bluetooth_open);

        // The controller's own row: the reading on the left, its two switches in
        // the same box every row of the controls tile ends with - so a switch
        // here is the same shape, at the same edge, as the one beside NETWORK.
        let bluetooth_state = gtk::Label::new(Some("Off"));
        bluetooth_state.add_css_class("value");
        bluetooth_state.set_xalign(0.0);
        // Both halves of "is the controller approachable": whether it is on, and
        // whether it answers pairing requests while it is. Each one shows the
        // state it is *in*, like every other switch in this panel.
        let bluetooth_visible = icon_button(
            names::EYE_OFF,
            "Hidden - answer pairing requests so new devices can find this one",
        );
        let bluetooth_power = icon_button(names::POWER, "Turn bluetooth on");
        let bluetooth_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bluetooth_row.add_css_class("radio-state");
        bluetooth_row.append(&bluetooth_state);
        bluetooth_row.append(&stretch());
        let bluetooth_actions = actions();
        bluetooth_actions.append(&bluetooth_visible);
        bluetooth_actions.append(&bluetooth_power);
        bluetooth_row.append(&bluetooth_actions);
        bluetooth_tile.append(&bluetooth_row);

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

        // ---- updates: what is waiting, and the one way to act on it ---------
        let (updates, updates_head) = tile("updates", names::UPDATES);
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
        let install = glyph_chip(names::UPDATES, "Install", {
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
        left.append(&network_tile);
        left.append(&bluetooth_tile);

        // ---- controls: one row per setting -------------------------------
        // The tile is a table, and the table is the design: a mark and a name on
        // the left, whatever the setting *is* on the right. One setting per
        // row, the same shape all the way down.
        let (controls, _) = tile("controls", names::CONTROLS);

        // (The wireless link is not a row here any more: it is a tile of its
        // own in the column beside this one, because the networks in reach need
        // more room than a row can give them - see the network tile above.)

        // SOUND: the sink's level as the reading, the mixer and the mute as the
        // controls. The volume *step* belongs to the volume element; what is
        // here is what the pill's right and middle clicks used to be.
        let sound_state = gtk::Label::new(Some("—"));
        sound_state.add_css_class("value");
        let (sound_row, sound_actions) = control_row(names::VOLUME_HIGH, "SOUND", &sound_state);
        let mixer = icon_button(names::CONTROLS, "pavucontrol - per-application levels");
        mixer.connect_clicked(move |_| sources::spawn("pavucontrol", &[]));
        let mute_toggle = icon_button(names::VOLUME_HIGH, "Mute");
        sound_actions.append(&mixer);
        sound_actions.append(&mute_toggle);
        controls.append(&sound_row);

        // The sound is the one you *use*; the four under it are the settings you
        // *set*. A hairline is all the separation that needs - the rows
        // themselves are already the same shape.
        controls.append(&hairline());

        // POWER: one button per profile, the lit one being the profile in force.
        // The switch takes the rest of the row, so the three profiles are three
        // equal targets - direct, with no guessing what the next click would
        // cycle to.
        let mut profile = Vec::new();
        let profile_row = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        profile_row.add_css_class("segmented");
        profile_row.set_hexpand(true);
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
            button.set_hexpand(true);
            profile_row.append(&button);
            profile.push(button);
        }
        controls.append(&wide_row(names::POWER_MODE, "POWER", &profile_row));

        // PRESENTATION: hold an idle/sleep block for as long as it is on. The
        // chip carries its own word ("off", "on") because it is a switch with
        // nothing else to say - which puts it in the reading column with the
        // access point and the volume, where a state belongs.
        let presentation = glyph_chip(names::PRESENTATION, "off", || {});
        presentation.set_tooltip_text(Some(
            "Hold a systemd inhibitor (idle and sleep) - what the old \
             idle_inhibitor module did, with a switch that goes away when this \
             element does",
        ));
        let (presentation_row, _) = control_row(names::PRESENTATION, "PRESENTATION", &presentation);
        controls.append(&presentation_row);

        // BRIGHTNESS: the slider takes the rest of the row and the percentage
        // sits at its end, where the reading of a slider belongs.
        let brightness_value = gtk::Label::new(Some("—"));
        brightness_value.add_css_class("value");
        brightness_value.set_xalign(1.0);
        brightness_value.set_size_request(38, -1);
        let brightness_scale =
            gtk::Scale::with_range(gtk::Orientation::Horizontal, 1.0, 100.0, 1.0);
        brightness_scale.set_draw_value(false);
        brightness_scale.set_size_request(80, -1);
        brightness_scale.set_hexpand(true);
        brightness_scale.set_valign(gtk::Align::Center);
        let brightness_row = wide_row(names::BRIGHTNESS, "BRIGHTNESS", &brightness_scale);
        brightness_row.append(&brightness_value);
        controls.append(&brightness_row);

        // LAYOUT: the bare code on the value side.
        let layout_value = gtk::Label::new(Some("—"));
        layout_value.add_css_class("value");
        layout_value.set_size_request(30, -1);
        layout_value.set_xalign(1.0);
        let (layout_row, _) = control_row(names::KEYBOARD, "LAYOUT", &layout_value);
        controls.append(&layout_row);

        let right = gtk::Box::new(gtk::Orientation::Vertical, 8);
        right.add_css_class("column");
        right.set_size_request(settings.right_width, -1);
        right.append(&controls);
        right.append(&updates);
        // The right column is the shorter of the two now that the left carries
        // both radios, so its tail takes the leftover height and the two columns
        // end on the same line - the same reason the bluetooth tile has it.
        updates.set_vexpand(true);

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
            settings_button,
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
            wifi_scan,
            wifi_list,
            wifi_hint,
            wifi_more,
            wifi_note,
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
            wifi_pending: RefCell::new(None),
            scale_guard: Cell::new(true),
            max_devices: settings.max_devices,
            max_networks: settings.max_networks,
            on_refresh: RefCell::new(None),
            on_scan: RefCell::new(None),
            on_wifi_scan: RefCell::new(None),
            terminal: settings.terminal_command.clone(),
            interface: settings.interface.clone(),
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

    /// Install what the network tile's Scan chip runs: a fresh scan, then the
    /// list again - `main` owns the read, so only `main` can do either.
    pub fn on_wifi_scan(&self, hook: Rc<dyn Fn()>) {
        *self.on_wifi_scan.borrow_mut() = Some(hook);
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

        connect_chip(&self.wifi_scan, self.me(), move |view| {
            // Cloned out of the cell first, for the same reason the bluetooth
            // Scan button does it: the hook re-reads and repaints the panel, and
            // that is exactly the kind of re-entry a live borrow panics on.
            let hook = view.on_wifi_scan.borrow().clone();
            if let Some(hook) = hook {
                hook();
            }
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
        // The visibility switch says what the controller is doing ("eye" while
        // it answers pairing requests, "eye-off" while it does not) and lights in
        // the accent while it is holding itself open - the same statement the
        // wifi switch in the controls tile makes about its radio.
        set_icon(
            &self.bluetooth_visible,
            if bluetooth.discoverable {
                names::EYE
            } else {
                names::EYE_OFF
            },
        );
        self.bluetooth_visible
            .set_tooltip_text(Some(if bluetooth.discoverable {
                "Visible - stop answering pairing requests"
            } else {
                "Hidden - answer pairing requests so new devices can find this one"
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
                // it belongs to the bluetooth manager (one button away, above).
                self.device_more.set_text(&format!(
                    "+{hidden} more - see all in the bluetooth manager"
                ));
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

        let icon = icons::lucide(device.icon.icon(), DEVICE_ICON);
        icon.add_css_class("dev-icon");
        // The badge is a square a good deal wider than the drawing inside it,
        // and a `GtkBox` hands a child its natural width at the box's *start* -
        // so the mark would sit against the badge's left edge with all the
        // padding on its right. Asking for the slack is what centres it:
        // `GtkImage` draws its icon in the middle of whatever room it is given
        // (which is also why it is already centred vertically), and a drawing
        // has no font-size to be centred with.
        icon.set_hexpand(true);
        let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        badge.add_css_class("dev-badge");
        badge.set_valign(gtk::Align::Center);
        // ...but the badge must refuse the slack for itself, or it takes the
        // icon's request as its own (a `GtkBox` inherits expandability from its
        // children) and grows into a wide rectangle around a 24px square's
        // worth of padding.
        badge.set_hexpand(false);
        badge.append(&icon);
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
            let icon = icons::lucide(names::BATTERY_FULL, BATTERY_ICON);
            icon.add_css_class("batt-icon");
            let value = gtk::Label::new(Some(&format!("{battery}%")));
            value.add_css_class("batt-value");
            pill.append(&icon);
            pill.append(&value);
            row.append(&pill);
        }

        let action = device.action();
        let waiting = self.pending_for(&device.address);
        // The row's buttons go in a box of their own, and that box is held at the
        // width two buttons need - the action the device's state asks for, and
        // the forget that a paired, unconnected one offers beside it. It is what
        // keeps the readings to its left in a column: the meter and the battery
        // are packed against this box, so a row that carries one button and a row
        // that carries two put them in the same place.
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        buttons.add_css_class("actions");
        buttons.set_hexpand(false);
        buttons.set_size_request(DEVICE_ACTIONS, -1);
        buttons.set_valign(gtk::Align::Center);
        buttons.append(&stretch());

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
            let icon = icons::lucide(action.icon(), BUTTON_ICON);
            button.set_child(Some(&icon));
            button.set_tooltip_text(Some(action.tooltip()));
            set_class(&button, "on", device.connected);
            let view = self.me();
            let address = device.address.clone();
            button.connect_clicked(move |_| view.run_device_action(&address, action));
        }
        buttons.append(&button);

        // The paired ones a scan no longer sees are exactly the ones worth
        // forgetting - so the forget button lives on a paired, unconnected row,
        // next to its Connect. Connected rows do not carry it: forgetting a
        // live link is never the first thing to want.
        if device.paired && !device.connected && !waiting {
            let remove = icon_button(DeviceAction::Remove.icon(), DeviceAction::Remove.tooltip());
            remove.add_css_class("danger");
            let view = self.me();
            let address = device.address.clone();
            remove.connect_clicked(move |_| view.run_device_action(&address, DeviceAction::Remove));
            buttons.append(&remove);
        }
        row.append(&buttons);

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
            // is a device that will not pair without a passkey. The manager card
            // uses the same agent, so this is where the limit is named.
            self.note(
                &self.device_note,
                "Could not pair - the device may want a passkey (bluetoothctl can ask for one)",
            );
        }
        self.rebuild_devices();
        self.refresh_soon(200);
    }

    /// Show the one-line note under a list, and take it away again by itself: it
    /// reports something that just happened, not a state. Both radio tiles have
    /// one, so the label is the caller's to name.
    fn note(&self, label: &gtk::Label, text: &str) {
        label.set_text(text);
        label.set_visible(true);
        let label = label.clone();
        glib::timeout_add_local_once(NOTE_TIME, move || label.set_visible(false));
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

    // ---- the wireless tile ----------------------------------------------

    /// The networks in reach: the rows, and the hint and count that go with
    /// them.
    pub fn render_wifi(&self, networks: Vec<Wifi>) {
        self.last.borrow_mut().wifi = networks;
        self.rebuild_wifi();
    }

    /// Rebuild the network rows from the last read.
    ///
    /// Rebuilt whole rather than diffed, exactly like the device list: networks
    /// come and go, a handful of labels is cheaper to remake than to reconcile,
    /// and this is what a click re-runs - so a button that was pressed stops
    /// looking pressable without waiting for the next read.
    fn rebuild_wifi(&self) {
        self.prune_wifi_pending();
        while let Some(child) = self.wifi_list.first_child() {
            self.wifi_list.remove(&child);
        }
        let last = self.last.borrow();
        let (radio, present) = (last.radio, last.present);
        let networks = last.wifi.clone();

        let hint = match (present, radio, networks.is_empty()) {
            (false, ..) => Some("No wireless card"),
            (_, false, _) => Some("Wifi is off"),
            (true, true, true) => Some("No networks in reach - scan to look again"),
            (true, true, false) => None,
        };
        self.wifi_hint.set_visible(hint.is_some());
        if let Some(hint) = hint {
            self.wifi_hint.set_text(hint);
        }
        // The dashed box is the "there is a radio and it heard nothing" state,
        // not a frame around the plain "no card" line.
        set_class(
            &self.wifi_hint,
            "empty",
            present && radio && networks.is_empty(),
        );
        self.wifi_list.set_visible(present && radio);

        if present && radio {
            for network in networks.iter().take(self.max_networks) {
                self.wifi_list.append(&self.wifi_row(network));
            }
            let hidden = networks.len().saturating_sub(self.max_networks);
            self.wifi_more.set_visible(hidden > 0);
            if hidden > 0 {
                // The count, not the names: the list is a list, and the rest of
                // it belongs to nmtui (one button away, in the heading).
                self.wifi_more
                    .set_text(&format!("+{hidden} more - see all in nmtui"));
            }
        } else {
            self.wifi_more.set_visible(false);
        }
    }

    /// One network: what it is, how strong its signal is, and the action that
    /// follows from its state - plus a way to forget it.
    ///
    /// The same row shape as a device's, down to the fixed-width slot at the end,
    /// because it answers the same question: what is here, and what can I do
    /// with it.
    fn wifi_row(&self, network: &Wifi) -> gtk::Box {
        let mark = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        mark.add_css_class("dev-mark");
        mark.set_valign(gtk::Align::Center);
        set_class(&mark, "live", network.connected);

        let icon = icons::lucide(network.icon(), DEVICE_ICON);
        icon.add_css_class("dev-icon");
        // Both lines of the badge trick are in `device_row`; a badge has to take
        // the slack to centre its drawing and refuse it for itself.
        icon.set_hexpand(true);
        let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        badge.add_css_class("dev-badge");
        badge.set_valign(gtk::Align::Center);
        badge.set_hexpand(false);
        badge.append(&icon);
        set_class(&badge, "live", network.connected);

        let name = gtk::Label::new(Some(&network.ssid));
        name.add_css_class("dev-name");
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        // Capped for the same reason the device names are: a long name must not
        // set the card's width, which is what the hover zone is measured against.
        // Tighter than a device's, because an SSID is up to 32 bytes and is
        // therefore the row that would decide the width for both tiles - and the
        // whole name is still in the row's tooltip.
        name.set_max_width_chars(WIFI_NAME_CHARS);
        let detail = gtk::Label::new(Some(&network.detail()));
        detail.add_css_class("dev-sub");
        detail.set_xalign(0.0);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        text.append(&name);
        text.append(&detail);

        // The signal as bars, like a device's link quality: the same meter, at
        // the same size, reading the same way.
        let signal = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        signal.add_css_class("dev-sig");
        signal.set_valign(gtk::Align::Center);
        let bars = network.bars() as usize;
        let on = gtk::Label::new(Some(&SIGNAL_BARS.chars().take(bars).collect::<String>()));
        on.add_css_class("sig-on");
        let off = gtk::Label::new(Some(&SIGNAL_BARS.chars().skip(bars).collect::<String>()));
        off.add_css_class("sig-off");
        signal.append(&on);
        signal.append(&off);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("dev");
        row.append(&mark);
        row.append(&badge);
        row.append(&text);
        row.append(&signal);

        let action = network.action();
        let waiting = self.wifi_pending_for(&network.ssid);
        // The same two-button slot the device rows hold open, so a row carrying
        // one button and a row carrying two put their meters and their buttons in
        // the same place.
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        buttons.add_css_class("actions");
        buttons.set_hexpand(false);
        buttons.set_size_request(DEVICE_ACTIONS, -1);
        buttons.set_valign(gtk::Align::Center);
        buttons.append(&stretch());

        let button = gtk::Button::new();
        button.add_css_class("iconbtn");
        button.set_focus_on_click(false);
        button.set_can_focus(false);
        if waiting {
            let spinner = gtk::Spinner::new();
            spinner.add_css_class("btn-spinner");
            spinner.start();
            button.set_child(Some(&spinner));
            button.set_sensitive(false);
            button.set_tooltip_text(Some("Talking to the network…"));
        } else {
            button.set_child(Some(&icons::lucide(action.icon(), BUTTON_ICON)));
            button.set_tooltip_text(Some(action.tooltip()));
            set_class(&button, "on", network.connected);
            let view = self.me();
            let ssid = network.ssid.clone();
            let profile = network.profile.clone();
            button
                .connect_clicked(move |_| view.run_wifi_action(&ssid, profile.as_deref(), action));
        }
        buttons.append(&button);

        // A stored profile that is not the link right now is exactly the one
        // worth forgetting - so the forget button lives there, beside Join, and
        // never on the row of the network carrying the session.
        if network.profile.is_some() && !network.connected && !waiting {
            let forget = icon_button(WifiAction::Forget.icon(), WifiAction::Forget.tooltip());
            forget.add_css_class("danger");
            let view = self.me();
            let ssid = network.ssid.clone();
            let profile = network.profile.clone();
            forget.connect_clicked(move |_| {
                view.run_wifi_action(&ssid, profile.as_deref(), WifiAction::Forget)
            });
            buttons.append(&forget);
        }
        row.append(&buttons);

        // The raw signal moves to the tooltip, as a device's dBm does: a number
        // nobody can calibrate at a glance is worse than the meter.
        row.set_tooltip_text(Some(&match &network.band {
            Some(band) => format!("{} · {}% · {band}", network.ssid, network.signal),
            None => format!("{} · {}%", network.ssid, network.signal),
        }));
        row
    }

    /// Run a network's action, and say so until the answer lands.
    ///
    /// `profile` is what the row knows about a stored profile - a Forget or a
    /// disconnect acts on it when there is one, and the SSID (`ssid`) is always
    /// the row's own name, which is what the pending spinner is keyed by.
    fn run_wifi_action(&self, ssid: &str, profile: Option<&str>, action: WifiAction) {
        match action {
            WifiAction::Connect => sources::connect_wifi(ssid),
            // The device rather than the profile: a profile can be renamed by
            // hand, the interface cannot.
            WifiAction::Disconnect => sources::disconnect_wifi(&self.interface),
            WifiAction::Join => {
                // The password has to be typed, and a pointer-driven panel has
                // nowhere to put a field - so the join happens in a terminal,
                // where `nmcli --ask` can ask for it. Nothing to wait for here:
                // the terminal owns it from this point on.
                sources::spawn(
                    &self.terminal,
                    &["-e", "nmcli", "--ask", "device", "wifi", "connect", ssid],
                );
                return;
            }
            WifiAction::Forget => match profile {
                Some(profile) => sources::forget_wifi(profile),
                // Only drawn on a row that has one; nothing to delete otherwise.
                None => return,
            },
        }
        *self.wifi_pending.borrow_mut() = Some(WifiPending {
            ssid: ssid.to_string(),
            action,
            until: Instant::now() + wifi_patience(action),
        });
        self.rebuild_wifi();
        self.refresh_soon(1200);
    }

    /// Whether this network's row is waiting for an answer.
    fn wifi_pending_for(&self, ssid: &str) -> bool {
        self.wifi_pending
            .borrow()
            .as_ref()
            .is_some_and(|pending| pending.ssid == ssid)
    }

    /// Drop a row's spinner once the network has done what was asked of it, or
    /// once the wait has gone on long enough that the answer is clearly not
    /// coming - and say so, because a join that quietly goes back to offering
    /// "Connect" is what a broken button looks like.
    fn prune_wifi_pending(&self) {
        let expired = {
            let mut pending = self.wifi_pending.borrow_mut();
            let Some(waiting) = pending.as_ref() else {
                return;
            };
            let network = self
                .last
                .borrow()
                .wifi
                .iter()
                .find(|network| network.ssid == waiting.ssid)
                .cloned();
            let settled = match (network, waiting.action) {
                // Out of reach (or off the air): whatever was asked of it, there
                // is no row left to show.
                (None, _) => true,
                (Some(network), WifiAction::Connect) => network.connected,
                (Some(network), WifiAction::Disconnect) => !network.connected,
                // No stored profile left: the forget took.
                (Some(network), WifiAction::Forget) => network.profile.is_none(),
                (Some(_), WifiAction::Join) => true,
            };
            if settled {
                *pending = None;
                return;
            }
            if Instant::now() < waiting.until {
                return;
            }
            // Out of patience. A join is the one that had a reason worth naming:
            // usually a wrong password, but from here there is no way to tell,
            // and nmtui can.
            let note = (waiting.action == WifiAction::Connect)
                .then(|| format!("Could not join {} - try nmtui", waiting.ssid));
            *pending = None;
            note
        };
        if let Some(note) = expired {
            self.note(&self.wifi_note, &note);
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
        // The switch wears the state it is *in* - the whole wifi mark while the
        // radio is on, the struck-through one while it is not - and the tooltip
        // says which way a click would take it.
        set_icon(
            &self.radio_toggle,
            if radio { names::WIFI } else { names::WIFI_OFF },
        );
        set_class(&self.radio_toggle, "on", radio);
        self.radio_toggle.set_tooltip_text(Some(if radio {
            "The wifi radio is on - turn it off"
        } else {
            "The wifi radio is off - turn it on"
        }));
        if !present {
            self.network_state.set_text("no wireless card");
            self.radio_toggle.set_sensitive(false);
            self.wifi_scan.set_sensitive(false);
            // The list's hint is drawn from the two flags above, so it follows
            // them rather than the (slower) network read.
            self.rebuild_wifi();
            return;
        }
        self.radio_toggle.set_sensitive(true);
        self.wifi_scan.set_sensitive(radio);
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
        // The list is drawn from the radio's state as well as from the networks,
        // and this is the read that says whether there is a radio at all.
        self.rebuild_wifi();
    }

    pub fn render_sound(&self, volume: Option<Volume>) {
        self.last.borrow_mut().volume = volume;
        match volume {
            Some(volume) => {
                // The reading says the level *and* whether it is being heard:
                // being muted is the one thing the number cannot say by itself.
                self.sound_state.set_text(&if volume.muted {
                    format!("{}% muted", volume.percent)
                } else {
                    format!("{}%", volume.percent)
                });
                set_class(&self.sound_state, "warn", volume.muted);
                set_icon(
                    &self.mute_toggle,
                    if volume.muted {
                        names::VOLUME_OFF
                    } else {
                        names::VOLUME_HIGH
                    },
                );
                set_class(&self.mute_toggle, "on", !volume.muted);
                self.mute_toggle.set_tooltip_text(Some(if volume.muted {
                    "Muted - make the sink audible again"
                } else {
                    "Sound is on - mute the sink"
                }));
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
        // The gear wears the accent while the settings window is up: the panel is
        // the handle that opened it, so it should say that it did.
        set_class(
            &self.settings_button,
            "on",
            state::read(&state::settings_panel()),
        );
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
fn tile(title: &str, icon: &str) -> (gtk::Box, gtk::Box) {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    line.add_css_class("section-row");
    line.append(&section_icon(icon));
    line.append(&section_label(title));

    let tile = gtk::Box::new(gtk::Orientation::Vertical, 6);
    tile.add_css_class("tile");
    tile.append(&line);
    (tile, line)
}

/// A tile's or a row's mark, at the size a heading's word needs. The size is
/// here and not in the stylesheet because a drawing has no font-size; CSS only
/// carries the colour it takes from the word beside it.
fn section_icon(icon: &str) -> gtk::Image {
    let mark = icons::lucide(icon, SECTION_ICON);
    mark.add_css_class("section-icon");
    mark
}

/// The word beside a mark: the same small caps on a tile's heading and on every
/// row of the tile, because they are the same kind of thing - a heading, once.
fn section_label(name: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(name));
    label.add_css_class("section");
    label
}

/// The hairline between two clusters of rows inside one tile.
fn hairline() -> gtk::Separator {
    let line = gtk::Separator::new(gtk::Orientation::Horizontal);
    line.add_css_class("rule");
    line
}

/// One resource gauge: icon and name, the reading, and the meter under it.
///
/// An empty box that takes the slack, so three of them share the tile's width
/// evenly whatever the numbers say - which is what keeps the three readings in
/// a row instead of in a queue.
fn gauge(icon: &str, name: &str) -> (gtk::Box, gtk::ProgressBar, gtk::Label) {
    let meter = gtk::ProgressBar::new();
    meter.add_css_class("meter");
    meter.set_show_text(false);
    meter.set_valign(gtk::Align::Center);

    let value = gtk::Label::new(Some("—"));
    value.add_css_class("metric-value");
    value.set_xalign(0.0);

    let icon = icons::lucide(icon, METRIC_ICON);
    icon.add_css_class("metric-icon");
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

/// One row of the controls table: the mark and the name, the reading against
/// the right edge, and - handed back in the second box - whatever the row has
/// to press.
///
/// The two halves come back separately because the reading is what makes a row
/// a row of this table while a control is what a row *may* have. A row with
/// nothing to press drops the box it is handed, so its reading lands against
/// the row's own right edge; the rows that do carry controls keep their reading
/// one control's width in, and - because the pair of buttons is the same width
/// in both of them - those readings line up with each other too. That is the
/// whole table: readings in a column, controls in a column, one hairline
/// between the two families.
fn control_row(icon: &str, name: &str, value: &impl IsA<gtk::Widget>) -> (gtk::Box, gtk::Box) {
    let row = row_head(icon, name);
    row.append(&stretch());
    row.append(value);

    let actions = actions();
    row.append(&actions);
    (row, actions)
}

/// A row whose control is given the *rest* of the row rather than the slot at
/// its end: the three-way switch and the slider both need the room more than
/// they need to line up in the column the buttons share.
fn wide_row(icon: &str, name: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = row_head(icon, name);
    row.append(control);
    row
}

/// The left end of any control row: the mark and the name, spaced like a tile's
/// heading - because that is what they are, once per row.
fn row_head(icon: &str, name: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("control-row");
    row.append(&section_icon(icon));
    row.append(&section_label(name));
    row
}

/// An empty box that takes the slack in a row: the popup's own `spacer` - the
/// thing that keeps two widgets apart without a hard-coded width.
fn stretch() -> gtk::Box {
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    spacer
}

/// The box a row's controls sit in, at the row's right end.
///
/// It is explicitly *not* expandable: a `GtkBox` inherits expansion from the
/// children that want it (the spacer inside, which holds a lone chip against the
/// right edge), and without this the box would swallow slack of its own and drag
/// the reading to its left out of its column. Every row of both tiles - the
/// controls and the controller, and each device - ends with one of these, so
/// every switch and every button in the popup shares one right edge.
fn actions() -> gtk::Box {
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    actions.add_css_class("actions");
    actions.set_hexpand(false);
    actions.set_valign(gtk::Align::Center);
    actions.append(&stretch());
    actions
}

/// A chip with a leading icon and a word: the popup's labelled button, and the
/// shape a *tile's* own action wears (btop, Install, Scan) where a row's
/// controls are round and wordless. The icon says what kind of thing the button
/// is (a terminal, a magnifier) and the word says what it does, so the word is
/// reachable through [`word_of`] when it has to change ("Wi-Fi off" → "Wi-Fi
/// on").
fn glyph_chip(icon: &str, text: &str, action: impl Fn() + 'static) -> gtk::Button {
    let icon = icons::lucide(icon, BUTTON_ICON);
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

/// A round button whose whole content is one icon: the popup's *direct*
/// controls - the bluetooth power, the action at the end of a device's row. The
/// tooltip carries the words, because there is nowhere else for them to go.
fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("iconbtn");
    button.set_child(Some(&icons::lucide(icon, BUTTON_ICON)));
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

/// Re-point an [`icon_button`]'s drawing at another icon - how a switch that
/// shows its *state* (a radio that is on, a sink that is muted, a controller
/// that is visible) follows the reading without the button being rebuilt.
fn set_icon(button: &gtk::Button, icon: &str) {
    let image = button
        .child()
        .and_downcast::<gtk::Image>()
        .expect("an icon button's child is its drawing");
    icons::set_lucide(&image, icon, BUTTON_ICON);
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

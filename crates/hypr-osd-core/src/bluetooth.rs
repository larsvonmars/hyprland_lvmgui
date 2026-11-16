//! Bluetooth: the controller, the devices BlueZ knows, and the handful of
//! actions a card can take on them.
//!
//! Everything here is `bluetoothctl`. BlueZ itself speaks D-Bus, and its
//! `ObjectManager` would answer all of this in one call - but its variants are a
//! nest of dictionaries that would have to be decoded and kept in step with
//! whatever BlueZ version is installed, while `bluetoothctl` has one command per
//! question and its output is a stable, line-oriented format. The collection's
//! rule is the same one the audio reads follow (`wpctl`): ask the tool the
//! desktop already ships instead of binding to a daemon's wire format.
//!
//! Three elements draw this: the system popup's compact bluetooth tile, the
//! dedicated manager card (`hypr-osd-bluetooth`), and the `status` verbs both of
//! them answer. So the model, the parser and the commands live here rather than
//! in whichever element happened to need them first - one parser, one answer to
//! "what is this device doing".
//!
//! Two details are easy to get wrong and are pinned by tests below:
//!
//! * `devices Paired` and `devices Connected` name only devices that were paired
//!   somewhere before. The plain `devices` list is the superset, and it is what
//!   makes pairing a freshly scanned device possible at all.
//! * `pair` exits 0 whether or not it worked, so the outcome has to be asked for
//!   (`info`) rather than inferred from the exit status.
//!
//! Everything slow is asynchronous. A `bluetoothctl` call can take a few hundred
//! milliseconds, and a card that froze its own drawing and pointer handling for
//! that long would feel broken; the reads therefore go through [`crate::output`],
//! which hands the answer back on the main loop. Only [`now`] blocks, because it
//! exists for `status` verbs, where blocking is exactly what is wanted.

use std::cell::Cell;
use std::collections::VecDeque;
use std::process::{Command, Stdio};
use std::rc::Rc;

use gtk::glib;

use crate::hardware;
use crate::icons::names;
use crate::output;

/// What the controller is doing, as far as a header and its rows go.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bluetooth {
    /// Whether a controller answered at all. `show` prints nothing without one
    /// (no BlueZ daemon, no adapter), which is a different thing from a radio
    /// that is switched off.
    pub present: bool,
    pub powered: bool,
    /// Whether the controller answers to pairing requests right now - the
    /// "visible as …" half of the header.
    pub discoverable: bool,
    /// How many devices are connected right now - the number that makes "on"
    /// mean something.
    pub connected: usize,
    /// The controller's own name (`Alias:` or `Name:` in `show`), empty when it
    /// did not answer. The manager shows it under the title.
    pub alias: String,
}

impl Bluetooth {
    /// The state as the BLUETOOTH row reads it.
    pub fn state(&self) -> String {
        if !self.present {
            return "No controller".to_string();
        }
        match (self.powered, self.connected) {
            (false, _) => "Off".to_string(),
            (true, 0) => "On".to_string(),
            (true, count) => format!("On · {count} connected"),
        }
    }

    /// What the power button does, as the word for its tooltip.
    pub fn action(&self) -> &'static str {
        if self.powered {
            "Turn off"
        } else {
            "Turn on"
        }
    }

    /// What the visibility chip says: "Visible" while the controller answers
    /// pairing requests, "Hidden" otherwise.
    pub fn visibility(&self) -> &'static str {
        if self.discoverable {
            "Visible"
        } else {
            "Hidden"
        }
    }

    /// Whether the controller is there and can be asked to do anything.
    pub fn usable(&self) -> bool {
        self.present && self.powered
    }
}

/// The class of a device, as BlueZ publishes it in the `Icon` property - the
/// only thing that tells a headset from a keyboard without asking for a picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceIcon {
    Headphones,
    Speaker,
    Keyboard,
    Mouse,
    Phone,
    Computer,
    Gamepad,
    Watch,
    Camera,
    Printer,
    Display,
    Network,
    /// Everything BlueZ did not name, and every device whose `info` was not
    /// read: a row always gets an icon, because a row without one reads
    /// broken.
    #[default]
    Other,
}

impl DeviceIcon {
    /// The icon the row shows, from the collection's shared vocabulary - the
    /// same drawings the system popup's own headings use, so a keyboard is a
    /// keyboard wherever it appears.
    ///
    /// Two of these had one drawing between them before (`Computer` and
    /// `Display` were the desktop and the monitor of the same font), which at a
    /// badge's size is a row you cannot tell from its neighbour; Lucide has a
    /// mark for each.
    pub fn icon(self) -> &'static str {
        match self {
            DeviceIcon::Headphones => names::HEADPHONES,
            DeviceIcon::Speaker => names::SPEAKER,
            DeviceIcon::Keyboard => names::KEYBOARD,
            DeviceIcon::Mouse => names::MOUSE,
            DeviceIcon::Phone => names::SMARTPHONE,
            DeviceIcon::Computer => names::MONITOR,
            DeviceIcon::Gamepad => names::GAMEPAD,
            DeviceIcon::Watch => names::WATCH,
            DeviceIcon::Camera => names::CAMERA,
            DeviceIcon::Printer => names::PRINTER,
            DeviceIcon::Display => names::DISPLAY,
            DeviceIcon::Network => names::WIFI,
            // BlueZ named nothing (or its `info` was not read): the bluetooth
            // mark itself, which is at least honest about the radio the row
            // came from.
            DeviceIcon::Other => names::BLUETOOTH,
        }
    }
}

impl From<&str> for DeviceIcon {
    /// BlueZ's `Icon:` is a freedesktop icon *name* (`audio-headphones`,
    /// `input-keyboard`, …), so the family is the prefix and only the suffix
    /// tells the variants apart. Matching on the prefix means a class BlueZ
    /// spells differently later still lands in the right family.
    fn from(icon: &str) -> Self {
        let name = icon.trim().to_ascii_lowercase();
        if name.starts_with("audio-head") {
            DeviceIcon::Headphones
        } else if name.starts_with("audio-") || name.starts_with("multimedia") {
            DeviceIcon::Speaker
        } else if name.starts_with("input-keyboard") {
            DeviceIcon::Keyboard
        } else if name.starts_with("input-mouse") || name.starts_with("input-tablet") {
            DeviceIcon::Mouse
        } else if name.starts_with("input-gaming") {
            DeviceIcon::Gamepad
        } else if name == "phone" || name == "modem" {
            DeviceIcon::Phone
        } else if name == "computer" {
            DeviceIcon::Computer
        } else if name == "watch" {
            DeviceIcon::Watch
        } else if name.starts_with("camera-") {
            DeviceIcon::Camera
        } else if name == "printer" || name == "scanner" {
            DeviceIcon::Printer
        } else if name == "video-display" {
            DeviceIcon::Display
        } else if name.starts_with("network-") {
            DeviceIcon::Network
        } else {
            DeviceIcon::Other
        }
    }
}

/// What the button at the end of a device's row does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceAction {
    /// A device the controller can see but which is not paired yet.
    Pair,
    Connect,
    Disconnect,
    /// Forget a paired device. Never the row's primary action - it is the
    /// secondary button next to Connect, because pairing the device again is
    /// the one thing here that cannot be undone from this panel.
    Remove,
}

impl DeviceAction {
    /// The icon at the end of a device's row. The tooltip says what the mark
    /// means, because a row's button has no room for words.
    pub fn icon(self) -> &'static str {
        match self {
            DeviceAction::Pair => names::PLUS,
            DeviceAction::Connect => names::LINK,
            DeviceAction::Disconnect => names::UNLINK,
            DeviceAction::Remove => names::CLOSE,
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            DeviceAction::Pair => {
                "Pair, trust and connect - a device that insists on a passkey \
                 cannot be paired from here"
            }
            DeviceAction::Connect => "Connect",
            DeviceAction::Disconnect => "Disconnect",
            DeviceAction::Remove => "Forget this device - unpair it",
        }
    }
}

/// One device, as a card lists it: what it is, what it is doing, and which
/// button its row carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub address: String,
    pub name: String,
    pub icon: DeviceIcon,
    pub connected: bool,
    pub paired: bool,
    pub trusted: bool,
    /// The link quality, when the device is connected and BlueZ reports it.
    pub rssi: Option<i32>,
    /// The device's own battery, when it publishes one (headsets usually do).
    pub battery: Option<i32>,
}

impl Device {
    /// The name to show. BlueZ answers with an empty alias for a device it knows
    /// the address of but has no name for, and an unnamed row is a row nobody
    /// can pick out.
    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            &self.address
        } else {
            &self.name
        }
    }

    /// The line under the name: what the device is doing. The link quality and
    /// the battery have their own widgets next to this line, so they stay out
    /// of it.
    pub fn detail(&self) -> String {
        match (self.connected, self.paired) {
            (true, _) => "connected".to_string(),
            (false, true) => "paired".to_string(),
            (false, false) => "nearby".to_string(),
        }
    }

    /// The line under the name in the manager: what the device is doing, with
    /// its address behind it - the address is what a `connect` verb takes, so a
    /// card that shows only a name can never be reproduced from a script.
    pub fn subtitle(&self) -> String {
        let state = match (self.connected, self.paired) {
            (true, true) => "Connected",
            (true, false) => "Connected (not paired)",
            (false, true) => "Paired",
            (false, false) => "Nearby",
        };
        format!("{state} · {}", self.address)
    }

    /// The link quality as 0–4 bars, for the little meter next to the name.
    /// `None` when BlueZ reports no RSSI.
    pub fn signal(&self) -> Option<u8> {
        let rssi = self.rssi?;
        Some(match rssi {
            rssi if rssi >= -50 => 4,
            rssi if rssi >= -60 => 3,
            rssi if rssi >= -70 => 2,
            rssi if rssi >= -80 => 1,
            _ => 0,
        })
    }

    /// The battery drawing that matches the reading, so a nearly empty cell
    /// reads as one at a glance rather than as a number to parse.
    pub fn battery_icon(&self) -> &'static str {
        match self.battery {
            Some(level) if level <= 10 => names::BATTERY_EMPTY,
            Some(level) if level < 30 => names::BATTERY_LOW,
            Some(level) if level < 70 => names::BATTERY_MEDIUM,
            _ => names::BATTERY_FULL,
        }
    }

    /// Which button the row carries, which follows from what the device is
    /// doing: a connected one can only be let go, a paired one can only be
    /// picked up, and one that is neither has to be paired first.
    pub fn action(&self) -> DeviceAction {
        match (self.connected, self.paired) {
            (true, _) => DeviceAction::Disconnect,
            (false, true) => DeviceAction::Connect,
            (false, false) => DeviceAction::Pair,
        }
    }
}

// ---------------------------------------------------------------------------
// Reading (asynchronous, for a card)
// ---------------------------------------------------------------------------

/// Read the controller state *and* its devices, and hand both to `on_done` on
/// the main loop.
///
/// Four questions, one after another (`bluetoothctl` answers one at a time, so
/// each answer costs a process), and then one `info` per device that will be
/// listed - which is where the icon, the link quality and the device's battery
/// come from. What makes that affordable is that it only runs while a card is
/// *open* and on its own slow clock: none of it is on the pointer's path, and a
/// closed card asks bluetoothctl nothing at all.
///
/// The questions are asked by this function and the three below it, one each:
/// what is paired, what is connected, what BlueZ knows at all, and finally what
/// the controller itself is doing. The chain is flat - each step starts the next
/// read from its own callback - rather than four closures deep, because every
/// step wants its own comment about why it is asked.
///
/// `limit` is how many devices the card shows - there is no point reading
/// details for rows nobody will see.
pub fn refresh(limit: usize, on_done: impl FnOnce(Bluetooth, Vec<Device>) + 'static) {
    output::read(
        "bluetoothctl",
        &argv(&["devices", "Paired"]),
        move |paired| {
            let paired = parse_devices(&text(paired));
            read_connected(paired, limit, on_done);
        },
    );
}

/// The second question: the devices the controller is talking to right now.
/// Their number is what "On" means in the header, and they are the rows the
/// list puts first.
fn read_connected(
    paired: Vec<(String, String)>,
    limit: usize,
    on_done: impl FnOnce(Bluetooth, Vec<Device>) + 'static,
) {
    output::read(
        "bluetoothctl",
        &argv(&["devices", "Connected"]),
        move |connected| {
            let connected = parse_devices(&text(connected));
            read_known(paired, connected, limit, on_done);
        },
    );
}

/// The third question, and the one that makes pairing possible from a card:
/// every device BlueZ knows, paired or not.
///
/// `devices Paired` can only ever name a device that was paired somewhere
/// before, so without this list a device a scan has just found has no row to be
/// paired from - which is what "the panel cannot pair anything" looks like from
/// the outside.
fn read_known(
    paired: Vec<(String, String)>,
    connected: Vec<(String, String)>,
    limit: usize,
    on_done: impl FnOnce(Bluetooth, Vec<Device>) + 'static,
) {
    output::read("bluetoothctl", &argv(&["devices"]), move |seen| {
        let seen = parse_devices(&text(seen));
        read_controller(paired, connected, seen, limit, on_done);
    });
}

/// The last question - `show` - and then the details, one `info` per row the
/// card will actually draw.
fn read_controller(
    paired: Vec<(String, String)>,
    connected: Vec<(String, String)>,
    seen: Vec<(String, String)>,
    limit: usize,
    on_done: impl FnOnce(Bluetooth, Vec<Device>) + 'static,
) {
    output::read("bluetoothctl", &argv(&["show"]), move |show| {
        let show = text(show);
        let bluetooth = Bluetooth {
            present: controller_present(&show),
            powered: powered(&show),
            discoverable: discoverable(&show),
            connected: connected.len(),
            alias: controller_alias(&show),
        };
        let devices = merge_devices(&paired, &connected, &seen);
        // The rows are drawn from this cheap answer already (the caller repaints
        // as soon as it has it), and the details land as a repaint underneath.
        //
        // `on_done` is an `FnOnce`, and the detail chain shares its callback
        // between as many answers as there are devices - so it is parked in a
        // cell and taken out by the one call that ends the chain. `bluetooth`
        // travels with it: the closure stored in the `Rc` is an `Fn`, so it
        // cannot move either value out of its captures more than once.
        let on_done = std::cell::RefCell::new(Some(on_done));
        let bluetooth = std::cell::RefCell::new(Some(bluetooth));
        detail(
            devices.into_iter().collect(),
            Vec::new(),
            limit,
            Rc::new(move |devices| {
                if let (Some(on_done), Some(bluetooth)) =
                    (on_done.borrow_mut().take(), bluetooth.borrow_mut().take())
                {
                    on_done(bluetooth, devices);
                }
            }),
        );
    });
}

/// Read one device's `info`, then the next one's, and hand the list back once
/// there is nobody left to ask.
///
/// Recursion rather than a loop, because every step is asynchronous: the "loop"
/// *is* the call chain, each link sitting in the previous read's callback. The
/// devices past `limit` are appended untouched - they are only there for the
/// "+N more" line.
fn detail(
    mut queue: VecDeque<Device>,
    mut done: Vec<Device>,
    limit: usize,
    on_done: Rc<dyn Fn(Vec<Device>)>,
) {
    let Some(device) = queue.pop_front() else {
        on_done(done);
        return;
    };
    if limit == 0 {
        done.push(device);
        done.extend(queue);
        on_done(done);
        return;
    }
    let address = device.address.clone();
    output::read("bluetoothctl", &argv(&["info", &address]), move |answer| {
        let mut device = device;
        apply_info(&mut device, &parse_info(&text(answer)));
        done.push(device);
        detail(queue, done, limit - 1, on_done);
    });
}

// ---------------------------------------------------------------------------
// Reading (blocking, for a `status` verb)
// ---------------------------------------------------------------------------

/// The same answer as [`refresh`], asked and *waited for*.
///
/// This is what a `status` verb prints: a read that blocks is exactly what a
/// status read is for, and a card's own reads are the ones that must not.
pub fn now(limit: usize) -> (Bluetooth, Vec<Device>) {
    let show = capture("bluetoothctl", &["show"]).unwrap_or_default();
    let paired =
        parse_devices(&capture("bluetoothctl", &["devices", "Paired"]).unwrap_or_default());
    let connected =
        parse_devices(&capture("bluetoothctl", &["devices", "Connected"]).unwrap_or_default());
    // The superset, and the reason `status` can name a device to pair from a
    // script: the two filtered lists above only ever name devices that were
    // paired before, which is exactly what a device found by a scan is not.
    let seen = parse_devices(&capture("bluetoothctl", &["devices"]).unwrap_or_default());
    let mut devices = merge_devices(&paired, &connected, &seen);
    for device in devices.iter_mut().take(limit) {
        let info =
            parse_info(&capture("bluetoothctl", &["info", &device.address]).unwrap_or_default());
        apply_info(device, &info);
    }
    let connected = devices.iter().filter(|device| device.connected).count();
    (
        Bluetooth {
            present: controller_present(&show),
            powered: powered(&show),
            discoverable: discoverable(&show),
            connected,
            alias: controller_alias(&show),
        },
        devices,
    )
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Whether `show` named a controller at all. A machine with no bluetooth
/// hardware (or no BlueZ daemon) gets an empty answer, not an error.
fn controller_present(show: &str) -> bool {
    show.lines()
        .any(|line| line.trim_start().starts_with("Controller "))
}

/// The controller's friendly name. `Alias:` is what BlueZ shows (it follows a
/// rename), with `Name:` as the fallback for builds that print only that.
fn controller_alias(show: &str) -> String {
    field(show, "Alias:")
        .or_else(|| field(show, "Name:"))
        .unwrap_or_default()
}

/// `Powered: yes` in `show` - and an empty answer (bluetooth off, no daemon) is
/// "off" rather than an error a card would have to render.
fn powered(show: &str) -> bool {
    show.lines().any(|line| line.trim() == "Powered: yes")
}

/// `Discoverable: yes` in `show` - the controller's half of "visible as …".
/// An absent line is "not discoverable".
fn discoverable(show: &str) -> bool {
    show.lines().any(|line| line.trim() == "Discoverable: yes")
}

/// One `devices [filter]` answer: `Device AA:BB:CC:DD:EE:FF Name, with spaces`.
/// The name is whatever follows the address, empty included.
///
/// A device BlueZ has no name for is aliased *as its own address*, spelled with
/// dashes instead of colons (`4A-DD-E6-22-05-B5`) - and a row whose name is a
/// worse spelling of the address it already carries reads as a bug. That alias
/// is dropped so the row falls back to the real address ([`Device::label`]).
fn parse_devices(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("Device ")?;
            let (address, name) = rest.split_once(' ').unwrap_or((rest, ""));
            let (address, name) = (address.trim(), name.trim());
            let name = if name.replace('-', ":") == address {
                ""
            } else {
                name
            };
            Some((address.to_string(), name.to_string()))
        })
        .collect()
}

/// What `info <address>` adds to a row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DeviceInfo {
    icon: DeviceIcon,
    connected: bool,
    paired: bool,
    trusted: bool,
    rssi: Option<i32>,
    battery: Option<i32>,
}

/// The `info` answer, parsed. A property bluetoothctl did not print is `None` or
/// `false`, never a zero that would read as a measurement.
fn parse_info(text: &str) -> DeviceInfo {
    let flag = |name: &str| {
        text.lines()
            .any(|line| line.trim() == format!("{name}: yes"))
    };
    DeviceInfo {
        icon: field(text, "Icon:")
            .map(|icon| DeviceIcon::from(icon.as_str()))
            .unwrap_or_default(),
        connected: flag("Connected"),
        paired: flag("Paired"),
        trusted: flag("Trusted"),
        rssi: field(text, "RSSI:").and_then(|value| value.parse().ok()),
        battery: field(text, "Battery Percentage:").and_then(|value| battery_percent(&value)),
    }
}

/// `0x64 (100)`, which is how bluetoothctl 5.87 prints a device's battery: the
/// hex value, and the same number as a decimal in brackets. Older builds print
/// the hex on its own, hence the fallback.
fn battery_percent(value: &str) -> Option<i32> {
    let value = value.trim();
    if let Some((_, rest)) = value.split_once('(') {
        if let Some((decimal, _)) = rest.split_once(')') {
            if let Ok(percent) = decimal.trim().parse() {
                return Some(percent);
            }
        }
    }
    i32::from_str_radix(value.trim_start_matches("0x"), 16).ok()
}

/// The value of a `Key: value` line, trimmed, or `None` when the key is not
/// there. bluetoothctl prints only the properties it has, so this is a lookup
/// rather than a parse.
fn field(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(key))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The three lists as one: what is connected first, then what is paired, then
/// what a scan has merely *seen*.
///
/// The order is deliberately not "by signal strength": a list that reorders
/// itself under the pointer while a card is up is worse than an unsorted one,
/// and a device's RSSI changes every second.
fn merge_devices(
    paired: &[(String, String)],
    connected: &[(String, String)],
    seen: &[(String, String)],
) -> Vec<Device> {
    // `seen` is the superset - every device BlueZ knows, paired or not - so it
    // is folded in first and the two filtered lists only contribute the flags
    // they answer for. The three overlap freely: one device is routinely in all
    // three, and a device that is neither paired nor connected is a device a
    // scan has found and nobody has claimed yet.
    let mut devices: Vec<Device> = Vec::new();
    for (address, name) in seen.iter().chain(paired).chain(connected) {
        let is_paired = paired.iter().any(|(known, _)| known == address);
        let is_connected = connected.iter().any(|(known, _)| known == address);
        match devices.iter_mut().find(|device| device.address == *address) {
            Some(device) => {
                // The filtered lists answer with the name too, and a device
                // paired before it was renamed can have an empty alias there.
                if device.name.is_empty() && !name.is_empty() {
                    device.name = name.clone();
                }
                device.paired |= is_paired;
                device.connected |= is_connected;
            }
            None => devices.push(Device {
                address: address.clone(),
                name: name.clone(),
                icon: DeviceIcon::Other,
                connected: is_connected,
                // A device in `devices Connected` but not in `devices Paired` is
                // one the controller is talking to anyway; BlueZ does not count
                // that as paired, and the flag is kept honest so the row can say
                // what it is (the row's action is decided by `connected`, which
                // is true here).
                paired: is_paired,
                trusted: false,
                rssi: None,
                battery: None,
            }),
        }
    }
    devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then_with(|| b.paired.cmp(&a.paired))
            .then_with(|| a.label().to_lowercase().cmp(&b.label().to_lowercase()))
    });
    devices
}

/// Fold an `info` answer into a row. The two flags `devices` already answered
/// are *or*'d rather than replaced: the device may have moved between the two
/// reads, and a row that claims a device is off while it is connected would be
/// the one lie the list must not tell.
fn apply_info(device: &mut Device, info: &DeviceInfo) {
    device.icon = info.icon;
    device.trusted = info.trusted;
    device.paired |= info.paired;
    device.connected |= info.connected;
    device.rssi = info.rssi;
    device.battery = info.battery;
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Switch the controller on or off.
///
/// `bluetoothctl` has no toggle subcommand, so the state has to be known first -
/// which the caller has, from its last refresh. Fire and forget: the answer is
/// read back the same way.
pub fn set_powered(powered: bool) {
    launch(
        "bluetoothctl",
        &["power", if powered { "on" } else { "off" }],
    );
}

/// Let the controller answer pairing requests - or stop. The "visible as …"
/// half of the header, switched directly like the power itself.
pub fn set_discoverable(on: bool) {
    launch(
        "bluetoothctl",
        &["discoverable", if on { "on" } else { "off" }],
    );
}

/// Forget a paired device: drop the pairing so it has to be made again.
///
/// Unlike connect and disconnect this is the one bluetooth action that is not
/// cleanly reversible from here - the device has to be paired again - so its
/// button is the secondary one on a row and coloured as a danger on hover.
pub fn remove_device(address: &str) {
    launch("bluetoothctl", &["remove", address]);
}

/// Pick a paired device up, or let it go. Both answer once and leave; the card
/// re-reads its device list a moment later, so the row tells the truth within a
/// second or two.
pub fn connect_device(address: &str) {
    launch("bluetoothctl", &["connect", address]);
}

pub fn disconnect_device(address: &str) {
    launch("bluetoothctl", &["disconnect", address]);
}

/// Run one of the short actions above without waiting for it.
///
/// [`hardware::launch`] rather than a bare `Command::spawn`: it hands the exit
/// to GLib, which reaps the child, where a bare spawn leaves a zombie behind for
/// every action in a daemon that lives as long as the session does.
fn launch(program: &str, args: &[&str]) {
    hardware::launch(program, args);
}

/// How long `bluetoothctl` is given to pair a device (`--timeout`).
///
/// Pairing is a conversation with the device - and, for anything that has to be
/// confirmed on the device itself, with whoever is holding it - so it is the one
/// bluetooth action here measured in tens of seconds. The bound is what keeps a
/// device that never answers from leaving its row spinning for good.
pub const PAIR_TIMEOUT_SECONDS: u64 = 30;

/// Pair, trust and connect a device a scan found - three steps, in that order,
/// and then the question whether it worked.
///
/// Pairing runs with a `NoInputNoOutput` agent, because a card has no keyboard
/// to type a passkey into: a device that insists on one cannot be paired from
/// here, and its row simply stays unpaired. `--timeout` bounds the whole
/// attempt, so a device that never answers cannot leave the row saying "…" for
/// good.
///
/// `on_done` runs on the main loop once the attempt is over, with whether the
/// device ended up paired. It is what takes the row's spinner away, and what the
/// row has to say when the answer was no: a pair that fails in silence cannot be
/// told apart from a button that does nothing.
pub fn pair_device(address: &str, on_done: impl FnOnce(bool) + 'static) {
    let address = address.to_string();
    output::read("bluetoothctl", &argv(&["pairable", "on"]), move |_| {
        let address = address.clone();
        let timeout = PAIR_TIMEOUT_SECONDS.to_string();
        output::read(
            "bluetoothctl",
            &argv(&[
                "--agent",
                "NoInputNoOutput",
                "--timeout",
                &timeout,
                "pair",
                &address,
            ]),
            move |_| {
                let address = address.clone();
                // Trusted before it is connected: pairing alone does not make a
                // device welcome back. An untrusted one has to be allowed to
                // connect every time it comes near, and there is nobody here to
                // answer that prompt.
                output::read("bluetoothctl", &argv(&["trust", &address]), move |_| {
                    let address = address.clone();
                    // Read to the end rather than spawned: connecting is what
                    // makes the row say "connected", and waiting for it is what
                    // lets `on_done` describe the state the attempt finished in.
                    output::read("bluetoothctl", &argv(&["connect", &address]), move |_| {
                        let address = address.clone();
                        // `pair` exits 0 whether or not it worked, so the
                        // outcome has to be asked for. This is the one answer in
                        // the chain that BlueZ does not hedge.
                        output::read("bluetoothctl", &argv(&["info", &address]), move |info| {
                            on_done(parse_info(&text(info)).paired);
                        });
                    });
                });
            },
        );
    });
}

/// A `bluetoothctl scan` that is running right now.
///
/// It is a flag rather than a child handle, deliberately: `--timeout` makes the
/// scan exit by itself, so there is nothing to kill and nothing that could leak.
/// What a card needs to know is only *that* a scan is in progress - so the
/// button can say so, and the list can be re-read when it ends.
#[derive(Default)]
pub struct Scanner {
    active: Cell<bool>,
}

impl Scanner {
    pub fn active(&self) -> bool {
        self.active.get()
    }

    /// Scan for `seconds`, then run `on_finished` on the main loop. A second
    /// request while one is running is ignored: the button is disabled by then,
    /// and two scans would only be two processes doing one job.
    pub fn start(me: &Rc<Scanner>, seconds: u64, on_finished: impl Fn() + 'static) {
        if me.active.replace(true) {
            return;
        }
        let seconds = seconds.max(1).to_string();
        let spawned = Command::new("bluetoothctl")
            .args(["--timeout", seconds.as_str(), "scan", "on"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(child) => {
                let me = me.clone();
                // The exit *is* the end of the scan: nothing else knows when the
                // controller stopped looking, and the button has to go back to
                // saying "Scan". (The child is reaped by this watch, not by us.)
                glib::child_watch_add_local(glib::Pid(child.id() as i32), move |_, _| {
                    me.active.set(false);
                    on_finished();
                });
            }
            Err(error) => {
                me.active.set(false);
                eprintln!("hypr-osd: cannot scan for bluetooth devices: {error}");
            }
        }
    }
}

/// `["a", "b"]` as the owned argument list [`output::read`] wants.
fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}

/// Run a command that answers once, and hand back its trimmed stdout when it
/// succeeded. Only [`now`] uses this: everything a card does is asynchronous.
fn capture(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn text(bytes: Option<Vec<u8>>) -> String {
    bytes
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controller() -> Bluetooth {
        Bluetooth {
            present: true,
            powered: true,
            discoverable: false,
            connected: 0,
            alias: "mercury".to_string(),
        }
    }

    #[test]
    fn the_controller_state_reads_as_a_sentence() {
        let off = Bluetooth {
            powered: false,
            ..controller()
        };
        assert_eq!(off.state(), "Off");
        assert_eq!(off.action(), "Turn on");
        assert_eq!(
            Bluetooth {
                connected: 2,
                ..controller()
            }
            .state(),
            "On · 2 connected"
        );
        assert_eq!(controller().state(), "On", "a zero would read as a reading");
        assert_eq!(off.visibility(), "Hidden");
        assert_eq!(
            Bluetooth {
                discoverable: true,
                ..controller()
            }
            .visibility(),
            "Visible"
        );
        assert_eq!(
            Bluetooth {
                present: false,
                ..controller()
            }
            .state(),
            "No controller"
        );
    }

    #[test]
    fn a_device_list_reads_as_addresses_and_names() {
        let text = "Device AA:BB:CC:DD:EE:FF WH-1000XM4\n\
                    Device 11:22:33:44:55:66 K380 Multi-Device\n\n";
        let devices = parse_devices(text);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].0, "AA:BB:CC:DD:EE:FF");
        // A name with spaces is the whole rest of the line, not one word.
        assert_eq!(devices[1].1, "K380 Multi-Device");
    }

    #[test]
    fn a_device_bluez_has_no_name_for_shows_its_address() {
        // Two spellings of "no name": the line can stop at the address, or BlueZ
        // can alias the device as that same address written with dashes. Neither
        // should put the address on the row twice.
        for text in [
            "Device AA:BB:CC:DD:EE:FF\n",
            "Device AA:BB:CC:DD:EE:FF AA-BB-CC-DD-EE-FF\n",
        ] {
            let merged = merge_devices(&parse_devices(text), &[], &[]);
            assert_eq!(merged[0].name, "", "{text}");
            assert_eq!(merged[0].label(), "AA:BB:CC:DD:EE:FF");
        }
    }

    #[test]
    fn a_connected_device_is_listed_before_a_paired_one() {
        // The real shape: the three lists answer separately, they overlap, and a
        // device a scan has found is in `devices` alone.
        let seen = parse_devices("Device AA:BB Buds\nDevice 11:22 Keyboard\nDevice CC:DD Beacon\n");
        let paired = parse_devices("Device AA:BB Buds\nDevice 11:22 Keyboard\n");
        let connected = parse_devices("Device 11:22 Keyboard\n");
        let merged = merge_devices(&paired, &connected, &seen);
        assert_eq!(merged.len(), 3, "one device in three lists is one row");
        assert_eq!(merged[0].label(), "Keyboard");
        assert!(merged[0].connected && merged[0].paired);
        assert_eq!(merged[1].label(), "Buds");
        assert!(!merged[1].connected && merged[1].paired);
        assert_eq!(merged[2].label(), "Beacon");
        assert!(!merged[2].connected && !merged[2].paired);
    }

    #[test]
    fn a_device_only_a_scan_has_seen_is_a_row_that_offers_to_pair_it() {
        // What the Scan button produces: the device is in `devices` - the list of
        // everything BlueZ knows - and in neither of the filtered ones, which is
        // exactly the row that has to be pairable.
        let seen = parse_devices("Device AA:BB Beacon\n");
        let merged = merge_devices(&[], &[], &seen);
        assert_eq!(merged[0].detail(), "nearby");
        assert_eq!(merged[0].action(), DeviceAction::Pair);
    }

    #[test]
    fn the_row_button_follows_what_the_device_is_doing() {
        let paired = parse_devices("Device AA:BB Buds\n");
        let connected = parse_devices("Device 11:22 Keyboard\n");
        let seen = parse_devices("Device 11:22 Keyboard\nDevice AA:BB Buds\nDevice CC:DD Beacon\n");
        let merged = merge_devices(&paired, &connected, &seen);
        assert_eq!(merged[0].action(), DeviceAction::Disconnect);
        assert_eq!(merged[1].action(), DeviceAction::Connect);
        assert_eq!(merged[2].action(), DeviceAction::Pair);
    }

    #[test]
    fn a_connected_device_that_is_not_paired_can_still_be_let_go() {
        let merged = merge_devices(&[], &parse_devices("Device AA:BB Headset\n"), &[]);
        assert!(merged[0].connected);
        assert!(!merged[0].paired);
        // Connected wins over "not paired": whatever else is true of the device,
        // the one thing a row can do about a live link is end it.
        assert_eq!(merged[0].action(), DeviceAction::Disconnect);
    }

    #[test]
    fn device_info_reads_the_icon_the_link_and_the_battery() {
        let info = parse_info(
            "Device AA:BB:CC:DD:EE:FF (public)\n\
             \tName: WH-1000XM4\n\
             \tAlias: WH-1000XM4\n\
             \tClass: 0x00240418 (2360344)\n\
             \tIcon: audio-headphones\n\
             \tPaired: yes\n\
             \tBonded: yes\n\
             \tTrusted: yes\n\
             \tBlocked: no\n\
             \tConnected: yes\n\
             \tRSSI: -56\n\
             \tBattery Percentage: 0x64 (100)\n",
        );
        assert_eq!(info.icon, DeviceIcon::Headphones);
        assert!(info.connected && info.paired && info.trusted);
        assert_eq!(info.rssi, Some(-56));
        assert_eq!(info.battery, Some(100));
    }

    #[test]
    fn info_without_the_optional_numbers_says_nothing_rather_than_zero() {
        let info = parse_info("Device AA:BB:CC:DD:EE:FF (public)\n\tIcon: input-keyboard\n");
        assert_eq!(info.icon, DeviceIcon::Keyboard);
        assert_eq!(info.rssi, None);
        assert_eq!(info.battery, None);
        assert!(!info.connected, "a property that is not there is not `yes`");
    }

    #[test]
    fn a_hex_only_battery_is_still_a_percentage() {
        assert_eq!(battery_percent("0x64 (100)"), Some(100));
        assert_eq!(battery_percent("0x50"), Some(80));
        assert_eq!(battery_percent(""), None);
    }

    #[test]
    fn an_icon_name_is_read_by_family() {
        assert_eq!(DeviceIcon::from("audio-headset"), DeviceIcon::Headphones);
        assert_eq!(DeviceIcon::from("audio-speakers"), DeviceIcon::Speaker);
        assert_eq!(DeviceIcon::from("input-gaming"), DeviceIcon::Gamepad);
        assert_eq!(DeviceIcon::from("input-mouse"), DeviceIcon::Mouse);
        assert_eq!(DeviceIcon::from("phone"), DeviceIcon::Phone);
        assert_eq!(DeviceIcon::from("video-display"), DeviceIcon::Display);
        // A class nobody has seen before still draws something.
        assert_eq!(DeviceIcon::from("something-new"), DeviceIcon::Other);
        assert_eq!(DeviceIcon::Other.icon(), names::BLUETOOTH);
        // Two kinds that used to share one drawing still have one each.
        assert_ne!(DeviceIcon::Computer.icon(), DeviceIcon::Display.icon());
    }

    #[test]
    fn the_detail_line_says_what_the_device_is_doing() {
        let paired = parse_devices("Device AA:BB Buds\n");
        let mut merged = merge_devices(&paired, &[], &[]);
        assert_eq!(merged[0].detail(), "paired");
        apply_info(
            &mut merged[0],
            &parse_info("\tIcon: audio-headset\n\tConnected: yes\n\tRSSI: -56\n\tBattery Percentage: 0x52 (82)\n"),
        );
        // The link quality and the battery draw their own widgets next to this
        // line - it carries the state, not the numbers.
        assert_eq!(merged[0].detail(), "connected");
    }

    #[test]
    fn the_link_quality_reads_as_bars() {
        let paired = parse_devices("Device AA:BB Buds\n");
        let mut device = merge_devices(&paired, &[], &[]).remove(0);
        let mut with_rssi = |rssi| {
            device.rssi = Some(rssi);
            device.signal()
        };
        assert_eq!(with_rssi(-40), Some(4));
        assert_eq!(with_rssi(-55), Some(3));
        assert_eq!(with_rssi(-65), Some(2));
        assert_eq!(with_rssi(-75), Some(1));
        assert_eq!(with_rssi(-90), Some(0));
        device.rssi = None;
        assert_eq!(device.signal(), None, "no RSSI is no meter, not a dead one");
    }

    #[test]
    fn the_controller_says_whether_it_is_discoverable() {
        let show = "Controller AA:BB:CC:DD:EE:FF [default]\n\
                    \tName: mercury\n\
                    \tAlias: mercury\n\
                    \tPowered: yes\n\
                    \tDiscoverable: yes\n";
        assert!(controller_present(show));
        assert!(powered(show));
        assert!(discoverable(show));
        assert_eq!(controller_alias(show), "mercury");
        let quiet = show.replace("Discoverable: yes", "Discoverable: no");
        assert!(powered(&quiet));
        assert!(!discoverable(&quiet));
        // A controller that did not answer at all is neither.
        assert!(!powered("") && !discoverable(""));
        assert!(!controller_present(""));
        assert_eq!(controller_alias(""), "");
    }

    #[test]
    fn the_battery_drawing_follows_the_reading() {
        let mut device = merge_devices(&parse_devices("Device AA:BB Buds\n"), &[], &[]).remove(0);
        for (level, expected) in [
            (5, names::BATTERY_EMPTY),
            (20, names::BATTERY_LOW),
            (50, names::BATTERY_MEDIUM),
            (95, names::BATTERY_FULL),
        ] {
            device.battery = Some(level);
            assert_eq!(device.battery_icon(), expected);
        }
        device.battery = None;
        assert_eq!(device.battery_icon(), names::BATTERY_FULL);
    }

    #[test]
    fn the_subtitle_carries_the_address_a_verb_takes() {
        let device = merge_devices(&parse_devices("Device AA:BB Buds\n"), &[], &[]).remove(0);
        assert_eq!(device.subtitle(), "Paired · AA:BB");
    }
}

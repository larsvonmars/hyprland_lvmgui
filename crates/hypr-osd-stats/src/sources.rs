//! The readings the system popup is drawn from.
//!
//! The popup is a *report on the machine*, so almost everything it shows comes
//! from somewhere that already publishes it: `/proc` and `/sys` for the load and
//! the backlight, `bluetoothctl` and `powerprofilesctl` for the radios and the
//! power profile, `rfkill` for whether the wireless radio is on at all, and
//! Hyprland itself for the keyboard layout. None of that is invented here - this
//! module is the parsing, split from the reading, so what the rows say can be
//! tested without the hardware.
//!
//! Two of these overlap with what the bar already reads, and they are shared
//! rather than copied: the load and the update count are
//! [`hypr_osd_core::system`], and the wireless link is
//! [`hypr_osd_core::hardware`]. One desktop, one opinion about what "hot" means.
//! What is here is what *only* the popup needs.
//!
//! Everything slow is asynchronous. The one genuinely slow read is
//! `bluetoothctl`, which has to talk to the bluetooth daemon over D-Bus and can
//! take a few hundred milliseconds; a popup that freezes its own pointer
//! sampling for that long would close late and feel broken. It goes through
//! [`output::read`], which hands the answer back on the main loop.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;

use gtk::glib;

use hypr_osd_core::hardware;
use hypr_osd_core::hypripc;
use hypr_osd_core::output;

// ---------------------------------------------------------------------------
// Bluetooth (through bluetoothctl)
// ---------------------------------------------------------------------------

/// What the controller is doing, as far as the panel's header and rows go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bluetooth {
    pub powered: bool,
    /// Whether the controller answers to pairing requests right now - the
    /// "visible as …" half of the header.
    pub discoverable: bool,
    /// How many devices are connected right now - the number that makes "on"
    /// mean something.
    pub connected: usize,
}

impl Bluetooth {
    /// The state as the BLUETOOTH row reads it.
    pub fn state(&self) -> String {
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
    /// read: a row always gets a glyph, because a row without one reads broken.
    #[default]
    Other,
}

impl DeviceIcon {
    /// The glyph the row shows, from the same Nerd Font the rest of the
    /// collection draws from.
    pub fn glyph(self) -> &'static str {
        match self {
            DeviceIcon::Headphones => "\u{f025}",
            DeviceIcon::Speaker => "\u{f028}",
            DeviceIcon::Keyboard => "\u{f11c}",
            DeviceIcon::Mouse => "\u{f8cc}",
            DeviceIcon::Phone => "\u{f10b}",
            DeviceIcon::Computer => "\u{f108}",
            DeviceIcon::Gamepad => "\u{f11b}",
            DeviceIcon::Watch => "\u{f017}",
            DeviceIcon::Camera => "\u{f030}",
            DeviceIcon::Printer => "\u{f02f}",
            DeviceIcon::Display => "\u{f26c}",
            DeviceIcon::Network => "\u{f1eb}",
            DeviceIcon::Other => "\u{f293}",
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
    pub fn glyph(self) -> &'static str {
        match self {
            DeviceAction::Pair => "\u{f067}",       // plus
            DeviceAction::Connect => "\u{f0c1}",    // link
            DeviceAction::Disconnect => "\u{f127}", // broken link
            DeviceAction::Remove => "\u{f00d}",     // cross
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

/// One device, as the panel lists it: what it is, what it is doing, and which
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

/// Read the controller state *and* its devices, and hand both to `on_done` on
/// the main loop.
///
/// Four questions, one after another (`bluetoothctl` answers one at a time, so
/// each answer costs a process), and then one `info` per device that will be
/// listed - which is where the icon, the link quality and the device's battery
/// come from. What makes that affordable is that it only runs while the panel is
/// *open* and on its own slow clock: none of it is on the pointer's path, and a
/// popup that is closed asks bluetoothctl nothing at all.
///
/// The questions are asked by this function and the three below it, one each:
/// what is paired, what is connected, what BlueZ knows at all, and finally what
/// the controller itself is doing. The chain is flat - each step starts the next
/// read from its own callback - rather than four closures deep, because every
/// step wants its own comment about why it is asked.
///
/// `limit` is how many devices the panel shows - there is no point reading
/// details for rows nobody will see.
pub fn refresh_bluetooth(limit: usize, on_done: impl FnOnce(Bluetooth, Vec<Device>) + 'static) {
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

/// The third question, and the one that makes pairing possible from here: every
/// device BlueZ knows, paired or not.
///
/// `devices Paired` can only ever name a device that was paired somewhere
/// before, so without this list a device the Scan button has just found has no
/// row to be paired from - which is what "the panel cannot pair anything" looks
/// like from the outside.
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
/// panel will actually draw.
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
            powered: powered(&show),
            discoverable: discoverable(&show),
            connected: connected.len(),
        };
        let devices = merge_devices(&paired, &connected, &seen);
        // The rows are drawn from this cheap answer already (the caller repaints
        // as soon as it has it), and the details land as a repaint underneath.
        //
        // `on_done` is an `FnOnce`, and the detail chain shares its callback
        // between as many answers as there are devices - so it is parked in a
        // cell and taken out by the one call that ends the chain.
        let on_done = RefCell::new(Some(on_done));
        detail(
            devices.into_iter().collect(),
            Vec::new(),
            limit,
            Rc::new(move |devices| {
                if let Some(on_done) = on_done.borrow_mut().take() {
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

/// The same answer as [`refresh_bluetooth`], asked and *waited for*.
///
/// This is what the `status` verb prints: a read that blocks is exactly what a
/// status read is for, and the panel's own reads are the ones that must not.
pub fn bluetooth_now(limit: usize) -> (Bluetooth, Vec<Device>) {
    let show = capture("bluetoothctl", &["show"]).unwrap_or_default();
    let powered = powered(&show);
    let discoverable = discoverable(&show);
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
            powered,
            discoverable,
            connected,
        },
        devices,
    )
}

/// `Powered: yes` in `show` - and an empty answer (bluetooth off, no daemon) is
/// "off" rather than an error the panel would have to render.
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
/// itself under the pointer while the panel is up is worse than an unsorted one,
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

/// Switch the controller on or off.
///
/// `bluetoothctl` has no toggle subcommand, so the state has to be known first -
/// which the panel has, from the last refresh. Fire and forget: the answer is
/// read back the same way.
pub fn set_bluetooth(powered: bool) {
    spawn(
        "bluetoothctl",
        &["power", if powered { "on" } else { "off" }],
    );
}

/// Let the controller answer pairing requests - or stop. The "visible as …"
/// half of the header, switched directly like the power itself.
pub fn set_discoverable(on: bool) {
    spawn(
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
    spawn("bluetoothctl", &["remove", address]);
}

/// Pick a paired device up, or let it go. Both answer once and leave; the panel
/// re-reads its device list a moment later (`refresh_soon`), so the row tells the
/// truth within a second or two.
pub fn connect_device(address: &str) {
    spawn("bluetoothctl", &["connect", address]);
}

pub fn disconnect_device(address: &str) {
    spawn("bluetoothctl", &["disconnect", address]);
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
/// Pairing runs with a `NoInputNoOutput` agent, because the panel has no keyboard
/// to type a passkey into: a device that insists on one cannot be paired from
/// here, and its row simply stays unpaired. `--timeout` bounds the whole attempt,
/// so a device that never answers cannot leave the row saying "…" for good.
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
/// What the panel needs to know is only *that* a scan is in progress - so the
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
                eprintln!("hypr-osd-stats: cannot scan: {error}");
            }
        }
    }
}

/// `["a", "b"]` as the owned argument list `output::read` wants.
fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}

// ---------------------------------------------------------------------------
// Power profile (power-profiles-daemon)
// ---------------------------------------------------------------------------

/// The profiles, quietest first, which is the order the segmented switch lays
/// them out in. A click sets the one that was clicked - there is nothing here
/// that has to know what "next" would mean, which is what the cycling button
/// this replaces needed.
pub const PROFILES: [&str; 3] = ["power-saver", "balanced", "performance"];

/// The current profile, or `None` when the daemon does not answer (not
/// installed, or the machine has no platform profile).
pub fn power_profile() -> Option<String> {
    capture("powerprofilesctl", &["get"]).filter(|profile| !profile.is_empty())
}

pub fn set_power_profile(profile: &str) {
    spawn("powerprofilesctl", &["set", profile]);
}

// ---------------------------------------------------------------------------
// Presentation mode (a systemd inhibitor)
// ---------------------------------------------------------------------------

/// The "presentation mode" toggle: a `systemd-inhibit` child holding an
/// idle/sleep block for as long as it lives.
///
/// This is the old bar's `idle_inhibitor` module moved into the card, with one
/// deliberate difference: the child is *ours*, so leaving the popup or quitting
/// the element releases the block. The script this replaces started an orphan
/// `sleep infinity` that outlived its window - which meant an inhibitor nobody
/// could see or stop, still blocking sleep an hour later.
#[derive(Default)]
pub struct Inhibitor {
    child: Option<Child>,
}

impl Inhibitor {
    pub fn active(&self) -> bool {
        self.child.is_some()
    }

    pub fn toggle(&mut self) {
        if self.active() {
            self.release();
        } else {
            self.acquire();
        }
    }

    /// Let go of the block, if there is one.
    pub fn release(&mut self) {
        if let Some(mut child) = self.child.take() {
            // Killing `sleep` is what ends the inhibitor; systemd takes the lock
            // away with the process.
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn acquire(&mut self) {
        let spawned = Command::new("systemd-inhibit")
            .args([
                "--what=idle:sleep",
                // The name systemd shows in `systemd-inhibit --list`, so it is
                // obvious which button put the lock there.
                "--who=hypr-osd-stats",
                "--why=Presentation",
                "--mode=block",
                "sleep",
                "infinity",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(child) => self.child = Some(child),
            Err(error) => eprintln!("hypr-osd-stats: cannot inhibit: {error}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Brightness (the kernel's backlight class, through brightnessctl)
// ---------------------------------------------------------------------------

/// Where the kernel keeps the backlights.
const BACKLIGHT_DIR: &str = "/sys/class/backlight";

/// The panel brightness in percent, or `None` on a machine whose backlight
/// cannot be read (a desktop, a VM).
pub fn brightness() -> Option<i32> {
    let dir = backlight_dir()?;
    let current: f64 = read_number(&dir.join("brightness"))?;
    let maximum: f64 = read_number(&dir.join("max_brightness"))?;
    backlight_percent(current, maximum)
}

/// The panel's own directory: `intel_backlight` where there is one (this
/// machine), otherwise whatever the kernel did register - the name is a driver
/// detail, and hard-coding it would silently turn the row off on another laptop.
fn backlight_dir() -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(BACKLIGHT_DIR)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    entries.sort();
    if let Some(preferred) = entries.iter().find(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains("intel_backlight"))
    }) {
        return Some(preferred.clone());
    }
    entries.into_iter().next()
}

/// Current over maximum, rounded to a percentage.
fn backlight_percent(current: f64, maximum: f64) -> Option<i32> {
    if maximum <= 0.0 {
        return None;
    }
    Some((current * 100.0 / maximum).round().clamp(0.0, 100.0) as i32)
}

/// Set the backlight. `-e4 -n2` is the same invocation the bar this replaces
/// used: exponent 4 (the kernel's own curve is not perceptually even), and
/// `-n2` so that a value that reads as the same percentage is not written
/// again.
pub fn set_brightness(percent: i32) {
    spawn(
        "brightnessctl",
        &["-e4", "-n2", "set", &format!("{}%", percent.clamp(1, 100))],
    );
}

// ---------------------------------------------------------------------------
// The wireless radio (through rfkill)
// ---------------------------------------------------------------------------

/// Whether the wireless radio is on at all.
///
/// `rfkill` rather than `nmcli radio wifi`: nmcli *translates its values*, so a
/// bar that greps its output for "enabled" reads "deaktiviert" on a German
/// machine and gets the answer wrong. rfkill's `Soft blocked: yes/no` is
/// kernel-defined and never localised.
pub fn radio_on() -> bool {
    parse_radio(&capture("rfkill", &["list", "wifi"]).unwrap_or_default())
}

fn parse_radio(text: &str) -> bool {
    // Anything unreadable is "on": a machine with no wireless card has nothing
    // to switch, and claiming the radio is off would be a lie with a button
    // attached to it.
    !text.lines().any(|line| {
        line.trim()
            .to_ascii_lowercase()
            .starts_with("soft blocked: yes")
            || line
                .trim()
                .to_ascii_lowercase()
                .starts_with("hard blocked: yes")
    })
}

/// Switch the wireless radio. `nmcli` is the tool that can do it; only its
/// *output* is untrustworthy, never its arguments.
pub fn set_radio(on: bool) {
    spawn("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]);
}

// ---------------------------------------------------------------------------
// Keyboard layout (through Hyprland)
// ---------------------------------------------------------------------------

/// The active layout as a short code, e.g. `DE`.
///
/// `j/devices` reports the first keyboard's `active_keymap`, which is the full
/// XKB name - `German (Switzerland)`, not `ch`. That is what the old
/// `hyprland/language` module read too, and its `de` came from XKB's
/// short description, which hyprctl does not expose at all. So the language word
/// is mapped instead, and [`LAYOUT_CODES`] grows as layouts are added to the
/// compositor's config.
pub fn layout() -> String {
    let Some(devices) = hypripc::json("j/devices") else {
        return "?".to_string();
    };
    let keymap = devices
        .get("keyboards")
        .and_then(|keyboards| keyboards.get(0))
        .and_then(|keyboard| keyboard.get("active_keymap"))
        .and_then(|keymap| keymap.as_str())
        .unwrap_or_default();
    layout_code(keymap)
}

/// The language word of an XKB keymap name, as the two-letter code the bar shows.
fn layout_code(keymap: &str) -> String {
    if keymap.is_empty() {
        return "?".to_string();
    }
    let language = keymap.split('(').next().unwrap_or_default().trim();
    match LAYOUT_CODES.iter().find(|(name, _)| *name == language) {
        Some((_, code)) => (*code).to_string(),
        // An unmapped layout shows its full name rather than a wrong code.
        None => keymap.to_string(),
    }
}

const LAYOUT_CODES: [(&str, &str); 8] = [
    ("English", "EN"),
    ("German", "DE"),
    ("French", "FR"),
    ("Italian", "IT"),
    ("Spanish", "ES"),
    ("Portuguese", "PT"),
    ("Dutch", "NL"),
    ("Swiss", "CH"),
];

// ---------------------------------------------------------------------------
// Running things
// ---------------------------------------------------------------------------

/// Start a command and walk away, the way the panel's buttons do: what changes
/// comes back through the next refresh.
///
/// [`hardware::launch`] rather than a bare `Command::spawn`, and not
/// [`hardware::run`] (which waits): half of these open a terminal (`btop`,
/// `nmtui`, `bluetoothctl`, `pacman`) and waiting for one would park the main
/// loop - and with it the pointer sampling that decides whether the panel should
/// still be up - for as long as its window is open. `launch` also hands the exit
/// to GLib, which reaps the child; a bare spawn leaves a zombie behind for every
/// one of these that *does* exit, and this program is a daemon that lives as long
/// as the session does.
pub fn spawn(program: &str, args: &[&str]) {
    hardware::launch(program, args);
}

/// Run a command that answers once, and hand back its trimmed stdout when it
/// succeeded. Used for the short reads (`powerprofilesctl get`, `rfkill`) where
/// waiting a few milliseconds is exactly what a `status` read should do.
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

fn read_number(path: &Path) -> Option<f64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_controller_state_reads_as_a_sentence() {
        let off = Bluetooth {
            powered: false,
            discoverable: false,
            connected: 0,
        };
        assert_eq!(off.state(), "Off");
        assert_eq!(off.action(), "Turn on");
        assert_eq!(
            Bluetooth {
                powered: true,
                discoverable: false,
                connected: 0
            }
            .state(),
            "On",
            "a zero would read as a reading"
        );
        assert_eq!(
            Bluetooth {
                powered: true,
                discoverable: false,
                connected: 2
            }
            .state(),
            "On · 2 connected"
        );
        assert_eq!(off.visibility(), "Hidden");
        assert_eq!(
            Bluetooth {
                powered: true,
                discoverable: true,
                connected: 0
            }
            .visibility(),
            "Visible"
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
        assert_eq!(DeviceIcon::Other.glyph(), "\u{f293}");
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
        assert!(powered(show));
        assert!(discoverable(show));
        let quiet = show.replace("Discoverable: yes", "Discoverable: no");
        assert!(powered(&quiet));
        assert!(!discoverable(&quiet));
        // A controller that did not answer at all is neither.
        assert!(!powered("") && !discoverable(""));
    }

    #[test]
    fn a_backlight_is_a_percentage_of_what_it_can_do() {
        assert_eq!(backlight_percent(96000.0, 96000.0), Some(100));
        assert_eq!(backlight_percent(0.0, 96000.0), Some(0));
        assert_eq!(backlight_percent(24000.0, 96000.0), Some(25));
    }

    #[test]
    fn a_backlight_that_claims_no_maximum_is_no_reading() {
        assert_eq!(backlight_percent(100.0, 0.0), None);
    }

    #[test]
    fn a_blocked_radio_reads_as_off() {
        let blocked = "1: phy0: Wireless LAN\n\tSoft blocked: yes\n\tHard blocked: no\n";
        assert!(!parse_radio(blocked));
        let on = "1: phy0: Wireless LAN\n\tSoft blocked: no\n\tHard blocked: no\n";
        assert!(parse_radio(on));
    }

    #[test]
    fn a_machine_without_a_radio_is_not_reported_as_switched_off() {
        // No rfkill answer at all: nothing to switch, so the button must not
        // claim the radio is off.
        assert!(parse_radio(""));
    }

    #[test]
    fn a_layout_is_the_language_it_is_named_after() {
        assert_eq!(layout_code("German (Switzerland)"), "DE");
        assert_eq!(layout_code("English (US)"), "EN");
        // A bare name, an unmapped name and nothing at all.
        assert_eq!(layout_code("German"), "DE");
        assert_eq!(layout_code("Klingon (Qo'noS)"), "Klingon (Qo'noS)");
        assert_eq!(layout_code(""), "?");
    }
}

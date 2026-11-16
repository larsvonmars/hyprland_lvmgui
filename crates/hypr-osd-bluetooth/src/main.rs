//! `hypr-osd-bluetooth` - element #10: the bluetooth manager.
//!
//! It is the card the system popup's bluetooth tile opens instead of a terminal
//! full of `bluetoothctl`: every device BlueZ knows, paired and merely nearby,
//! with the battery, the link quality and the one action that makes sense for
//! each. The controller's own switches (power and discoverability) and a scan
//! are at the top, and a filter narrows the list when a machine has more devices
//! than rows.
//!
//! Why a card of its own and not a bigger tile: the system popup is a *report on
//! the machine*, and it is sized for readings - a device list longer than five
//! rows would either scroll under the pointer (which would scroll itself away) or
//! push the controls column out of shape. A manager needs room, a filter and a
//! way to sit still while a pairing handshake takes half a minute, so it is a
//! card you open on purpose, like the settings window.
//!
//! Interaction:
//!
//! * **It takes the keyboard while it is up**, because the filter is typed into
//!   and because Escape is the way out. It lets go of the keyboard after
//!   `idle_close_ms` of silence - a session lock takes the keyboard from every
//!   other surface, and a card behind a lock screen that cannot hear Escape would
//!   otherwise sit there for good.
//! * **Up/Down walk the rows and Enter runs the selected row's action** - the
//!   action its state asks for, the same one its button carries. A click on a row
//!   only selects it; connecting is always a button, never a stray click.
//! * **A tap on a row/its buttons is enough**; `.touch` in `bluetooth.css` grows
//!   the rows and the buttons for a fingertip, and the filter field raises the
//!   on-screen keyboard the same way the applications panel's does.
//!
//! Verbs (`hypr-osd-bluetooth <verb>`):
//!
//! ```text
//!   toggle               open the manager, or close it again
//!   show [filter]        open it (with the filter pre-filled)
//!   hide | close         take it away
//!   scan                 look for devices nearby (and open the card)
//!   refresh              read the controller and the devices again
//!   power on|off         the radio itself
//!   discoverable on|off  answer pairing requests, or stop
//!   connect <address>    pick a paired device up
//!   disconnect <address> let a connected device go
//!   pair <address>       pair, trust and connect a scanned device
//!   remove <address>     forget a paired device
//!   status               the controller, the devices and the card's state
//!   (no verb)            start the daemon and wait for the popup's chip
//! ```
//!
//! The addresses are the ones `status` prints. The card's own buttons do not go
//! through the verbs - they *are* this process - but the verbs exist so a script
//! can do everything a finger can.

mod view;

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::bluetooth::{self, Bluetooth, Device, DeviceAction, Scanner};
use hypr_osd_core::timer::Timer;
use hypr_osd_core::{css, run, touch, Config, Content, Keyboard, Opts, Osd, Placement};

use view::BluetoothView;

/// D-Bus application id - and therefore the single-instance key.
const APP_ID: &str = "com.schells2.osd.bluetooth";
/// Layer-shell namespace, the collection's shared prefix.
const NAMESPACE: &str = "hypr-osd";
/// Element name: `~/.config/hypr-osd/bluetooth.conf`.
const ELEMENT: &str = "bluetooth";

// Defaults; every one is overridable in the config file.
/// The card's width. Wide enough for a name, an address, a signal meter, a
/// battery and two buttons on one row - and no wider, because it is a card in
/// the middle of the screen rather than a window.
const DEFAULT_WIDTH: i32 = 640;
/// The height of the scrolling list. The header, the filter and the footer are
/// outside it, so this is the part that changes with the number of devices.
const DEFAULT_LIST_HEIGHT: i32 = 380;
/// How many devices the card reads details for (one `bluetoothctl info` each).
/// A machine with more than this is a museum, and the rows past the limit are
/// still listed with the cheap answer.
const DEFAULT_MAX_DEVICES: usize = 24;
/// How often the controller and the devices are re-read while the card is open.
///
/// It is on a slow clock on purpose: the list costs one `bluetoothctl` per
/// device, so it is re-read when someone is looking, every few seconds - and
/// never at all while the card is closed.
const DEFAULT_DEVICES_EVERY_MS: u64 = 5000;
/// How long the Scan button looks for nearby devices. `bluetoothctl --timeout`
/// makes the scan exit by itself, and that exit is what ends it.
const DEFAULT_SCAN_SECONDS: u64 = 12;
/// Close after this long without a key press or pointer movement (`0` never).
const DEFAULT_IDLE_CLOSE_MS: u64 = 120_000;

/// How long a change is given before the card is read again: a power switch, a
/// connect, a forget - `bluetoothctl` talks to the daemon, and the daemon to the
/// device, so the answer is not there yet at the moment of the click.
const SETTLE_MS: u64 = 900;

struct Settings {
    width: i32,
    list_height: i32,
    max_devices: usize,
    devices_every: u64,
    scan_seconds: u64,
    idle_close: Option<Duration>,
}

impl Settings {
    fn load(config: &Config) -> Self {
        let idle = config.millis("idle_close_ms", DEFAULT_IDLE_CLOSE_MS);
        Settings {
            width: config.i32("width", DEFAULT_WIDTH).max(420),
            list_height: config.i32("list_height", DEFAULT_LIST_HEIGHT).max(160),
            max_devices: config.i32("max_devices", DEFAULT_MAX_DEVICES as i32).max(1) as usize,
            devices_every: config
                .millis("devices_every_ms", DEFAULT_DEVICES_EVERY_MS)
                .as_millis() as u64,
            scan_seconds: config
                .i32("scan_seconds", DEFAULT_SCAN_SECONDS as i32)
                .max(1) as u64,
            idle_close: if idle.is_zero() { None } else { Some(idle) },
        }
    }
}

/// The element's state: what is on the card, the scan, the idle watchdog and the
/// one read that may be in flight.
struct Panel {
    osd: Rc<Osd>,
    view: Rc<BluetoothView>,
    settings: Rc<Settings>,
    scanner: Rc<Scanner>,
    bluetooth: RefCell<Bluetooth>,
    devices: RefCell<Vec<Device>>,
    /// Set while a read is in flight. The last read to start is the one whose
    /// answer is painted; a second read before the first answered would only be
    /// a second chain of processes asking the same questions.
    reading: Cell<bool>,
    idle: Rc<Timer>,
}

impl Panel {
    /// Open the card, pinned. There is no hover here: the manager is opened by a
    /// chip in another card, so it is a place you go and leave.
    fn show(self: &Rc<Self>, filter: Option<&str>) {
        self.osd.show();
        self.refresh();
        if let Some(filter) = filter {
            self.view.set_query(filter);
        }
        // On a laptop the filter is ready for typing; on a tablet focusing it
        // would raise the on-screen keyboard over the very list the card was
        // opened to tap, so there it waits to be tapped.
        if !touch::tablet() {
            self.view.focus_entry();
            // Again once GTK has laid the card out: a freshly mapped surface
            // can refuse focus until it has been allocated.
            let view = self.view.clone();
            glib::timeout_add_local_once(Duration::ZERO, move || view.focus_entry());
        }
        self.arm_idle();
    }

    fn hide(self: &Rc<Self>) {
        self.idle.cancel();
        self.osd.hide();
    }

    /// Close the card after a while with nothing happening. Escape and the ✕ are
    /// how it normally ends; neither works after a session lock, which takes the
    /// keyboard from every other surface. Every interaction re-arms it.
    fn arm_idle(self: &Rc<Self>) {
        let Some(duration) = self.settings.idle_close else {
            return;
        };
        let me = self.clone();
        self.idle.arm(duration, move || {
            if me.osd.is_visible() {
                me.hide();
            }
        });
    }

    /// Read the controller and the devices, and paint them.
    fn refresh(self: &Rc<Self>) {
        if self.reading.replace(true) {
            return;
        }
        let me = self.clone();
        bluetooth::refresh(self.settings.max_devices, move |controller, devices| {
            me.reading.set(false);
            *me.bluetooth.borrow_mut() = controller.clone();
            *me.devices.borrow_mut() = devices.clone();
            me.view.render(controller, devices, me.scanner.active());
        });
    }

    /// Read again once a command has had time to take hold.
    fn refresh_soon(self: &Rc<Self>, millis: u64) {
        let me = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(millis), move || {
            me.refresh();
        });
    }

    fn toggle_power(self: &Rc<Self>) {
        let powered = self.bluetooth.borrow().powered;
        bluetooth::set_powered(!powered);
        self.refresh_soon(SETTLE_MS);
        self.arm_idle();
    }

    fn toggle_discoverable(self: &Rc<Self>) {
        let discoverable = self.bluetooth.borrow().discoverable;
        bluetooth::set_discoverable(!discoverable);
        self.refresh_soon(SETTLE_MS);
        self.arm_idle();
    }

    fn scan(self: &Rc<Self>) {
        if self.scanner.active() {
            return;
        }
        self.view.render_scanning(true);
        let me = self.clone();
        Scanner::start(&self.scanner, self.settings.scan_seconds, move || {
            me.view.render_scanning(false);
            me.refresh();
        });
        self.arm_idle();
    }

    /// Run one device's action, and hold its row's button until the answer is in.
    fn run_action(self: &Rc<Self>, address: &str, action: DeviceAction) {
        match action {
            DeviceAction::Connect => {
                self.view.set_pending(address, action);
                bluetooth::connect_device(address);
            }
            DeviceAction::Disconnect => {
                self.view.set_pending(address, action);
                bluetooth::disconnect_device(address);
            }
            DeviceAction::Remove => {
                self.view.set_pending(address, action);
                bluetooth::remove_device(address);
            }
            DeviceAction::Pair => {
                self.view.set_pending(address, action);
                let me = self.clone();
                let waiting = address.to_string();
                // The chain reports when it is over, which is what takes the
                // spinner away - and what the card has to say when the attempt
                // failed, because a pair that goes back to saying "Pair" in
                // silence cannot be told apart from a button that does nothing.
                bluetooth::pair_device(address, move |paired| {
                    me.view.clear_pending(&waiting);
                    if !paired {
                        me.view.note(
                            "Could not pair - the device may want a passkey (bluetoothctl can ask for one)",
                        );
                    }
                    me.refresh();
                });
                self.arm_idle();
                return;
            }
        }
        self.refresh_soon(SETTLE_MS);
        self.arm_idle();
    }

    fn status(self: &Rc<Self>) -> String {
        // A fresh, blocking read: `status` is what a script asks instead of
        // looking at the card, so it must not depend on when the card was last
        // painted.
        let (controller, devices) = bluetooth::now(self.settings.max_devices);
        let state = if self.osd.is_visible() {
            "open"
        } else {
            "closed"
        };
        let mut lines = vec![
            format!("state {state}"),
            format!(
                "controller {} · {} · {}",
                if controller.alias.is_empty() {
                    "bluetooth"
                } else {
                    &controller.alias
                },
                controller.state(),
                controller.visibility().to_lowercase()
            ),
            format!(
                "powered {} discoverable {} present {}",
                controller.powered, controller.discoverable, controller.present
            ),
            format!(
                "devices {} ({} connected, {} paired)",
                devices.len(),
                devices.iter().filter(|device| device.connected).count(),
                devices.iter().filter(|device| device.paired).count()
            ),
        ];
        for device in &devices {
            lines.push(format!(
                "  {} {} · {} · {}{}{}",
                if device.connected {
                    "connected"
                } else if device.paired {
                    "paired"
                } else {
                    "nearby"
                },
                device.label(),
                device.address,
                device.icon.icon(),
                device
                    .battery
                    .map(|battery| format!(" · {battery}%"))
                    .unwrap_or_default(),
                device
                    .rssi
                    .map(|rssi| format!(" · {rssi} dBm"))
                    .unwrap_or_default(),
            ));
        }
        if let Some(selected) = self.view.selected() {
            lines.push(format!("selected {selected}"));
        }
        lines.push(format!("measured {}", self.view.measured()));
        lines.join("\n")
    }
}

fn main() -> glib::ExitCode {
    let config = Config::load(ELEMENT);
    let settings = Rc::new(Settings::load(&config));

    let opts = Opts {
        app_id: APP_ID.to_string(),
        css: css::stylesheet(include_str!("bluetooth.css")),
        namespace: NAMESPACE.to_string(),
        width: settings.width,
        // A place you go to manage something, not a notification that passes by:
        // the middle of the screen, and the keyboard while it is up.
        placement: Placement::Center,
        keyboard: Keyboard::Exclusive,
        ..Opts::default()
    };

    let state_cell: Rc<OnceCell<Rc<Panel>>> = Rc::new(OnceCell::new());

    let build = {
        let state_cell = state_cell.clone();
        let settings = settings.clone();
        Box::new(move |osd: &Rc<Osd>| {
            let view = BluetoothView::new(settings.list_height);
            let scanner = Rc::new(Scanner::default());
            let panel = Rc::new(Panel {
                osd: osd.clone(),
                view: view.clone(),
                settings: settings.clone(),
                scanner,
                bluetooth: RefCell::new(Bluetooth::default()),
                devices: RefCell::new(Vec::new()),
                reading: Cell::new(false),
                idle: Rc::new(Timer::new()),
            });

            // The card's own controls. Everything that changes what is on screen
            // runs through the panel, so there is one path for a click and a
            // verb.
            view.on_close({
                let panel = panel.clone();
                Rc::new(move || panel.hide())
            });
            view.on_power({
                let panel = panel.clone();
                Rc::new(move || panel.toggle_power())
            });
            view.on_discoverable({
                let panel = panel.clone();
                Rc::new(move || panel.toggle_discoverable())
            });
            view.on_scan({
                let panel = panel.clone();
                Rc::new(move || panel.scan())
            });
            view.on_refresh({
                let panel = panel.clone();
                Rc::new(move || {
                    panel.refresh();
                    panel.arm_idle();
                })
            });
            view.on_action({
                let panel = panel.clone();
                Rc::new(move |address, action| panel.run_action(address, action))
            });
            view.on_query({
                let panel = panel.clone();
                Rc::new(move |_query| panel.arm_idle())
            });

            // Moving the pointer over the card is using it, and a typed filter
            // is too: neither is being idle.
            let motion = gtk::EventControllerMotion::new();
            {
                let panel = panel.clone();
                motion.connect_motion(move |_, _, _| panel.arm_idle());
            }
            view.root.add_controller(motion);

            // A focused field is what raises the on-screen keyboard on a tablet
            // (the daemon is started and stopped by the bar, see `core::touch`).
            if touch::usable() {
                touch::osk().attach(&view.entry);
            }

            // The devices, on their own slow clock - and only while the card is
            // open. The list costs one `bluetoothctl` per device, so a closed
            // card asks nothing at all. A background read is not activity and
            // deliberately does not re-arm the idle watchdog.
            {
                let panel = panel.clone();
                let every = Duration::from_millis(settings.devices_every.max(1000));
                glib::timeout_add_local(every, move || {
                    if panel.osd.is_visible() && !panel.reading.get() {
                        panel.refresh();
                    }
                    glib::ControlFlow::Continue
                });
            }

            install_keys(osd, panel.clone());

            // A fresh process has no card up, whatever a previous one left
            // behind: `kill` reaches neither a `hide` nor a state flag, and a
            // card that is not on screen must not be described as open.
            let root = view.root.clone().upcast::<gtk::Widget>();
            state_cell.set(panel).ok();
            Content::Single(root)
        })
    };

    let handle = {
        let state_cell = state_cell.clone();
        Rc::new(
            move |_osd: &Rc<Osd>, args: &[String]| -> Result<String, String> {
                let panel = state_cell
                    .get()
                    .ok_or("the bluetooth manager has not finished starting up")?;
                let Some(verb) = args.first().map(String::as_str) else {
                    // Started by Hyprland's autostart: wait for the popup's chip.
                    return Ok(String::new());
                };
                let address = args.get(1).map(String::as_str);
                match verb {
                    "toggle" => {
                        if panel.osd.is_visible() {
                            panel.hide();
                        } else {
                            panel.show(None);
                        }
                        Ok(String::new())
                    }
                    "show" | "open" => {
                        panel.show(args.get(1).map(String::as_str));
                        Ok(String::new())
                    }
                    "hide" | "close" => {
                        panel.hide();
                        Ok(String::new())
                    }
                    "scan" => {
                        if !panel.osd.is_visible() {
                            panel.show(None);
                        }
                        panel.scan();
                        Ok(String::new())
                    }
                    "refresh" => {
                        panel.refresh();
                        Ok(String::new())
                    }
                    "power" => match args.get(1).map(String::as_str) {
                        Some("on") => {
                            bluetooth::set_powered(true);
                            panel.refresh_soon(SETTLE_MS);
                            Ok(String::new())
                        }
                        Some("off") => {
                            bluetooth::set_powered(false);
                            panel.refresh_soon(SETTLE_MS);
                            Ok(String::new())
                        }
                        _ => Err("usage: power on|off".to_string()),
                    },
                    "discoverable" => match args.get(1).map(String::as_str) {
                        Some("on") => {
                            bluetooth::set_discoverable(true);
                            panel.refresh_soon(SETTLE_MS);
                            Ok(String::new())
                        }
                        Some("off") => {
                            bluetooth::set_discoverable(false);
                            panel.refresh_soon(SETTLE_MS);
                            Ok(String::new())
                        }
                        _ => Err("usage: discoverable on|off".to_string()),
                    },
                    "connect" => match address {
                        Some(address) => {
                            panel.run_action(address, DeviceAction::Connect);
                            Ok(String::new())
                        }
                        None => Err("usage: connect <address>".to_string()),
                    },
                    "disconnect" => match address {
                        Some(address) => {
                            panel.run_action(address, DeviceAction::Disconnect);
                            Ok(String::new())
                        }
                        None => Err("usage: disconnect <address>".to_string()),
                    },
                    "pair" => match address {
                        Some(address) => {
                            panel.run_action(address, DeviceAction::Pair);
                            Ok(String::new())
                        }
                        None => Err("usage: pair <address>".to_string()),
                    },
                    "remove" | "forget" => match address {
                        Some(address) => {
                            panel.run_action(address, DeviceAction::Remove);
                            Ok(String::new())
                        }
                        None => Err("usage: remove <address>".to_string()),
                    },
                    "status" => Ok(panel.status()),
                    other => Err(format!(
                    "unknown command `{other}` (toggle | show [filter] | hide | scan | refresh | \
                     power on|off | discoverable on|off | \
                     connect|disconnect|pair|remove <address> | status)"
                )),
                }
            },
        )
    };

    run(opts, build, handle)
}

/// Wire the keys the card lives on.
///
/// Installed through the shell rather than on the widgets, because the shell
/// rebuilds the surface when the card moves to another monitor - a keyboard card
/// that lost its keys after a monitor change would be a bug nobody could explain.
fn install_keys(osd: &Rc<Osd>, panel: Rc<Panel>) {
    osd.on_keys(
        {
            let panel = panel.clone();
            move |key, state| {
                if !panel.osd.is_visible() {
                    return glib::Propagation::Proceed;
                }
                let control = state.contains(gdk::ModifierType::CONTROL_MASK);
                let key = *key;
                let handled = match key {
                    gdk::Key::Escape => {
                        panel.hide();
                        true
                    }
                    gdk::Key::Up if !control => {
                        panel.view.step(-1);
                        panel.arm_idle();
                        true
                    }
                    gdk::Key::Down if !control => {
                        panel.view.step(1);
                        panel.arm_idle();
                        true
                    }
                    gdk::Key::Page_Up if !control => {
                        panel.view.step(-5);
                        panel.arm_idle();
                        true
                    }
                    gdk::Key::Page_Down if !control => {
                        panel.view.step(5);
                        panel.arm_idle();
                        true
                    }
                    gdk::Key::Home if !control => {
                        panel.view.step(i32::MIN / 2);
                        panel.arm_idle();
                        true
                    }
                    gdk::Key::End if !control => {
                        panel.view.step(i32::MAX / 2);
                        panel.arm_idle();
                        true
                    }
                    gdk::Key::Return | gdk::Key::KP_Enter => {
                        panel.view.activate_selected();
                        panel.arm_idle();
                        true
                    }
                    // Ctrl+F is the one field shortcut worth taking: the filter
                    // is the only control here that accepts text.
                    gdk::Key::f if control => {
                        panel.view.focus_entry();
                        true
                    }
                    _ => false,
                };
                if handled {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        },
        // No release handling: this is a card you leave, not a key you hold.
        |_| {},
    );
}

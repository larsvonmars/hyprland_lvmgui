//! `hypr-osd-stats` - the system popup.
//!
//! The bar's status pill is the handle; this is what comes out of it. Hovering
//! the pill asks this program to open (`hypr-osd-stats open`), and from then on
//! the panel looks after itself: it samples the pointer until either the dwell is
//! up (open) or the pointer has been away long enough (close), because *leaving*
//! means moving onto the panel itself - somewhere the bar cannot see.
//!
//! It is the right-hand twin of the island popup, and a separate program for the
//! same reason: a GTK widget can never paint outside its own window, so an
//! "expansion" of a 40-pixel pill is necessarily a second layer-shell surface. It
//! is also the reason the two popups share [`hypr_osd_core::hover`] instead of
//! each carrying a copy of the same state machine: same problem, one answer.
//!
//! The island is centred on the player it unfolds from; this one hangs off the
//! bar's right end, where the tray and the power button sit beside the status
//! pill. The bar hands the actual pill rectangle over with `open`, and each panel
//! uses exactly that (with a fallback strip for an `open` typed by hand).
//!
//! `open` is the *only* verb the bar runs, for both of the pill's handles - the
//! pointer arriving and a left click - so the panel is pointer-driven in every
//! case, exactly like the island: it appears when it is asked for and goes away
//! when the pointer leaves, and nothing the bar does can leave it up. (`toggle`
//! and `show` pin it, which is for a script or for looking at it by hand; a
//! pinned panel is not watched, so only a verb closes it again.)
//!
//! Verbs (`hypr-osd-stats <verb>`):
//!
//! ```text
//!   open [connector] [x y w h]
//!                        the pointer arrived on the pill; the panel opens if it
//!                        stays. What the bar sends: the connector the bar is on
//!                        (so the panel is measured on, and drawn on, the same
//!                        monitor) and the pill's rectangle inside the bar's card.
//!                        Typed by hand without them, the pointer is watched
//!                        against a strip at the bar's right end instead.
//!   close                take it away now
//!   toggle               close it if it is up, otherwise pin it up (a script's
//!                        verb: a pinned panel is not closed by the pointer)
//!   show                 pin it up (no pointer tracking) - debugging
//!   wifi                 switch the wireless radio on/off, which is what the
//!                        bar's status pill runs on a right click: the switch
//!                        has to *read* `rfkill` before it can flip it (see
//!                        `sources`), and this is the element that owns that
//!                        reading.
//!   wifi <verb>          the wireless controls for a script - `on`, `off`,
//!                        `scan` (a fresh scan, not a cached one),
//!                        `connect <ssid>`, `disconnect`, and `forget <ssid>`
//!                        (deletes the stored profile, password included). The
//!                        card's own rows do not come through here: they *are*
//!                        this element, so they call `sources` directly.
//!   bluetooth <verb>     the bluetooth controls for a script - `on`, `off`,
//!                        `scan`, and `connect|disconnect|pair|remove <address>`
//!                        (the addresses `status` prints). The card's own
//!                        buttons do not come through here: they *are* this
//!                        element, so they call `sources` directly.
//!   status               print every reading, without a card
//!   (no verb)            start the daemon and wait for the bar
//! ```

mod sources;
mod view;

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::hover::{self, parse_open, Action, BarGeometry, Delays, Machine, Rect, Side};
use hypr_osd_core::island::{Island, Origin};
use hypr_osd_core::system;
use hypr_osd_core::{css, hardware, run, state, Config, Content, Opts, Osd, Placement, SHADOW_PAD};

use view::StatsView;

/// D-Bus application id - and therefore the single-instance key: the `open` the
/// bar runs reaches the running panel instead of starting a second one.
const APP_ID: &str = "com.schells2.osd.stats";
/// Layer-shell namespace, the collection's shared prefix.
const NAMESPACE: &str = "hypr-osd";
/// Element name: `~/.config/hypr-osd/stats.conf`.
const ELEMENT: &str = "stats";

// Defaults. The bar's geometry ones mirror the bar's own defaults, because this
// panel has to know where the bar is: it hangs itself below the bar's bottom edge
// and lines its right edge up with the bar's, neither of which the compositor can
// work out on its own (see `hover::BarGeometry`).
const DEFAULT_BAR_HEIGHT: i32 = 40;
const DEFAULT_BAR_MARGIN_TOP: i32 = 0;
const DEFAULT_BAR_MARGIN_X: i32 = 0;
const DEFAULT_GAP: i32 = 6;
/// Only used for an `open` that arrives without a rectangle (a hand-typed one,
/// or a bar too old to send one): the width of the strip at the bar's right end
/// that counts as "over the pill".
const DEFAULT_HOT_WIDTH: i32 = 240;
// The two columns. The left one is the wider, because it carries the bluetooth
// device list, where a name and its readings have to fit side by side; the right
// one is a table of short values on one line each. Both numbers are what the
// content needs - and they are *minimums* as far as the panel is concerned: a
// widget that cannot be narrower than the theme says (a slider's trough, a
// progress bar) wins over them, so they are set to the width the rows really
// take rather than to a number that looks good.
const DEFAULT_LEFT_WIDTH: i32 = 260;
const DEFAULT_RIGHT_WIDTH: i32 = 240;
const DEFAULT_OPEN_DELAY_MS: u64 = 120;
const DEFAULT_CLOSE_DELAY_MS: u64 = 300;
const DEFAULT_POLL_MS: u64 = 50;
/// The fast readings - CPU, memory, temperature, the update count - which are
/// file reads and can keep up with the heartbeat.
const DEFAULT_TICK_MS: u64 = 1000;
/// Everything a *command* has to answer: `powerprofilesctl`, `rfkill`, the
/// backlight, the keyboard layout. Cheaper than the readings by nothing, and
/// nothing here changes faster than a person can click a button.
const DEFAULT_SLOW_EVERY_MS: u64 = 5000;
/// How often the bluetooth devices are re-read while the panel is open.
///
/// It is the slowest clock here on purpose: the list costs one `bluetoothctl`
/// per device (a process each, and the *only* way to learn a device's icon, its
/// link quality or its battery), so it is re-read when someone is looking, every
/// few seconds - and never at all while the panel is closed.
const DEFAULT_DEVICES_EVERY_MS: u64 = 10_000;
/// How long the Scan button looks for nearby devices. `bluetoothctl --timeout`
/// makes the scan exit by itself, and that exit is what ends the scan.
const DEFAULT_SCAN_SECONDS: u64 = 12;
/// How many device rows the list shows before it switches to a count. Five rows
/// is what the card holds next to the controls without becoming a scrolling
/// window - and a popup that scrolls under the pointer would scroll itself away
/// from the pointer.
const DEFAULT_MAX_DEVICES: i32 = 5;
/// How often the networks in reach are re-read while the panel is open.
///
/// It sits between the two clocks above: one `nmcli` for the list, one for the
/// stored profiles and one per profile (see `sources::refresh_wifi`) - a handful
/// of quick processes, not one per row. Fifteen seconds is also about how often
/// NetworkManager's own background scan refreshes the cache the list is read
/// from, so a shorter clock would only re-read the same answer.
const DEFAULT_NETWORKS_EVERY_MS: u64 = 15_000;
/// How many network rows the wireless tile shows before it switches to a count.
/// The same number as the bluetooth list, and for the same reason: the two tiles
/// sit one above the other, and a popup that scrolled would scroll itself away
/// from the pointer.
const DEFAULT_MAX_NETWORKS: i32 = 5;
/// How often a stale update count is refreshed. `checkupdates` syncs a pacman
/// database in a temporary directory, and the count only changes when somebody
/// syncs anyway - so half an hour, which is also how long a count is believed
/// (see `system::UPDATES_MAX_AGE`).
const DEFAULT_UPDATES_EVERY_MS: u64 = 1_800_000;

/// What the config file resolved to.
struct Settings {
    /// How the bar sits, in the pixels this panel is placed in.
    bar: BarGeometry,
    hot_width: f64,
    left_width: i32,
    right_width: i32,
    delays: Delays,
    /// How often to ask the compositor where the pointer is, while it matters.
    poll: Duration,
    tick: Duration,
    slow_every: u64,
    updates_every: u64,
    devices_every: u64,
    /// How long the Scan button scans for, and how many device rows the list
    /// shows. The second one is also what `main` caps its `bluetoothctl info`
    /// reads at, so no details are read for rows that will not be drawn.
    scan_seconds: u64,
    max_devices: usize,
    /// How often the networks in reach are re-read while the panel is open, and
    /// how many of them the tile lists before it switches to a count.
    networks_every: u64,
    max_networks: usize,
    /// The wireless interface the NETWORK row reports on.
    interface: String,
    /// Where the buttons that open something put it, and where `pacman` runs.
    terminal_command: String,
    /// What the header's gear runs: the settings element (see
    /// `hypr-osd-settings`). A program name or an absolute path - the installer
    /// writes the absolute one, because `~/.local/bin` is usually not on
    /// Hyprland's `PATH`.
    settings_command: String,
    /// What the bluetooth tile's "Manage" chip runs: the collection's own
    /// manager card (`hypr-osd-bluetooth`), the replacement for a terminal full
    /// of `bluetoothctl`. Same convention as `settings_command`.
    bluetooth_command: String,
    /// What prints the pending-update list (`checkupdates`). Empty turns the
    /// updates tile off: it then says nothing at all rather than "up to date".
    updates_command: String,
}

impl Settings {
    fn load(config: &Config) -> Self {
        Settings {
            bar: BarGeometry {
                height: config.i32("bar_height", DEFAULT_BAR_HEIGHT).max(1) as f64,
                margin_top: config.i32("bar_margin_top", DEFAULT_BAR_MARGIN_TOP).max(0) as f64,
                margin_x: config.i32("bar_margin_x", DEFAULT_BAR_MARGIN_X).max(0) as f64,
                gap: config.i32("gap", DEFAULT_GAP).max(0) as f64,
                shadow_pad: f64::from(SHADOW_PAD),
            },
            hot_width: config.i32("hot_width", DEFAULT_HOT_WIDTH).max(1) as f64,
            left_width: config.i32("left_width", DEFAULT_LEFT_WIDTH).max(80),
            right_width: config.i32("right_width", DEFAULT_RIGHT_WIDTH).max(80),
            delays: Delays {
                open: config.millis("open_delay_ms", DEFAULT_OPEN_DELAY_MS),
                close: config.millis("close_delay_ms", DEFAULT_CLOSE_DELAY_MS),
            },
            poll: config.millis("poll_ms", DEFAULT_POLL_MS),
            tick: config.millis("tick_ms", DEFAULT_TICK_MS),
            slow_every: config
                .millis("slow_every_ms", DEFAULT_SLOW_EVERY_MS)
                .as_millis() as u64,
            updates_every: config
                .millis("updates_every_ms", DEFAULT_UPDATES_EVERY_MS)
                .as_millis() as u64,
            devices_every: config
                .millis("devices_every_ms", DEFAULT_DEVICES_EVERY_MS)
                .as_millis() as u64,
            scan_seconds: config
                .i32("scan_seconds", DEFAULT_SCAN_SECONDS as i32)
                .max(1) as u64,
            max_devices: config.i32("max_devices", DEFAULT_MAX_DEVICES).max(1) as usize,
            networks_every: config
                .millis("networks_every_ms", DEFAULT_NETWORKS_EVERY_MS)
                .as_millis() as u64,
            max_networks: config.i32("max_networks", DEFAULT_MAX_NETWORKS).max(1) as usize,
            interface: config.string("network_interface", "wlan0"),
            terminal_command: config.string("terminal_command", "kitty"),
            settings_command: config.string("settings_command", "hypr-osd-settings"),
            bluetooth_command: config.string("bluetooth_command", "hypr-osd-bluetooth"),
            updates_command: config.string("updates_command", "checkupdates"),
        }
    }

    /// The panel's *content* width: the two columns, the gap between them, and
    /// the padding this panel's own stylesheet puts around the body (2px each
    /// side - see `box.stats` in `stats.css`).
    ///
    /// Getting this right is what makes the derived hover zone line up with the
    /// panel that is actually on screen: a column whose content is wider than its
    /// request grows the card, and the zone would then be measured against a
    /// rectangle the card no longer is. `status` prints the measured width next
    /// to this one so the two can be compared instead of guessed at.
    fn content_width(&self) -> i32 {
        self.left_width + self.right_width + 10 + 4
    }
}

/// The panel's state: what the hover machine says, what is on the card, and the
/// readings behind it.
struct Panel {
    machine: Machine,
    node: Rc<StatsView>,
    /// The shape the panel unfolds from, and the card it becomes (see
    /// `core::island`). The card's widgets live inside it; the surface is a
    /// transparent canvas the size of the screen.
    island: Rc<Island>,
    /// The connector the bar is on, when the bar said so with `open`. The bar is
    /// pinned to the output it started on, which is not necessarily the focused
    /// one - so every coordinate this panel works with, and the surface it draws
    /// on, come from here rather than from "the focused output".
    monitor: RefCell<Option<String>>,
    /// The pill's rectangle, when the bar said where it is. The bar knows - it
    /// is the one drawing it - and it is the only place that *can* know, because
    /// the tray and the power button sit between this pill and the bar's edge.
    pill: RefCell<Option<Rect>>,
    /// The CPU reading is a delta between two samples, so it needs both.
    sampler: RefCell<system::Sampler>,
    reading: RefCell<Option<system::Reading>>,
    updates: Cell<i32>,
    names: RefCell<Vec<String>>,
    /// The SSID `iw` says the wireless link is on, as last read. It is what
    /// marks a row in the network tile as connected - nmcli's own `IN-USE` field
    /// is only a hint and was measured pointing at the wrong access point (see
    /// `sources`), so the list is drawn against this instead.
    link: RefCell<Option<String>>,
    /// The bluetooth scan the Scan button started, if it is still running. The
    /// scanner knows when it ends, and `main` holds it because the panel's own
    /// state is what the button and the header chip are drawn from.
    scanner: Rc<sources::Scanner>,
}

fn main() -> glib::ExitCode {
    let config = Config::load(ELEMENT);
    let settings = Rc::new(Settings::load(&config));

    let opts = Opts {
        app_id: APP_ID.to_string(),
        css: css::stylesheet(include_str!("stats.css")),
        namespace: NAMESPACE.to_string(),
        // A transparent canvas the size of the screen: the card does not hang
        // under the bar from here, it **unfolds out of the status pill** - the
        // shape's own rectangle is worked out by `core::island`, from the bar
        // geometry and the pill the bar sends with `open` (see `Placement::Canvas`).
        placement: Placement::Canvas,
        ..Opts::default()
    };

    let panel: Rc<OnceCell<Rc<RefCell<Panel>>>> = Rc::new(OnceCell::new());

    let build = {
        let panel = panel.clone();
        let settings = settings.clone();
        Box::new(move |osd: &Rc<Osd>| {
            let node = StatsView::new(&settings);
            // The card is put into the island's body here and stays there: the
            // island is what the surface draws, and every repaint below goes on
            // changing these same widgets.
            let island = Island::new(&node.root);
            node.on_refresh({
                let panel = panel.clone();
                let settings = settings.clone();
                Rc::new(move || {
                    if let Some(panel) = panel.get() {
                        refresh(panel, &settings);
                    }
                })
            });

            let scanner = Rc::new(sources::Scanner::default());
            // The Scan button asks for a scan; the scanner owns it from there.
            // The repaint is immediate (the button says "Scanning" while the
            // controller looks), and the *end* of the scan is what re-reads the
            // list - a device that answered the scan is only listed once
            // bluetoothctl has been asked again.
            node.on_scan({
                let panel = panel.clone();
                let settings = settings.clone();
                let scanner = scanner.clone();
                Rc::new(move || {
                    let Some(state) = panel.get() else {
                        return;
                    };
                    state.borrow().node.render_scanning(true);
                    let panel = panel.clone();
                    let settings = settings.clone();
                    sources::Scanner::start(&scanner, settings.scan_seconds, move || {
                        if let Some(state) = panel.get() {
                            state.borrow().node.render_scanning(false);
                        }
                        if let Some(state) = panel.get() {
                            refresh_devices(state, &settings);
                        }
                    });
                })
            });

            // A fresh process has no panel up, whatever the flag says: an element
            // that was killed while the panel was open would otherwise leave the
            // bar's status pill lit for the rest of the session, because the flag
            // is written on show/hide and a `kill` reaches neither.
            state::write(&state::stats_panel(), false);

            // The network tile's own Scan. It needs no scanner: the read *is* the
            // scan (`nmcli --rescan yes`), it answers on the main loop like every
            // other read here, and the list it hands back is the fresh one.
            node.on_wifi_scan({
                let panel = panel.clone();
                Rc::new(move || {
                    if let Some(state) = panel.get() {
                        refresh_wifi(state, true);
                    }
                })
            });

            let state = Rc::new(RefCell::new(Panel {
                machine: Machine::default(),
                node: node.clone(),
                island: island.clone(),
                monitor: RefCell::new(None),
                pill: RefCell::new(None),
                sampler: RefCell::new(system::Sampler::new()),
                reading: RefCell::new(None),
                updates: Cell::new(0),
                names: RefCell::new(Vec::new()),
                link: RefCell::new(None),
                scanner,
            }));
            let _ = panel.set(state.clone());

            // The first paint, before the surface is ever shown: the card has to
            // be *sized* before the hover machine can be told where it is, and a
            // panel that opened empty and filled in a second later would be a
            // visible lie about the machine.
            refresh(&state, &settings);

            // The update count comes from a shared cache; asking for it may mean
            // running `checkupdates`, which syncs a pacman database - always in
            // the background (see `system::refresh_updates`).
            if !settings.updates_command.is_empty() && system::cached_updates().is_none() {
                ask_for_updates(&state, &settings);
            }

            // One heartbeat for the readings: cheap file reads, so they can keep
            // up with it, and the panel is right from its second tick.
            let ticks = Rc::new(Cell::new(0u64));
            {
                let state = state.clone();
                let settings = settings.clone();
                glib::timeout_add_local(settings.tick, move || {
                    let count = ticks.get() + 1;
                    ticks.set(count);
                    if state.borrow().machine.is_open() {
                        render_readings(&state, &settings);
                        if every(count, settings.slow_every, settings.tick) {
                            render_slow(&state, &settings);
                        }
                        if every(count, settings.devices_every, settings.tick) {
                            // The device list, on its own slower clock: it is
                            // the one read here that costs a process per row.
                            refresh_devices(&state, &settings);
                        }
                        if every(count, settings.networks_every, settings.tick) {
                            // The networks in reach, on the same kind of clock
                            // and for the same reason - a handful of `nmcli`
                            // processes nobody asked for while the panel is shut.
                            refresh_wifi(&state, false);
                        }
                        if every(count, settings.updates_every, settings.tick)
                            && system::cached_updates().is_none()
                        {
                            // The count is believed for half an hour; this is
                            // what asks again once it goes stale.
                            ask_for_updates(&state, &settings);
                        }
                        // The card can change size under the shape (a device
                        // appears, a row wraps): the island is the screen, so it
                        // has to be told, where the surface used to grow with
                        // its content by itself.
                        if let Some(origin) = origin_of(&state.borrow(), &settings) {
                            state.borrow().island.refresh(&origin);
                        }
                    }
                    glib::ControlFlow::Continue
                });
            }

            // The pointer, on its own faster clock - and only while there is
            // something to watch. An idle popup asks the compositor for nothing at
            // all, where the script this replaces polled for the whole session.
            {
                let state = state.clone();
                let settings = settings.clone();
                let osd = osd.clone();
                glib::timeout_add_local(settings.poll, move || {
                    if state.borrow().machine.is_watching() {
                        let action = sample(&state, &settings);
                        act(&state, &osd, &settings, action);
                    }
                    glib::ControlFlow::Continue
                });
            }

            // A block that outlives its popup is a block nobody can see: letting
            // go of the presentation inhibitor is part of going away.
            osd.on_shutdown({
                let state = state.clone();
                move || state.borrow().node.release()
            });

            Content::Single(island.canvas().clone().upcast::<gtk::Widget>())
        })
    };

    let handle = {
        let panel = panel.clone();
        let settings = settings.clone();
        Rc::new(
            move |osd: &Rc<Osd>, args: &[String]| -> Result<String, String> {
                let panel = panel
                    .get()
                    .ok_or("the panel has not finished starting up")?;
                let Some(verb) = args.first().map(String::as_str) else {
                    return Ok(String::new());
                };
                match verb {
                    // What the bar's status pill runs when the pointer arrives.
                    // It only *arms* the panel: the dwell, and the decision to
                    // open, are the panel's own (see `hover`).
                    "open" => {
                        let (connector, rect) = parse_open(&args[1..]);
                        let mut state = panel.borrow_mut();
                        // Where the bar is, so this panel can be there too - the
                        // shell moves the surface if it is not.
                        if let Some(connector) = connector.as_deref() {
                            osd.pin_output(Some(connector));
                        }
                        if connector.is_some() {
                            *state.monitor.borrow_mut() = connector;
                        }
                        *state.pill.borrow_mut() = rect;
                        state.machine.arm(Instant::now());
                        Ok(String::new())
                    }
                    "close" => {
                        let action = panel.borrow_mut().machine.dismiss();
                        act(panel, osd, &settings, action);
                        Ok(String::new())
                    }
                    "toggle" => {
                        let action = {
                            let mut state = panel.borrow_mut();
                            if state.machine.is_open() {
                                state.machine.dismiss()
                            } else {
                                state.machine.pin();
                                Action::Show
                            }
                        };
                        act(panel, osd, &settings, action);
                        Ok(String::new())
                    }
                    "show" => {
                        panel.borrow_mut().machine.pin();
                        act(panel, osd, &settings, Action::Show);
                        Ok(String::new())
                    }
                    // The wireless controls. With no argument this is the radio
                    // switch the bar's status pill runs on a right click: it has
                    // to *read* `rfkill` before it can flip it (see `sources`),
                    // and this is the element that owns that reading. The rest
                    // name a network by its SSID, which `status` prints - they
                    // are a script's way of asking, because the card's own rows
                    // call `sources` directly (they *are* this element).
                    "wifi" => {
                        match args.get(1).map(String::as_str) {
                            None => sources::set_radio(!sources::radio_on()),
                            Some("on") => sources::set_radio(true),
                            Some("off") => sources::set_radio(false),
                            Some("scan") => {
                                // The read *is* the scan (`--rescan yes`), and it
                                // answers on its own when the card has looked.
                                refresh_wifi(panel, true);
                                return Ok(String::new());
                            }
                            Some("connect") => match args.get(2) {
                                Some(ssid) => sources::connect_wifi(ssid),
                                None => return Err(wifi_usage()),
                            },
                            // By device, not by profile: a profile can be renamed
                            // by hand, the interface cannot.
                            Some("disconnect") => sources::disconnect_wifi(&settings.interface),
                            Some("forget") => {
                                match args.get(2).and_then(|ssid| sources::profile_for(ssid)) {
                                    Some(profile) => sources::forget_wifi(&profile),
                                    None => return Err(wifi_usage()),
                                }
                            }
                            Some(_) => return Err(wifi_usage()),
                        }
                        // Whatever it was, it takes a moment to take hold - the
                        // radio comes up, a join gets its lease, a profile is
                        // deleted - so both reads are repeated once it has had
                        // one.
                        let panel = panel.clone();
                        let settings = settings.clone();
                        glib::timeout_add_local_once(Duration::from_millis(900), move || {
                            render_slow(&panel, &settings);
                            refresh_wifi(&panel, false);
                        });
                        Ok(String::new())
                    }
                    // The bluetooth controls, for a script (the card's own
                    // buttons call `sources` directly - they *are* this element).
                    // `bluetooth power off` is the radio; the rest take an
                    // address, which `status` prints.
                    "bluetooth" => {
                        let (verb, address) = (args.get(1), args.get(2));
                        match (verb.map(String::as_str), address.map(String::as_str)) {
                            (Some("on"), _) => sources::set_bluetooth(true),
                            (Some("off"), _) => sources::set_bluetooth(false),
                            (Some("connect"), Some(address)) => sources::connect_device(address),
                            (Some("disconnect"), Some(address)) => {
                                sources::disconnect_device(address)
                            }
                            (Some("pair"), Some(address)) => {
                                // The attempt outlives this answer by design -
                                // the verb is a script's way of *asking*, and
                                // waiting here would park the panel's own pointer
                                // sampling for the whole handshake. A script that
                                // wants to know how it went reads `status`.
                                sources::pair_device(address, |_| {})
                            }
                            (Some("remove"), Some(address)) => sources::remove_device(address),
                            (Some("scan"), _) => {
                                let scanner = panel.borrow().scanner.clone();
                                let seconds = settings.scan_seconds;
                                sources::Scanner::start(&scanner, seconds, || {});
                            }
                            _ => {
                                return Err("usage: bluetooth on|off|scan | \
                                     bluetooth connect|disconnect|pair|remove <address>"
                                    .to_string())
                            }
                        }
                        // Whatever it was, it takes a moment; the rows are
                        // re-read once it has had one.
                        let panel = panel.clone();
                        let settings = settings.clone();
                        glib::timeout_add_local_once(Duration::from_millis(900), move || {
                            refresh_devices(&panel, &settings);
                        });
                        Ok(String::new())
                    }
                    "status" => Ok(status(&panel.borrow(), &settings)),
                    other => Err(format!(
                        "unknown command `{other}` \
                         (open [connector] [connector] [x y w h] | close | toggle | show | \
                         wifi [on|off|scan|connect|disconnect|forget] | \
                         bluetooth on|off|scan | bluetooth connect|disconnect|pair|remove <address> | \
                         status)"
                    )),
                }
            },
        )
    };

    run(opts, build, handle)
}

/// Show or hide the surface, and say so in the state flag while doing it.
///
/// The flag is what the bar's status pill reads to light up. It is written here
/// and nowhere else, so the two programs cannot disagree about whether the panel
/// is up.
///
/// Showing is also when everything that costs a process is read: both radio
/// lists and the readings behind the rows. They are asked for when somebody is
/// about to look at them rather than on the heartbeat, so a panel opened after a
/// minute of being shut is not a minute out of date.
fn act(panel: &Rc<RefCell<Panel>>, osd: &Rc<Osd>, settings: &Rc<Settings>, action: Action) {
    match action {
        Action::None => {}
        Action::Show => {
            // The surface comes up with the card already unfolded: a panel that
            // appeared at its full size for one frame and *then* grew would flash.
            // The shape starts as the pill the pointer came from, so the first
            // frame is the pill the bar is drawing - the two surfaces hand over.
            {
                let state = panel.borrow();
                if let Some(origin) = origin_of(&state, settings) {
                    state.island.unfold(&origin, settings.hot_width);
                }
            }
            osd.show();
            state::write(&state::stats_panel(), true);
            refresh(panel, settings);
            // The reads above can change the card's size (a device row appears
            // before the first frame), and the island has to know.
            if let Some(origin) = origin_of(&panel.borrow(), settings) {
                panel.borrow().island.refresh(&origin);
            }
        }
        Action::Hide => {
            // The flag goes out first so the bar starts retracting its pills
            // while the shape is still shrinking: they are one movement.
            state::write(&state::stats_panel(), false);
            let osd = osd.clone();
            panel.borrow().island.fold(move || osd.hide());
        }
    }
}

/// Read the bluetooth controller and its devices, and repaint the tile when the
/// answer lands.
fn refresh_devices(panel: &Rc<RefCell<Panel>>, settings: &Rc<Settings>) {
    let limit = settings.max_devices;
    sources::refresh_bluetooth(limit, {
        let panel = panel.clone();
        move |bluetooth, devices| {
            let state = panel.borrow();
            let scanning = state.scanner.active();
            state.node.render_bluetooth(bluetooth, devices, scanning);
            state.node.render_chips();
        }
    });
}

/// Ask `checkupdates` for a new count, in the background, and repaint when it
/// answers. Repainting while the panel is up *and* while it is down, because the
/// tile is part of the first paint the next time it opens.
fn ask_for_updates(panel: &Rc<RefCell<Panel>>, settings: &Rc<Settings>) {
    system::refresh_updates(&settings.updates_command, {
        let panel = panel.clone();
        let settings = settings.clone();
        move |_| render_readings(&panel, &settings)
    });
}

/// Read the networks in reach and the profiles behind them, and repaint the tile
/// when the answer lands.
///
/// The link comes from the panel's own last reading of `iw` rather than from the
/// async answer, because a row is "connected" when the *link* is on it - see
/// [`Panel::link`] - and only one read in this program knows that.
fn refresh_wifi(panel: &Rc<RefCell<Panel>>, rescan: bool) {
    let link = panel.borrow().link.borrow().clone();
    sources::refresh_wifi(link, rescan, {
        let panel = panel.clone();
        move |networks| {
            let state = panel.borrow();
            state.node.render_wifi(networks);
        }
    });
}

/// One pointer sample: read the cursor, work out whether it is over the pill or
/// over the panel, and let the machine decide what to do about it.
fn sample(panel: &Rc<RefCell<Panel>>, settings: &Rc<Settings>) -> Action {
    // Scoped, so the immutable borrow is gone before the machine is told below.
    let rects = { zones(&panel.borrow(), settings) };
    let Some((pill, panel_rect)) = rects else {
        return Action::None;
    };
    let Some(cursor) = hover::cursor() else {
        // The compositor did not answer. Leaving the state alone is the right
        // reaction: a panel that closed because a socket hiccupped would be worse
        // than one that stayed up a moment too long.
        return Action::None;
    };
    panel.borrow_mut().machine.sample(
        pill.contains(cursor, 0.0),
        panel_rect.contains(cursor, 8.0),
        Instant::now(),
        settings.delays,
    )
}

/// Where this panel unfolds from, resolved the way every zone here is: against
/// the monitor the *bar* is on, not the focused one.
///
/// The bar is pinned to the output it started on, which is not necessarily the
/// focused one - so every coordinate this panel works with, and the surface it
/// draws on, come from here. The focused output is the fallback for a hand-typed
/// `open`, and for a bar too old to say where it is.
fn origin_of(panel: &Panel, settings: &Settings) -> Option<Origin> {
    let monitor = panel
        .monitor
        .borrow()
        .as_deref()
        .and_then(hover::monitor_of)
        .or_else(hover::focused_monitor)?;
    Some(Origin {
        side: Side::Right,
        bar: settings.bar,
        monitor,
        pill: *panel.pill.borrow(),
    })
}

/// The two rectangles the machine reasons about, in the layout: the pill's hot
/// zone at the right end of the bar, and the island the panel unfolds into.
///
/// `None` when the compositor cannot say where the monitor is - which is also
/// when nothing should open, because neither rectangle means anything without it.
/// Both come from `core::island`, so the zone the pointer is judged against is
/// the shape that is really on screen (the neck between the bar and the card is
/// part of it now).
fn zones(panel: &Panel, settings: &Settings) -> Option<(Rect, Rect)> {
    let origin = origin_of(panel, settings)?;
    Some(panel.island.zones(&origin, settings.hot_width))
}

/// The two zones as one `status` line, or why there are none.
fn describe_zone(panel: &Panel, settings: &Settings) -> String {
    let Some((pill, panel_rect)) = zones(panel, settings) else {
        return "zones      none - the compositor did not answer hyprctl".to_string();
    };
    format!(
        "zones      pill {:.0},{:.0} {:.0}x{:.0}{}  panel {:.0},{:.0} to {:.0},{:.0}",
        pill.x,
        pill.y,
        pill.width,
        pill.height,
        if panel.pill.borrow().is_some() {
            " (from the bar)"
        } else {
            " (fallback strip)"
        },
        panel_rect.x,
        panel_rect.y,
        panel_rect.x + panel_rect.width,
        panel_rect.bottom(),
    )
}

/// What the `wifi` verb accepts - one string, so an unknown sub-verb and a
/// missing network name cannot describe two different sets of verbs.
fn wifi_usage() -> String {
    "usage: wifi [on|off|scan|connect <ssid>|disconnect|forget <ssid>]".to_string()
}

/// The readings that are cheap enough for the heartbeat: the three sensors and
/// the update count (a file read - *asking* for a new count is what is slow, and
/// that only ever happens in the background).
fn render_readings(panel: &Rc<RefCell<Panel>>, settings: &Rc<Settings>) {
    if !settings.updates_command.is_empty() {
        // A cache that is there and young enough is believed; a stale one is
        // left alone here (the heartbeat is what asks again) so the count does
        // not flicker back to zero while the new one is on its way.
        if let Some(lines) = system::cached_lines() {
            let state = panel.borrow_mut();
            state.updates.set(lines.len() as i32);
            *state.names.borrow_mut() = lines
                .iter()
                .map(|line| {
                    line.split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_string()
                })
                .collect();
        }
    }
    let reading = {
        let state = panel.borrow_mut();
        let reading = state.sampler.borrow_mut().read();
        *state.reading.borrow_mut() = Some(reading);
        reading
    };
    let state = panel.borrow();
    state.node.render_resources(&reading);
    state
        .node
        .render_updates(state.updates.get(), &state.names.borrow());
    state.node.render_chips();
}

/// The rows that each cost a command: the wireless link and its radio, the sink,
/// the power profile, the backlight and the keyboard layout. None of them change
/// faster than a person can click a button, so they sit on the slower interval -
/// and the two that are genuinely slow (the bluetooth device list and the
/// networks in reach) are not here at all: they have their own clocks (see
/// [`refresh_devices`] and [`refresh_wifi`]).
fn render_slow(panel: &Rc<RefCell<Panel>>, settings: &Rc<Settings>) {
    let network = hardware::network(&settings.interface);
    let radio = sources::radio_on();
    {
        let state = panel.borrow_mut();
        // The link's SSID, remembered for the network list: `iw` is the one read
        // that knows what is connected (`nmcli`'s own `IN-USE` is not reliable -
        // see `sources`), and the list is drawn against it.
        *state.link.borrow_mut() = network
            .as_ref()
            .ok()
            .and_then(|link| link.as_ref())
            .map(|link| link.ssid.clone());
        state.node.render_network(
            network.clone().ok().flatten().as_ref(),
            network.is_ok(),
            radio,
        );
        state.node.render_sound(hardware::volume());
        let profile = sources::power_profile();
        state.node.render_profile(profile.as_deref());
        state.node.render_brightness(sources::brightness());
        state.node.render_layout(&sources::layout());
        state.node.render_chips();
    }
}

/// Everything, now: the first paint, and what a button's re-read runs.
fn refresh(panel: &Rc<RefCell<Panel>>, settings: &Rc<Settings>) {
    render_readings(panel, settings);
    render_slow(panel, settings);
    refresh_devices(panel, settings);
    refresh_wifi(panel, false);
}

/// Whether this tick is the one for an interval (the bar's `every`, which this
/// panel needs for the same reason: the interval is in milliseconds and the
/// heartbeat may not be).
fn every(count: u64, interval_ms: u64, tick: Duration) -> bool {
    let tick_ms = tick.as_millis().max(1) as u64;
    count.is_multiple_of((interval_ms / tick_ms).max(1))
}

/// What `status` prints: every reading the panel would show, whether or not it
/// is up.
///
/// This is the answer to "why does the row say that", without a card in the way.
fn status(panel: &Panel, settings: &Settings) -> String {
    let reading = panel.reading.borrow();
    let network = hardware::network(&settings.interface);
    // The devices are read *now* rather than taken from the card's last read:
    // this is a verb for answering questions, and "which address do I pass to
    // `bluetooth connect`" is one of them. Waiting a few tenths of a second is
    // what a status read does everywhere else in this program.
    let (bluetooth, devices) = sources::bluetooth_now(settings.max_devices);
    let mut lines = vec![
        match zones(panel, settings) {
            Some((_, island)) => format!(
                "panel      {:.0},{:.0} {:.0}x{:.0} - one shape, from the bar's top edge",
                island.x, island.y, island.width, island.height
            ),
            None => "panel      no shape - the compositor did not answer hyprctl".to_string(),
        },
        // The size the card *actually* came out at, against the width the two
        // columns were given: the two have to agree, because a widget that asks
        // for more room than the columns were given widens the card - and the
        // island is measured from whatever the content really is.
        {
            let measured = panel.node.root.width();
            let planned = settings.content_width();
            format!(
                "size       content {}x{}, planned {planned}x? - {}",
                measured,
                panel.node.root.height(),
                match measured {
                    // A panel that has never been shown has no allocation: the
                    // question does not apply yet, and saying "mismatch" would
                    // be a lie about a card nobody has looked at.
                    0 => "not drawn yet".to_string(),
                    measured if measured == planned =>
                        "the columns came out as planned".to_string(),
                    measured if measured < planned =>
                        format!("{}px narrower than planned", planned - measured),
                    measured => format!("{}px wider than planned", measured - planned),
                }
            )
        },
        // Where the two hover zones ended up, in the layout. There is no way to
        // move a pointer from here, so this is how "the panel does not open" gets
        // diagnosed: the pill's zone has to be the pill.
        describe_zone(panel, settings),
        // What the machine makes of the pointer - and in particular whether the
        // panel is still being watched. A panel that is up and *not* watched is
        // one that no amount of moving the pointer will take away again, which is
        // the answer to "why will it not close" (`show` and `toggle` pin it; the
        // bar never does - see the bar's status pill).
        format!(
            "state      {}",
            match (panel.machine.is_open(), panel.machine.is_watching()) {
                (false, _) => "closed".to_string(),
                (true, true) => "open, watching the pointer".to_string(),
                (true, false) => "open, pinned - only a verb closes this".to_string(),
            }
        ),
        match reading.as_ref() {
            Some(reading) => format!(
                "resources  CPU {:.0}%  memory {:.0}%  {}",
                reading.cpu,
                reading.memory,
                match reading.temperature {
                    Some(temperature) => format!("{temperature:.0}°C"),
                    None => "no temperature sensor".to_string(),
                }
            ),
            None => "resources  no reading yet".to_string(),
        },
        match (settings.updates_command.is_empty(), panel.updates.get()) {
            (true, _) => "updates    turned off in the config".to_string(),
            (_, 0) => "updates    none pending".to_string(),
            (_, 1) => "updates    1 package waiting".to_string(),
            (_, count) => format!("updates    {count} packages waiting"),
        },
        format!(
            "bluetooth  {} · {} (the power button would {})",
            bluetooth.state(),
            bluetooth.visibility().to_lowercase(),
            bluetooth.action().to_lowercase()
        ),
        format!(
            "scan       {}",
            if panel.scanner.active() {
                format!("running ({}s at a time)", settings.scan_seconds)
            } else {
                "not running".to_string()
            }
        ),
        format!(
            "network    {}",
            match (network.is_ok(), network) {
                (false, _) => "no wireless card".to_string(),
                (true, Ok(Some(link))) => {
                    format!("{} at {}% ({} dBm)", link.ssid, link.signal, link.dbm)
                }
                (true, _) => "not connected".to_string(),
            }
        ),
        format!(
            "sound      {}",
            match hardware::volume() {
                Some(volume) => format!(
                    "{}%{}",
                    volume.percent,
                    if volume.muted { " (muted)" } else { "" }
                ),
                None => "no sink".to_string(),
            }
        ),
        format!(
            "power      {}",
            sources::power_profile().unwrap_or_else(|| "no profile daemon".to_string())
        ),
        format!(
            "brightness {}",
            match sources::brightness() {
                Some(percent) => format!("{percent}%"),
                None => "no backlight".to_string(),
            }
        ),
        format!("layout     {}", sources::layout()),
        format!(
            "presentation {} (the toggle holds a systemd idle/sleep inhibitor)",
            if panel.node.presenting() { "on" } else { "off" }
        ),
    ];
    // The devices as the list would draw them, with the address spelled out:
    // this is where the one for a `bluetooth connect` comes from.
    lines.push(format!(
        "devices    {} known, {} shown at a time{}",
        devices.len(),
        settings.max_devices,
        if devices.is_empty() {
            " (none paired, nothing seen by a scan)"
        } else {
            ""
        }
    ));
    for device in &devices {
        // The link quality and the battery draw as widgets in the panel;
        // `status` spells them out so the shorter detail line loses nothing.
        let readings = [
            device.rssi.map(|rssi| format!("{rssi} dBm")),
            device.battery.map(|battery| format!("battery {battery}%")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("  ");
        lines.push(format!(
            "           {}  {}  {}{}",
            device.address,
            device.label(),
            device.detail(),
            if readings.is_empty() {
                String::new()
            } else {
                format!("  {readings}")
            }
        ));
    }

    // The networks in reach, as the tile draws them - and the profile behind
    // each one, which is what `wifi forget` deletes (the command itself takes the
    // SSID, because that is what the row shows).
    let networks = sources::wifi_now(&settings.interface);
    lines.push(format!(
        "wifi       {} in reach, {} shown at a time{}",
        networks.len(),
        settings.max_networks,
        if networks.is_empty() {
            " (none - the radio is off, or nothing answered)"
        } else {
            ""
        }
    ));
    for network in &networks {
        let profile = match network.profile.as_deref() {
            Some(profile) => format!("  profile {profile}"),
            None => String::new(),
        };
        let band = match &network.band {
            Some(band) => format!("  {band}"),
            None => String::new(),
        };
        lines.push(format!(
            "           {}  {}  {}%{profile}{band}",
            network.ssid,
            network.detail(),
            network.signal
        ));
    }
    lines.join("\n")
}

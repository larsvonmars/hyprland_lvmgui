//! `hypr-osd-bar` - the desktop's top bar.
//!
//! This is the element that is not a card. It owns a layer-shell surface across
//! the top of **every** screen - one surface per output, kept in step with the
//! monitor list, so plugging a screen in grows a bar onto it and unplugging one
//! takes that bar away (`output = eDP-1` in the config narrows it back down to a
//! single screen). Each surface is reserved with the compositor, so windows are
//! laid out below it, and it is up for the whole session rather than appearing
//! for a moment. Everything it shows is read from the system directly in Rust:
//! the workspaces and the focused window from Hyprland's own sockets, what is
//! playing from MPRIS through `playerctl`, the tray from the session bus, and the
//! volume, battery and wireless link from `wpctl` and the kernel's sysfs. There
//! are no helper scripts and no shell-outs to configure a module - which is the
//! whole point of the exercise: the palette, the layout and the data all belong
//! to one program, so they cannot disagree with each other.
//!
//! The bars do not each read the system: they are painted from one set of
//! readings ([`Bars`]), because what a bar shows is a property of the session
//! rather than of a screen. Two monitors therefore cannot show two different
//! volumes, and a screen that appears halfway through is painted immediately from
//! values already in hand instead of waiting for the next tick.
//!
//! Verbs (`hypr-osd-bar <verb>`):
//!
//! ```text
//!   show                 make sure the bar is up
//!   hide                 take it away (it comes back on the next rebuild)
//!   toggle               show it, or hide it
//!   refresh              re-read every pill now
//!   status               print what each pill currently reads, no bar
//!   minimise             put the focused window away, into the tray
//!   restore [n|0x…]      bring one back (the last one put away, by default)
//!   minimised            list what is put away, no bar
//!   tablet on|off        the machine was folded into (or out of) a tablet - the
//!                        Type Cover's switch bind runs this; `tablet status`
//!                        prints the answer instead of changing it
//!   (no verb)            start the bar (what the autostart runs)
//! ```
//!
//! # The finger
//!
//! A finger has no hover and no right button, and the bar is the one element
//! that is always up, so it is where most of this desktop's touch handling
//! lives. Three things are different from the pointer.
//!
//! * **The hover handles do not fire.** GDK turns a tap into an emulated
//!   pointer that *enters* the pill it landed on and never leaves it, so a
//!   motion handler left alone would ask a panel to open on every tap and then
//!   have to guess when to close it. The pointer's way in is unchanged (a real
//!   hover runs `open`); the finger's is the new `tap` verb, which asks the
//!   panel to open *pinned* with a tap catcher around its card - the only way
//!   out of a popup on a screen that has neither Escape nor a pointer to move
//!   away (see [`hypr_osd_core::touch`] and `Osd::tap_catcher`).
//! * **A right click has a long press behind it.** The clock's notification
//!   toggle, the media pill's next track, the status pill's wireless switch and
//!   a tray icon's context menu are bound with `touch::on_secondary`, so the
//!   same action is reachable both ways.
//! * **The bar is a number of pixels rather than a CSS length.** A fingertip
//!   needs about 44 logical pixels, so while the machine is a tablet the bar
//!   grows by [`TOUCH_HEIGHT_EXTRA`], its pills keep their 44-pixel target in
//!   `bar.css` and its marks are drawn a few pixels larger. The height is set
//!   when the surface is made, so the fold is handled with
//!   `Osd::set_placement` (see [`Bars::apply_tablet`]) rather than with a
//!   stylesheet.
//!
//! Three of those verbs are the bar's other job. It is where the desktop's one
//! *window* action lives: the keybinding that puts the focused window away
//! (Hyprland has no minimise of its own - see [`minimise`]) and the row of icons
//! beside the application indicators that brings one back. It belongs in the bar
//! for the same reason the tray does: that row is the only place on this desktop
//! where something that is not on screen is drawn.
//!
//! The verbs speak for *all* the bars - they are one element with one set of
//! pills per screen. Like every element here the binary is single-instance: a
//! verb reaches the running bar over D-Bus instead of starting a second one,
//! which for a bar would be two bars fighting over the same exclusive zone.
//!
//! One rule holds everywhere in this file: **nothing that runs a command waits
//! for it**. The pills' actions run other elements (`hypr-osd-stats`,
//! `hypr-osd-volume`, `hypr-osd-session`) and popups stay up until they are
//! dismissed, so a bar that waited would freeze until the user closed a panel -
//! which is exactly what it used to do. See `sources::launch`.

mod hypr;
mod minimise;
mod tray;
mod view;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
// The machine's own state - the audio sink, the battery, the wireless link -
// lives in the shared half, because the system popup reads it too. The alias
// keeps the call sites reading as they always did: these are the *sources* the
// bar's pills are drawn from.
use hypr_osd_core::hardware::{self as sources, Battery, Network, Volume};
use hypr_osd_core::mpris::{self, Track};
use hypr_osd_core::{css, monitors, run, state, touch, Config, Content, Opts, Osd, Placement};

use view::BarView;

/// D-Bus application id - and therefore the single-instance key.
const APP_ID: &str = "com.schells2.osd.bar";
/// Layer-shell namespace. The whole collection shares the `hypr-osd` prefix so
/// one layer rule in `osd.lua` covers every element - including the
/// `ignore_alpha` rule, which is what keeps the transparent margin around the
/// bar from swallowing clicks on the window behind it.
const NAMESPACE: &str = "hypr-osd";
/// Element name: `~/.config/hypr-osd/bar.conf`.
const ELEMENT: &str = "bar";

// Defaults; every one of them is overridable in the config file. They are the
// numbers the bar this replaces used, so the desktop looks the same after the
// switch: a 40px bar, 8px below the top edge, 12px in from the sides.
const DEFAULT_HEIGHT: i32 = 40;
/// How much taller the bar is while the machine is a tablet, in logical pixels.
///
/// The 40px bar is a pointer's bar: a fingertip needs a 44-pixel target, and a
/// pill keeps a few pixels of breathing room above and below (see `bar.css`),
/// which together do not fit in 40. So the bar grows by this much when the Type
/// Cover is folded away - and the island popup mirrors the same number in its own
/// bar geometry (it hangs below the bar and cannot ask it; see `TOUCH_HEIGHT_EXTRA`
/// in `hypr-osd-island`).
pub const TOUCH_HEIGHT_EXTRA: i32 = 12;
/// How much larger a pill's mark is drawn while the machine is a tablet. The
/// pointer's marks are 11-16px; a finger uses the same pill, so the picture on it
/// has to be readable at arm's length too.
pub const TOUCH_ICON_EXTRA: i32 = 3;
const DEFAULT_MARGIN_TOP: i32 = 0;
const DEFAULT_MARGIN_X: i32 = 0;
/// How many numbered workspaces the row keeps room for, so a workspace you have
/// never visited is still clickable.
const DEFAULT_WORKSPACES: i32 = 5;
/// How much room the window title may take before it is ellipsised.
const DEFAULT_TITLE_WIDTH: i32 = 320;
/// How much room the player pill's track may take before it is ellipsised. It is
/// the bar's centre child, so the cap is what keeps a long title from pushing the
/// two ends apart.
const DEFAULT_MEDIA_WIDTH: i32 = 340;
const DEFAULT_BATTERY: &str = "BAT1";
const DEFAULT_ADAPTER: &str = "ADP1";
const DEFAULT_INTERFACE: &str = "wlan0";
const DEFAULT_ICON_SIZE: i32 = 18;
const DEFAULT_CLOCK_FORMAT: &str = "%H:%M";
/// The small line under the time. Empty turns it off, which gives a one-line
/// clock.
const DEFAULT_DATE_FORMAT: &str = "%a, %b %d";
const DEFAULT_TICK_MS: u64 = 1000;
const DEFAULT_VOLUME_EVERY_MS: u64 = 1000;
const DEFAULT_NETWORK_EVERY_MS: u64 = 5000;
const DEFAULT_BATTERY_EVERY_MS: u64 = 30000;
/// How often the bar re-reads Hyprland even with no event, and re-asks MPRIS.
/// The event socket is the real mechanism; this is the safety net for a socket
/// that quietly died, which would otherwise leave the row frozen for the
/// session and look like a bug in the bar.
const DEFAULT_RESYNC_EVERY_MS: u64 = 5000;

/// Which parts of the bar are drawn.
///
/// Every one of them is a switch on the settings window's Bar page (`show_*` in
/// `bar.conf`), and every one of them defaults to *on*: a bar that quietly lost
/// half its pills after an upgrade would read as a broken bar rather than as a
/// setting. A part that is switched off is not built, not read and not drawn -
/// the media follower still runs (it is one process for the session, shared with
/// nothing else), but the pill it would paint is simply never shown.
#[derive(Clone, Copy)]
pub struct Shows {
    /// The applications button at the far left - the panel's trigger.
    pub apps: bool,
    /// The workspace row.
    pub workspaces: bool,
    /// The focused window's title.
    pub title: bool,
    /// The clock at the right end. It is a reading: the player in the middle is
    /// the island popup's handle.
    pub clock: bool,
    /// The media pill (what is playing) - and the handle that unfolds the island
    /// popup. Hiding it takes that popup's trigger with it.
    pub media: bool,
    /// The combined status pill: the link, the sound and the battery - and the
    /// handle that unfolds the system popup.
    pub status: bool,
    /// The tray: the application indicators. Off, the bar does not claim the
    /// StatusNotifier watcher at all, rather than being a tray that is there and
    /// empty - so the indicators that find no watcher go wherever they go when
    /// this desktop has no tray.
    pub tray: bool,
    /// The windows put away, drawn at the left of the tray's row.
    pub minimised: bool,
    /// The session button, between the status pill and the clock.
    pub power: bool,
}

impl Shows {
    fn load(config: &Config) -> Self {
        Shows {
            apps: config.bool("show_apps", true),
            workspaces: config.bool("show_workspaces", true),
            title: config.bool("show_title", true),
            clock: config.bool("show_clock", true),
            media: config.bool("show_media", true),
            status: config.bool("show_status", true),
            tray: config.bool("show_tray", true),
            minimised: config.bool("show_minimised", true),
            power: config.bool("show_power", true),
        }
    }

    /// The parts that are switched off, in the order the bar draws them - what
    /// `status` prints, so "why is that pill missing" has an answer that is not
    /// guesswork.
    fn hidden(&self) -> Vec<&'static str> {
        let parts = [
            ("apps", self.apps),
            ("workspaces", self.workspaces),
            ("title", self.title),
            ("media", self.media),
            ("minimised", self.minimised),
            ("tray", self.tray),
            ("status", self.status),
            ("power", self.power),
            ("clock", self.clock),
        ];
        parts
            .iter()
            .filter(|(_, on)| !on)
            .map(|(name, _)| *name)
            .collect()
    }
}

/// What the config file resolved to.
pub struct Settings {
    height: i32,
    margin_top: i32,
    margin_x: i32,
    /// Whether the bar reserves its strip with the compositor. Off means tiled
    /// windows slide under it.
    exclusive: bool,
    /// The connector to pin the bar to, or `None` for "the focused one at
    /// start-up".
    output: Option<String>,
    /// Which parts of the bar are drawn (see [`Shows`]).
    shows: Shows,
    workspaces: i32,
    title_width: i32,
    media_width: i32,
    battery: String,
    adapter: String,
    interface: String,
    icon_size: i32,
    clock_format: String,
    date_format: String,
    tick: Duration,
    volume_every: u64,
    network_every: u64,
    battery_every: u64,
    resync_every: u64,
    /// What the clock's hover and clicks run. All of them are programs the
    /// desktop already has, and every one of them is replaceable: the bar only
    /// ever *runs* them, so pointing one elsewhere is a config line.
    island_command: String,
    notifications_command: String,
    volume_command: String,
    session_command: String,
    /// The system popup the status pill unfolds. The pill asks it to open - on
    /// hover and on a left click alike - and to switch the wireless radio on a
    /// right click. It is never *pinned* from here: a pinned panel is not watched
    /// by the pointer, so one click would leave it up for good (see the pill's
    /// handlers in `view`).
    stats_command: String,
    /// The applications panel the left-end button opens. Unlike the two popups
    /// this one is asked to *toggle*: it is a panel you deliberately opened and
    /// that keeps the keyboard while it is up, so the button is the way back out
    /// of it (see `hypr-osd-apps`).
    apps_command: String,
}

impl Settings {
    fn load(config: &Config) -> Self {
        Settings {
            height: config.i32("height", DEFAULT_HEIGHT).max(1),
            margin_top: config.i32("margin_top", DEFAULT_MARGIN_TOP).max(0),
            margin_x: config.i32("margin_x", DEFAULT_MARGIN_X).max(0),
            exclusive: config.bool("exclusive", true),
            output: Some(config.string("output", "")).filter(|output| !output.is_empty()),
            shows: Shows::load(config),
            workspaces: config.i32("workspaces", DEFAULT_WORKSPACES).max(0),
            title_width: config.i32("title_width", DEFAULT_TITLE_WIDTH).max(40),
            media_width: config.i32("media_width", DEFAULT_MEDIA_WIDTH).max(40),
            battery: config.string("battery", DEFAULT_BATTERY),
            adapter: config.string("adapter", DEFAULT_ADAPTER),
            interface: config.string("network_interface", DEFAULT_INTERFACE),
            icon_size: config.i32("tray_icon_size", DEFAULT_ICON_SIZE).max(8),
            clock_format: config.string("clock_format", DEFAULT_CLOCK_FORMAT),
            date_format: config.string("date_format", DEFAULT_DATE_FORMAT),
            tick: config.millis("tick_ms", DEFAULT_TICK_MS),
            volume_every: config
                .millis("volume_every_ms", DEFAULT_VOLUME_EVERY_MS)
                .as_millis() as u64,
            network_every: config
                .millis("network_every_ms", DEFAULT_NETWORK_EVERY_MS)
                .as_millis() as u64,
            battery_every: config
                .millis("battery_every_ms", DEFAULT_BATTERY_EVERY_MS)
                .as_millis() as u64,
            resync_every: config
                .millis("resync_every_ms", DEFAULT_RESYNC_EVERY_MS)
                .as_millis() as u64,
            island_command: config.string("island_command", "hypr-osd-island"),
            notifications_command: config.string("notifications_command", "swaync-client"),
            volume_command: config.string("volume_command", "hypr-osd-volume"),
            session_command: config.string("session_command", "hypr-osd-session"),
            stats_command: config.string("stats_command", "hypr-osd-stats"),
            apps_command: config.string("apps_command", "hypr-osd-apps"),
        }
    }

    /// The bar's height for the machine in `tablet` state.
    ///
    /// The surface height is given to layer-shell when the surface is made, so
    /// this is not a CSS length: the fold is answered by asking the shell to
    /// rebuild the bars with this number (see [`Bars::apply_tablet`]).
    fn bar_height(&self, tablet: bool) -> i32 {
        bar_height(self.height, tablet)
    }

    /// The bar's placement for a machine in `tablet` state - the geometry the
    /// bar starts with, and the one a fold replaces it with.
    fn placement(&self, tablet: bool) -> Placement {
        Placement::Bar {
            height: self.bar_height(tablet),
            margin_top: self.margin_top,
            margin_x: self.margin_x,
            exclusive: self.exclusive,
        }
    }

    /// How large a tray icon is drawn. A fingertip needs a bigger picture than a
    /// pointer does - the icon *is* the target - and the put-away windows' icons
    /// are the same gesture, so they use the same number.
    fn tray_icon_size(&self, tablet: bool) -> i32 {
        tray_icon_size(self.icon_size, tablet)
    }
}

/// The bar's height for a base height and a machine state - the arithmetic of
/// [`Settings::bar_height`], split out so it can be tested without a config file.
fn bar_height(base: i32, tablet: bool) -> i32 {
    base + if tablet { TOUCH_HEIGHT_EXTRA } else { 0 }
}

/// The tray icon size for a base size and a machine state (see
/// [`Settings::tray_icon_size`]).
///
/// The extra is twice the pill mark's, because this picture is the whole target
/// rather than a mark inside a pill - and 24 is the floor: a tablet never draws a
/// tray icon smaller than a fingertip can see.
fn tray_icon_size(base: i32, tablet: bool) -> i32 {
    if tablet {
        (base + TOUCH_ICON_EXTRA * 2).max(24)
    } else {
        base
    }
}

fn main() -> glib::ExitCode {
    let config = Config::load(ELEMENT);
    let settings = Rc::new(Settings::load(&config));

    // The machine's state when the bar starts decides the numbers that cannot
    // change through CSS: the height of the bar's strip and the size of the
    // pictures in it. A fold later is answered by `Bars::apply_tablet`.
    let tablet = touch::tablet();

    let opts = Opts {
        app_id: APP_ID.to_string(),
        css: css::stylesheet(include_str!("bar.css")),
        namespace: NAMESPACE.to_string(),
        placement: settings.placement(tablet),
        // A bar is furniture: it stays on the monitor it started on instead of
        // following the pointer to whichever screen was clicked last.
        pin_output: settings.output.clone(),
        // The width is the screen's, so the card width does not apply; the rest
        // of `Opts` is the OSD default, which a bar ignores.
        ..Opts::default()
    };

    // Every monitor's bar, and the readings they all share. Built out here
    // because the verbs reach it too - and because the shell's per-output factory
    // has to be able to hand out views before `build` has even returned: it is
    // `run` that asks for the first one (see `Content::PerOutput`).
    let bars = Rc::new(Bars::new(settings.clone()));

    // One monitor's bar, as the shell asks for it: once per output at start-up,
    // and again whenever a screen appears.
    let factory: Rc<dyn Fn(&str) -> gtk::Widget> = {
        let bars = bars.clone();
        Rc::new(move |connector: &str| bars.adopt(connector).root.clone().upcast::<gtk::Widget>())
    };

    let build = {
        let bars = bars.clone();
        let settings = settings.clone();
        let factory = factory.clone();
        Box::new(move |osd: &Rc<Osd>| {
            // The bars need the shell to answer one question - which of them are
            // still on screen - so a tray whose screen was unplugged can be given
            // to a bar that still exists (see `Bars::keep_tray`).
            bars.attach(osd);

            // The fold. `begin_touch` hands the current answer over once, so a
            // bar that comes up on an already-folded machine applies the tablet
            // layout without waiting for a switch to fire - and one that comes up
            // on a laptop does nothing at all, because `Bars` starts with that
            // answer. The handler runs on a change from *anywhere*: a switch
            // bind, the shared flag another element wrote, the keyboard leaving
            // (see `touch::watch_tablet`).
            {
                let bars = Rc::downgrade(&bars);
                let shell = Rc::downgrade(osd);
                osd.on_tablet_change(move |tablet| {
                    if let (Some(bars), Some(osd)) = (bars.upgrade(), shell.upgrade()) {
                        bars.apply_tablet(&osd, tablet);
                    }
                });
            }

            // 1. The tray: a StatusNotifier host, kept for the session; dropping
            //    it releases the D-Bus names and the icons go with them. It is
            //    *hosted* by `Bars::adopt`, because it is a session-wide object
            //    with one set of icons and a widget can only be in one bar - so
            //    exactly one of the bars carries it.

            // 2. Hyprland: the workspace row, the window title and the windows
            //    that are put away, kept up to date by the event stream; the
            //    heartbeat re-reads them too, as the safety net for a socket that
            //    quietly died.
            let events = hypr::watch({
                let bars = bars.clone();
                // The event's name travels with the call, because the tray's icons
                // are only worth re-reading when the event can have moved a window
                // (see `minimise::CHANGES`).
                move |event| bars.refresh_hypr(Some(event))
            });

            // 3. MPRIS: the media pill. The follower prints the state it finds
            //    as soon as it attaches, so a player that starts before the bar
            //    shows up here too.
            let follower = mpris::follow({
                let bars = bars.clone();
                move |event| {
                    bars.edit(|paint| {
                        paint.media = Some(event.track.clone());
                        paint.playing = event.playing;
                    })
                }
            });

            // 4. The tick. One timer for everything periodic, so the bars have a
            //    single heartbeat to reason about; each source decides how often
            //    it is worth asking (see `Bars::tick`).
            {
                let bars = bars.clone();
                glib::timeout_add_local(settings.tick, move || {
                    bars.tick();
                    glib::ControlFlow::Continue
                });
            }

            osd.on_shutdown({
                let bars = bars.clone();
                move || {
                    // The follower is a child process and the subscription is a
                    // socket: neither is cleaned up by this process merely dying.
                    // The tray's D-Bus names are a handle rather than a resource
                    // that frees itself, so they have to be given back
                    // explicitly.
                    events.stop();
                    follower.stop();
                    bars.release_tray();
                    // The bar is the element that owns the on-screen keyboard
                    // daemon (see `Bars::apply_tablet`), so it is also the one
                    // that has to stop it: a squeekboard started by a bar that
                    // then died would cover the screen for the rest of the
                    // session.
                    touch::osk().stop_daemon();
                }
            });

            // The shell builds one bar per output and keeps them in step with
            // GDK's monitor list; this is how it knows what to build.
            Content::PerOutput(factory)
        })
    };

    let handle = {
        let bars = bars.clone();
        Rc::new(
            move |osd: &Rc<Osd>, args: &[String]| -> Result<String, String> {
                let Some(verb) = args.first().map(String::as_str) else {
                    // No verb: started by Hyprland's autostart. The bar shows
                    // itself, so there is nothing to do but be there.
                    return Ok(String::new());
                };
                match verb {
                    // The verbs speak for every bar: this is one element with a
                    // set of pills per screen, not one element per screen.
                    "show" => {
                        osd.show();
                        Ok(String::new())
                    }
                    "hide" => {
                        osd.hide();
                        Ok(String::new())
                    }
                    "toggle" => {
                        if osd.is_visible() {
                            osd.hide();
                        } else {
                            osd.show();
                        }
                        Ok(String::new())
                    }
                    "refresh" => {
                        bars.refresh();
                        Ok(String::new())
                    }
                    "status" => Ok(bars.status()),
                    // Spelled with an `s` because that is how this collection
                    // writes English; the `z` is accepted because that is how a
                    // hand types it.
                    "minimise" | "minimize" => bars.minimise(),
                    // Without an argument: the window put away last, which is the
                    // icon nearest the indicators and what the keybinding runs.
                    "restore" => bars.restore(args.get(1).map(String::as_str)),
                    "minimised" | "minimized" => Ok(bars.minimised()),
                    // What the Type Cover's tablet-mode switch bind runs
                    // (`switch:on:` / `switch:off:` in `osd.lua`): the switch has
                    // no event a client can hear on Hyprland 0.56, only a bind, so
                    // the compositor tells the bar and the bar tells everyone else
                    // through the shared flag. `status` answers without changing
                    // anything, which is what a hand-written bind or a debugging
                    // session wants.
                    "tablet" => match args.get(1).map(String::as_str) {
                        Some("on") | Some("off") => {
                            let tablet = args.get(1).is_some_and(|arg| arg == "on");
                            // A machine with no touchscreen (or with the touch
                            // handling switched off in `touch.conf`) is never a
                            // tablet - `touch::tablet` says so by definition - so
                            // a bind that fires here must not grow the bar for a
                            // screen no finger can reach.
                            if touch::usable() {
                                touch::set_tablet(tablet);
                                bars.apply_tablet(osd, tablet);
                                // A bind may fire again while the state is
                                // already the one applied (after a resume, say),
                                // and the keyboard daemon is worth asking for
                                // every time: it is cheap, and it may have died
                                // since the fold. `apply_tablet` returns early
                                // for a state it has already applied.
                                if tablet {
                                    touch::osk().start_daemon();
                                } else {
                                    touch::osk().stop_daemon();
                                }
                            }
                            Ok(format!(
                                "touch: {}",
                                if !touch::usable() {
                                    "unavailable"
                                } else if tablet {
                                    "tablet"
                                } else {
                                    "laptop"
                                }
                            ))
                        }
                        Some("status") | None => Ok(bars.touch_status(osd)),
                        Some(other) => Err(format!(
                            "unknown tablet state `{other}` (on | off | status)"
                        )),
                    },
                    other => Err(format!(
                        "unknown command `{other}` (show | hide | toggle | refresh | status | \
                         minimise | restore | minimised | tablet)"
                    )),
                }
            },
        )
    };

    run(opts, build, handle)
}
// ---------------------------------------------------------------------------
// The bars
// ---------------------------------------------------------------------------

/// Every monitor's bar, and what they all show.
///
/// The bar is on every output, which means one [`BarView`] per screen - and one
/// *reading* for all of them, because what a bar shows (the workspaces, the
/// track, the volume, the battery) is a property of the session rather than of a
/// screen. Every view is painted from the same values, so two bars can never
/// disagree; only the surfaces differ.
///
/// The state lives here rather than in each view for one more reason: a screen
/// that is plugged in halfway through has its bar painted **immediately** from
/// the values already in hand. Waiting for the next tick would leave it without a
/// battery for up to half a minute.
struct Bars {
    settings: Rc<Settings>,
    /// The shell, so the heartbeat can ask which bars are still on screen (see
    /// [`Bars::keep_tray`]).
    osd: RefCell<Weak<Osd>>,
    /// One view per connector. Keyed rather than listed, so a screen that comes
    /// back replaces its view - the shell asks for the content again - instead of
    /// leaving two bars for the same connector behind.
    views: RefCell<BTreeMap<String, Rc<BarView>>>,
    /// Everything the bars draw, as of the last time each part was read.
    paint: RefCell<Paint>,
    /// The tray. It is a session-wide D-Bus object with one set of icons, and a
    /// widget can only be in one bar, so exactly one of the bars carries it.
    tray: RefCell<Option<tray::Tray>>,
    /// Where the windows this bar put away came from, so a click on one of their
    /// icons can put it back where it was (see [`minimise::Memory`]).
    memory: RefCell<minimise::Memory>,
    /// The screen the tray *prefers* to live on (see [`Bars::new`]).
    tray_home: String,
    /// The screen it actually ended up on, so it can be moved if that screen
    /// goes away.
    tray_output: RefCell<Option<String>>,
    /// How many heartbeats have happened, for the "every N ticks" intervals.
    ticks: Cell<u64>,
    /// Whether the cache has been filled yet (see [`Bars::adopt`]).
    warm: Cell<bool>,
    /// The tablet state the bars were last laid out for. `None` until the shell
    /// says so - the first `on_tablet_change` call from `begin_touch` is what
    /// applies the layout a bar starts with (see [`Bars::apply_tablet`]).
    tablet: Cell<Option<bool>>,
}

/// Everything the bars draw, as of the last time each part was read.
///
/// One copy for the whole session rather than one per bar: every bar shows the
/// same thing, and a bar that appears later is painted from this instead of
/// sitting empty until the next tick.
#[derive(Default)]
struct Paint {
    /// The workspace row and the focused window's title.
    hypr: hypr::Snapshot,
    /// The windows the bar has put away, in the order the tray draws them (see
    /// [`minimise`]).
    minimised: Vec<minimise::Minimised>,
    /// The clock, already formatted - `now` is the only code that knows the
    /// configured formats. The date is its own line under the time, and may be
    /// empty (a config that asked for no date).
    clock: String,
    date: String,
    /// Whether each panel is unfolded, so the pill that opened it can wear the
    /// accent. Three flags, because the three panels are three processes and any
    /// of them can be up on its own.
    island_open: bool,
    status_open: bool,
    apps_open: bool,
    /// The track and whether it is playing. The two travel together, because a
    /// pill drawn from one without the other would be a lie.
    media: Option<Track>,
    playing: bool,
    volume: Option<Volume>,
    /// `None` is "nothing read yet" - a pill that says nothing about a link -
    /// which is a different answer from the `Err` [`sources::network`] returns
    /// for "this machine has no wireless card".
    network: Option<Result<Option<Network>, ()>>,
    battery: Option<Battery>,
}

impl Bars {
    fn new(settings: Rc<Settings>) -> Self {
        // The tray is a session-wide object with one set of icons, and a widget
        // can only be in one bar - so it goes on exactly one of them: the screen
        // the config names, else the one that had the focus when the bar started
        // (the screen the user is looking at), else whichever bar comes first.
        let tray_home = settings
            .output
            .clone()
            .or_else(monitors::focused_connector)
            .unwrap_or_default();
        Bars {
            settings,
            osd: RefCell::new(Weak::new()),
            views: RefCell::new(BTreeMap::new()),
            paint: RefCell::new(Paint::default()),
            tray: RefCell::new(None),
            memory: RefCell::new(minimise::Memory::default()),
            tray_home,
            tray_output: RefCell::new(None),
            ticks: Cell::new(0),
            warm: Cell::new(false),
            tablet: Cell::new(None),
        }
    }

    /// The tablet state the bars are currently laid out for.
    fn tablet(&self) -> bool {
        self.tablet.get().unwrap_or_else(touch::tablet)
    }

    /// The shell, while this element is alive.
    fn osd(&self) -> Option<Rc<Osd>> {
        self.osd.borrow().upgrade()
    }

    /// Remember the shell, so the heartbeat can ask it which bars are on screen.
    fn attach(&self, osd: &Rc<Osd>) {
        *self.osd.borrow_mut() = Rc::downgrade(osd);
    }

    /// The bar for one output - what the shell's per-output factory calls, once
    /// for every screen that gets one.
    fn adopt(&self, connector: &str) -> Rc<BarView> {
        let view = Rc::new(BarView::new(&self.settings, connector, self.tablet()));

        // The tray goes on exactly one bar; see [`Bars::new`] for which. Until it
        // has a home, any bar that comes up takes it - unless the screen it
        // prefers is one this session has, in which case the bar that is coming up
        // now is not it and the tray keeps waiting.
        //
        // With `show_tray = false` there is no host at all: the watcher's D-Bus
        // names are a claim on the session bus, and claiming them for a row of
        // icons nobody can see would be a lie (see [`Shows::tray`]).
        if self.settings.shows.tray {
            self.host_tray(connector, &view);
        }

        // A first paint, so the bar that appears is already correct rather than
        // appearing empty and filling in over the next second - which for the
        // battery would be half a minute. The very first bar of the session is
        // what fills the cache (every source, read now); a screen plugged in
        // later is painted from what is already there.
        if !self.warm.replace(true) {
            self.refresh();
        }
        self.paint_one(&view);

        self.views
            .borrow_mut()
            .insert(connector.to_owned(), view.clone());
        view
    }

    /// Put the session's one tray on this output's bar.
    ///
    /// The tray is a single D-Bus object with a single set of icon widgets, so
    /// "moving" it means moving those widgets: they are taken out of the old
    /// row's container and appended to this one ([`tray::Tray::rehost`]). That
    /// is what makes a *rebuild* - every bar surface is destroyed and made again
    /// when the machine folds - keep its indicators instead of losing them to a
    /// container that went with the old window. Releasing the D-Bus names would
    /// not bring them back: an indicator registers once, when it starts.
    fn host_tray(&self, connector: &str, view: &BarView) {
        let icon_size = self.settings.tray_icon_size(self.tablet());
        if self.tray.borrow().is_some() {
            // Already hosted: on a rebuild this output's old row has just been
            // destroyed, so the icons move to the new one - and a fold also
            // changes the size they are drawn at.
            if self.tray_output.borrow().as_deref() == Some(connector) {
                if let Some(tray) = self.tray.borrow().as_ref() {
                    tray.rehost(view.tray_container(), icon_size);
                }
            }
            return;
        }
        if connector == self.tray_home || monitors::find(&self.tray_home).is_none() {
            *self.tray.borrow_mut() = Some(tray::Tray::host(view.tray_container(), icon_size));
            *self.tray_output.borrow_mut() = Some(connector.to_owned());
        }
    }

    /// Lay every bar out for a machine that is (or is no longer) a tablet.
    ///
    /// Called from the shell's tablet watch (which hands over the current answer
    /// once at start-up as well) and from the `tablet on|off` verb. The two
    /// numbers involved (the bar's height and the size its pictures are drawn
    /// at) are not CSS lengths, so the surfaces are made again with the new
    /// ones; the same call is what starts and stops the on-screen keyboard
    /// daemon, because the bar is the one element that is always up and
    /// therefore the one that owns it (see [`touch::Osk::start_daemon`]).
    fn apply_tablet(&self, osd: &Rc<Osd>, tablet: bool) {
        if self.tablet.get() == Some(tablet) {
            return;
        }
        self.tablet.set(Some(tablet));
        let Placement::Bar {
            margin_top,
            margin_x,
            exclusive,
            ..
        } = osd.placement()
        else {
            return;
        };
        let wanted = Placement::Bar {
            height: self.settings.bar_height(tablet),
            margin_top,
            margin_x,
            exclusive,
        };
        // The first call comes from `begin_touch` with the state the bars were
        // already built for, so the placement is often the one on screen
        // already - and rebuilding every surface for a no-op would be a visible
        // flicker at every start-up. The keyboard below still runs: it is the
        // one thing that cannot be asked about, only started and stopped.
        if osd.placement() != wanted {
            osd.set_placement(wanted);
        }
        if tablet {
            touch::osk().start_daemon();
        } else {
            touch::osk().stop_daemon();
        }
        // The rebuild asked the factory for every output's content again; the
        // views the shell no longer has a bar for (an unplugged screen from
        // before the fold) are not part of the element any more either - and the
        // tray may have lost the row it was in with them.
        self.prune_views();
        self.keep_tray();
    }

    /// Forget the views whose screens the shell has no bar for any more. The
    /// shell owns the list (a bar is reconciled against GDK's monitors), so this
    /// only mirrors it - and it never prunes everything: while the shell has no
    /// surfaces at all the views are all the element knows.
    fn prune_views(&self) {
        let Some(osd) = self.osd() else {
            return;
        };
        let on_screen = osd.outputs();
        if on_screen.is_empty() {
            return;
        }
        self.views
            .borrow_mut()
            .retain(|connector, _| on_screen.contains(connector));
    }

    /// What `tablet status` prints: the answer, the layout it means, and the
    /// keyboard the bar is responsible for.
    fn touch_status(&self, osd: &Rc<Osd>) -> String {
        let tablet = touch::tablet();
        let height = match osd.placement() {
            Placement::Bar { height, .. } => height,
            _ => self.settings.bar_height(tablet),
        };
        let usable = if touch::usable() {
            ""
        } else {
            " (no touchscreen: the state is ignored)"
        };
        [
            format!(
                "touch      {}{usable}",
                if tablet { "tablet" } else { "laptop" }
            ),
            format!(
                "bar        {height}px tall, {}px tray icons",
                self.settings.tray_icon_size(tablet)
            ),
            format!(
                "keyboard   {}",
                touch::osk().program().unwrap_or("none installed")
            ),
        ]
        .join("\n")
    }

    /// Change what the bars draw and repaint them.
    ///
    /// The one entry point for "something changed", so that a change and the
    /// repaint it causes cannot come apart.
    fn edit(&self, change: impl FnOnce(&mut Paint)) {
        change(&mut self.paint.borrow_mut());
        self.repaint();
    }

    /// Paint one bar from the cached values.
    fn paint_one(&self, view: &BarView) {
        apply(view, &self.paint.borrow());
    }

    /// Paint every bar from the cached values.
    fn repaint(&self) {
        let paint = self.paint.borrow();
        for view in self.views.borrow().values() {
            apply(view, &paint);
        }
    }

    /// Read Hyprland's own picture and paint it: the callers that have nothing
    /// else to add to the same repaint use this (the event stream, the minimise
    /// verbs).
    fn refresh_hypr(&self, event: Option<&str>) {
        let (snapshot, minimised) = self.read_hypr(event);
        self.edit(|paint| {
            paint.hypr = snapshot;
            if let Some(minimised) = minimised {
                paint.minimised = minimised;
            }
        });
    }

    /// Read what Hyprland knows about windows: the workspace row, the focused
    /// window's title, and what is put away.
    ///
    /// `event` is the event that caused the read, or `None` for "not because of
    /// an event" (the `refresh` verb, the heartbeat, a key press). Only an event
    /// that can move a window in or out of the put-away workspace is worth
    /// re-reading the tray's icons for: everything else the bar hears about (a
    /// title, a focus change) cannot change them, and `j/clients` is the largest
    /// answer Hyprland gives.
    fn read_hypr(&self, event: Option<&str>) -> (hypr::Snapshot, Option<Vec<minimise::Minimised>>) {
        let snapshot = hypr::read(self.settings.workspaces);
        let moves_windows = match event {
            Some(event) => minimise::CHANGES.contains(&event),
            None => true,
        };
        let minimised = if moves_windows {
            self.read_minimised()
        } else {
            None
        };
        (snapshot, minimised)
    }

    /// The windows that are put away, as the tray draws them.
    ///
    /// The list of them comes from Hyprland and the order and the way back from
    /// the bar's own memory, so the two are reconciled here, in one place:
    /// whatever is not put away any more is forgotten, and whatever Hyprland
    /// knows but the bar does not is drawn anyway (see [`minimise::Memory`]).
    ///
    /// `None` when Hyprland cannot be asked - the tray then keeps the icons it
    /// has, because an empty tray would state that nothing is put away.
    fn read_minimised(&self) -> Option<Vec<minimise::Minimised>> {
        let put_away = minimise::read()?;
        let mut memory = self.memory.borrow_mut();
        memory.retain(&put_away);
        Some(memory.arrange(put_away))
    }

    /// Put the focused window away - what the bar's one keybinding runs.
    ///
    /// The icons are re-read here rather than left to the event stream: the icon
    /// arriving with the key press is the feedback the press deserves, and the
    /// title pill moves on too, because the focus has moved to another window.
    fn minimise(&self) -> Result<String, String> {
        let message = minimise::minimise(&mut self.memory.borrow_mut())?;
        self.refresh_hypr(None);
        Ok(message)
    }

    /// Bring a put-away window back: the one `what` names - a window address, or
    /// a number counting from 1 in the order the tray draws them - else the one
    /// put away last, which is what the keybinding asks for.
    fn restore(&self, what: Option<&str>) -> Result<String, String> {
        let Some(entries) = self.read_minimised() else {
            return Err("Hyprland did not answer".to_string());
        };
        let entry = match what {
            None => entries.first(),
            Some(what) => entries
                .iter()
                .find(|entry| entry.matches(what))
                .or_else(|| {
                    // A number counts the icons from 1, the way the digits count tiles
                    // in the overview and the switcher.
                    what.parse::<usize>()
                        .ok()
                        .and_then(|number| number.checked_sub(1))
                        .and_then(|index| entries.get(index))
                }),
        };
        let Some(entry) = entry else {
            // Say how much there *is*: "no window matches `9`" on a tray that
            // happens to be empty is a different story from the same answer when
            // two windows are waiting.
            let count = match entries.len() {
                0 => "nothing is put away".to_string(),
                1 => "1 window is put away".to_string(),
                count => format!("{count} windows are put away"),
            };
            return Err(match what {
                None => "nothing is put away".to_string(),
                Some(what) => format!("no put-away window matches `{what}` ({count})"),
            });
        };
        let message = minimise::bring_back(entry)?;
        self.refresh_hypr(None);
        Ok(message)
    }

    /// What the `minimised` verb prints: one line per put-away window, in the
    /// order the tray draws them.
    fn minimised(&self) -> String {
        let Some(entries) = self.read_minimised() else {
            return "Hyprland did not answer".to_string();
        };
        if entries.is_empty() {
            return "nothing is put away".to_string();
        }
        entries
            .iter()
            .map(minimise::Minimised::describe)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Read every source into the cache and show it: the `refresh` verb, and how
    /// the first bar of the session paints itself.
    ///
    /// This is the *full* read. The heartbeat reads a slice of it at a time
    /// instead, each source on its own clock (see [`Bars::tick`]).
    fn refresh(&self) {
        let settings = self.settings.clone();
        let (snapshot, minimised) = self.read_hypr(None);
        let volume = sources::volume();
        let network = sources::network(&settings.interface);
        let battery = sources::battery(&settings.battery, &settings.adapter);
        let media = mpris::current().ok();
        let clock = now(&settings.clock_format);
        let date = now(&settings.date_format);
        let island_open = state::read(&state::island_panel());
        let status_open = state::read(&state::stats_panel());
        let apps_open = state::read(&state::apps_panel());
        self.edit(|paint| {
            paint.hypr = snapshot;
            if let Some(minimised) = minimised {
                paint.minimised = minimised;
            }
            paint.clock = clock;
            paint.date = date;
            paint.island_open = island_open;
            paint.status_open = status_open;
            paint.apps_open = apps_open;
            paint.volume = volume;
            paint.network = Some(network);
            paint.battery = battery;
            match media {
                Some(event) => {
                    paint.media = Some(event.track);
                    paint.playing = event.playing;
                }
                None => {
                    paint.media = None;
                    paint.playing = false;
                }
            }
        });
    }

    /// One heartbeat: the clock every tick, everything else on its own interval.
    ///
    /// `count` is how many ticks have happened, so "every N milliseconds" is
    /// `count % (interval / tick) == 0` - which stays honest if someone edits
    /// `tick_ms` in the config rather than the interval. The whole tick changes
    /// the cache and then repaints **once**, so a bar can never be caught showing
    /// half of an update, and every bar shows the same half of nothing.
    fn tick(&self) {
        let count = self.ticks.get() + 1;
        self.ticks.set(count);
        let settings = self.settings.clone();
        let due = |interval: u64| every(count, interval, settings.tick);
        let resync = due(settings.resync_every);

        // Read before the repaint (see [`Bars::read_hypr`]): a resync is the
        // safety net for an event socket that quietly died, so it reads exactly
        // what the event path reads - including the tray's icons, which no event
        // may have mentioned for a long while.
        let resynced = resync.then(|| self.read_hypr(None));

        self.edit(|paint| {
            paint.clock = now(&settings.clock_format);
            paint.date = now(&settings.date_format);
            // While a popup is unfolded, the pill that opened it wears the accent.
            // The flags are files because each panel is a separate process (see
            // `state`).
            paint.island_open = state::read(&state::island_panel());
            paint.status_open = state::read(&state::stats_panel());
            paint.apps_open = state::read(&state::apps_panel());

            if due(settings.volume_every) {
                paint.volume = sources::volume();
            }
            if due(settings.network_every) {
                // `Err` is "this machine has no wireless card, or no `iw` to
                // ask", which is a different answer from "there is a card and
                // nothing is joined" - see `sources`. The pill draws neither, but
                // it does draw a card with nothing joined as "off".
                paint.network = Some(sources::network(&settings.interface));
            }
            if due(settings.battery_every) {
                paint.battery = sources::battery(&settings.battery, &settings.adapter);
            }
            if let Some((snapshot, minimised)) = resynced {
                // The event socket is the real mechanism, so this is only the
                // safety net - for one that quietly died, or for Hyprland being
                // restarted under a running bar.
                paint.hypr = snapshot;
                if let Some(minimised) = minimised {
                    paint.minimised = minimised;
                }
                // `playerctl --follow` only speaks when something changes;
                // asking again here is what makes the pill correct after a player
                // exits without saying goodbye.
                match mpris::current() {
                    Ok(event) => {
                        paint.media = Some(event.track);
                        paint.playing = event.playing;
                    }
                    Err(_) => {
                        paint.media = None;
                        paint.playing = false;
                    }
                }
            }
        });

        if resync {
            // A screen can also appear or go without the shell acting on GDK's
            // monitor list in time (and a monitor added is only noticed by the
            // shell's own hook), so this is the belt to that braces - cheap,
            // because nothing happens when nothing changed.
            if let Some(osd) = self.osd.borrow().upgrade() {
                osd.sync_outputs();
            }
            self.prune_views();
            self.keep_tray();
        }
    }

    /// The one-line-per-pill summary the `status` verb prints.
    fn status(&self) -> String {
        self.refresh();

        let paint = self.paint.borrow();
        let workspaces: Vec<String> = paint
            .hypr
            .workspaces
            .iter()
            .map(|workspace| {
                format!(
                    "{}{}{}",
                    workspace.id,
                    if workspace.active {
                        "*"
                    } else if workspace.visible {
                        "~"
                    } else {
                        ""
                    },
                    if workspace.windows > 0 { "!" } else { "" }
                )
            })
            .collect();
        let on_screen = self.on_screen();
        let tray = self
            .tray_output
            .borrow()
            .clone()
            .unwrap_or_else(|| "nowhere".to_string());
        let hidden = self.settings.shows.hidden();
        // The height is live rather than configured: a fold makes the bar taller,
        // and a `status` read should say what is on screen now.
        let (height, margin_top) = match self.osd().map(|osd| osd.placement()) {
            Some(Placement::Bar {
                height, margin_top, ..
            }) => (height, margin_top),
            _ => (
                self.settings.bar_height(self.tablet()),
                self.settings.margin_top,
            ),
        };

        [
            format!(
                "bar        {height}px tall, {margin_top}px below the top, on {}",
                if on_screen.is_empty() {
                    "no monitor".to_string()
                } else {
                    on_screen.join(", ")
                }
            ),
            format!(
                "touch      {}",
                if !touch::usable() {
                    "no touchscreen".to_string()
                } else if self.tablet() {
                    format!(
                        "tablet (bar {}px, tray icons {}px)",
                        self.settings.bar_height(true),
                        self.settings.tray_icon_size(true)
                    )
                } else {
                    "laptop".to_string()
                }
            ),
            // Which pills this bar is *supposed* to have, so a part that is
            // missing because of a `show_*` switch is not chased as a bug.
            format!(
                "hidden     {}",
                if hidden.is_empty() {
                    "nothing".to_string()
                } else {
                    hidden.join(", ")
                }
            ),
            format!(
                "tray       {}",
                if self.settings.shows.tray {
                    format!("on {tray}")
                } else {
                    "off (show_tray = false)".to_string()
                }
            ),
            format!(
                "minimised  {}",
                if paint.minimised.is_empty() {
                    "nothing is put away".to_string()
                } else {
                    paint
                        .minimised
                        .iter()
                        .map(|entry| entry.window.label().to_string())
                        .collect::<Vec<_>>()
                        .join(" · ")
                }
            ),
            format!(
                "workspaces {}  (*you are here, ~other monitor, !has windows)",
                workspaces.join(" ")
            ),
            format!(
                "title      {}",
                if paint.hypr.title.is_empty() {
                    "-"
                } else {
                    &paint.hypr.title
                }
            ),
            format!(
                "media      {}",
                paint
                    .media
                    .as_ref()
                    .map(|track| format!(
                        "{}{}",
                        track.summary(),
                        if paint.playing { "" } else { " (paused)" }
                    ))
                    .unwrap_or_else(|| "nothing playing".to_string())
            ),
            // The three parts of the combined status pill, on their own lines:
            // they are read on three different clocks, so a line each is the only
            // way to see which of them is the stale one.
            format!(
                "link       {}",
                match paint.network.clone() {
                    None | Some(Err(())) => "no wireless interface".to_string(),
                    Some(Ok(None)) => "not connected".to_string(),
                    Some(Ok(Some(link))) => {
                        format!("{} at {}% ({} dBm)", link.ssid, link.signal, link.dbm)
                    }
                }
            ),
            format!(
                "sound      {}",
                paint
                    .volume
                    .map(|volume| format!(
                        "{}%{}",
                        volume.percent,
                        if volume.muted { " (muted)" } else { "" }
                    ))
                    .unwrap_or_else(|| "no sink".to_string())
            ),
            format!(
                "battery    {}",
                paint
                    .battery
                    .map(|battery| format!("{}%", battery.percent))
                    .unwrap_or_else(|| "none".to_string())
            ),
        ]
        .join("\n")
    }

    /// The connectors the bars are on, as the *shell* sees it: the outputs it
    /// actually has surfaces for, rather than the ones this element believes in.
    fn on_screen(&self) -> Vec<String> {
        self.osd
            .borrow()
            .upgrade()
            .map(|osd| osd.outputs())
            .filter(|outputs| !outputs.is_empty())
            .unwrap_or_else(|| self.views.borrow().keys().cloned().collect())
    }

    /// Keep the tray on a bar that still exists.
    ///
    /// A screen that is unplugged takes its bar - and the row the icons were in -
    /// with it, while the D-Bus objects and their buttons are session-wide and
    /// still alive. The widgets are therefore moved to a bar that is still on
    /// screen rather than re-hosted: an indicator registers with a watcher once,
    /// when it starts, so giving the D-Bus names back would cost the icons.
    fn keep_tray(&self) {
        if !self.settings.shows.tray {
            return;
        }
        let Some(hosted) = self.tray_output.borrow().clone() else {
            return;
        };
        let on_screen = self.on_screen();
        if on_screen.contains(&hosted) {
            return;
        }
        let views = self.views.borrow();
        let Some(view) = on_screen
            .iter()
            .find_map(|connector| views.get(connector).cloned())
        else {
            return; // nothing on screen to put it on
        };
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.rehost(
                view.tray_container(),
                self.settings.tray_icon_size(self.tablet()),
            );
            *self.tray_output.borrow_mut() = Some(view.connector().to_owned());
        }
    }

    /// Give the tray's D-Bus names back (see [`tray::Tray::release`]).
    fn release_tray(&self) {
        if let Some(tray) = self.tray.borrow_mut().take() {
            tray.release();
        }
    }
}

/// Paint one bar from the cached values.
fn apply(view: &BarView, paint: &Paint) {
    view.render_hypr(&paint.hypr);
    view.render_minimised(&paint.minimised);
    view.render_clock(&paint.clock, &paint.date);
    view.set_island_open(paint.island_open);
    view.set_status_open(paint.status_open);
    view.set_apps_open(paint.apps_open);
    view.render_media(paint.media.as_ref(), paint.playing);
    view.set_volume(paint.volume);
    if let Some(network) = paint.network.clone() {
        view.set_network(network);
    }
    view.set_battery(paint.battery);
}

/// Whether this tick is the one for an interval.
fn every(count: u64, interval_ms: u64, tick: Duration) -> bool {
    let tick_ms = tick.as_millis().max(1) as u64;
    let ticks = (interval_ms / tick_ms).max(1);
    count.is_multiple_of(ticks)
}

/// The current local time in a `strftime` format, through glib - the same C
/// library `strftime` the rest of the desktop's configs assume, and a date
/// library this program does not need to link.
fn now(pattern: &str) -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|now| now.format(pattern).ok())
        .map(|text| text.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_interval_is_expressed_in_ticks() {
        let tick = Duration::from_millis(1000);
        assert!(every(1, 1000, tick));
        assert!(!every(1, 5000, tick));
        assert!(every(5, 5000, tick));
        assert!(every(30, 30000, tick));
    }

    #[test]
    fn an_interval_shorter_than_the_tick_is_every_tick() {
        let tick = Duration::from_millis(1000);
        // 200ms cannot be honoured by a 1s heartbeat, so it is not skipped for
        // half the time - it happens every tick.
        assert!(every(1, 200, tick));
        assert!(every(7, 200, tick));
    }

    #[test]
    fn a_zero_tick_cannot_divide_by_zero() {
        // A zero tick is nonsense (the main loop would spin), but it must not
        // panic: every interval is then counted in milliseconds instead.
        assert!(every(1000, 1000, Duration::ZERO));
        assert!(!every(3, 1000, Duration::ZERO));
    }

    #[test]
    fn every_part_of_the_bar_is_shown_unless_the_config_hides_it() {
        let path = std::env::temp_dir().join("hypr-osd-bar-shows-test.conf");
        std::fs::write(&path, "show_media = false\nshow_tray = false\n").unwrap();
        let shows = Shows::load(&Config::load_from(path.clone()));
        let _ = std::fs::remove_file(&path);

        // The two the file switched off are off, in the order the bar draws
        // them...
        assert!(!shows.media);
        assert!(!shows.tray);
        assert_eq!(shows.hidden(), vec!["media", "tray"]);
        // ...and everything else is still on, because a bar that lost a pill
        // nobody asked it to lose would read as a bug.
        assert!(shows.apps && shows.workspaces && shows.title && shows.clock);
        assert!(shows.status && shows.minimised && shows.power);

        let path = std::env::temp_dir().join("hypr-osd-bar-shows-default-test.conf");
        std::fs::write(&path, "height = 40\n").unwrap();
        let shows = Shows::load(&Config::load_from(path.clone()));
        let _ = std::fs::remove_file(&path);
        assert!(shows.hidden().is_empty(), "a fresh config hides nothing");
    }

    #[test]
    fn a_tablet_gets_a_taller_bar_and_bigger_pictures() {
        // The laptop keeps exactly the numbers the config (or the defaults) say.
        assert_eq!(bar_height(40, false), 40);
        assert_eq!(tray_icon_size(18, false), 18);
        // A fold grows the bar by the extra the pointer-sized pills cannot fit
        // in, and the pictures with it.
        assert_eq!(bar_height(40, true), 40 + TOUCH_HEIGHT_EXTRA);
        assert_eq!(tray_icon_size(18, true), 24);
        // A config that asks for big icons already keeps them, only larger.
        assert_eq!(tray_icon_size(40, true), 46);
        // And a non-default bar height is grown, not replaced: a user who chose
        // 48 still gets their choice plus the finger's room.
        assert_eq!(bar_height(48, true), 48 + TOUCH_HEIGHT_EXTRA);
    }
}

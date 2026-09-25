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
//!   (no verb)            start the bar (what the autostart runs)
//! ```
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
use hypr_osd_core::system::{self as stats, Reading};
use hypr_osd_core::{css, monitors, run, state, Config, Content, Opts, Osd, Placement};

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
const DEFAULT_MARGIN_TOP: i32 = 8;
const DEFAULT_MARGIN_X: i32 = 12;
/// How many numbered workspaces the row keeps room for, so a workspace you have
/// never visited is still clickable.
const DEFAULT_WORKSPACES: i32 = 5;
/// How much room the window title may take before it is ellipsised.
const DEFAULT_TITLE_WIDTH: i32 = 320;
const DEFAULT_BATTERY: &str = "BAT1";
const DEFAULT_ADAPTER: &str = "ADP1";
const DEFAULT_INTERFACE: &str = "wlan0";
const DEFAULT_ICON_SIZE: i32 = 18;
const DEFAULT_CLOCK_FORMAT: &str = "%H:%M";
const DEFAULT_TICK_MS: u64 = 1000;
const DEFAULT_VOLUME_EVERY_MS: u64 = 1000;
const DEFAULT_NETWORK_EVERY_MS: u64 = 5000;
const DEFAULT_BATTERY_EVERY_MS: u64 = 30000;
/// CPU load, memory and temperature: cheap file reads, so they keep up with the
/// heartbeat. (The CPU number is a delta between two readings, which is why the
/// pill appears on the second tick rather than the first.)
const DEFAULT_STATS_EVERY_MS: u64 = 1000;
/// How often a stale update count is refreshed. `checkupdates` syncs a pacman
/// database in a temporary directory, and the count only changes when somebody
/// syncs anyway - so half an hour, which is also how long a count is believed
/// (see `stats::UPDATES_MAX_AGE`).
const DEFAULT_UPDATES_EVERY_MS: u64 = 1_800_000;
/// How often the bar re-reads Hyprland even with no event, and re-asks MPRIS.
/// The event socket is the real mechanism; this is the safety net for a socket
/// that quietly died, which would otherwise leave the row frozen for the
/// session and look like a bug in the bar.
const DEFAULT_RESYNC_EVERY_MS: u64 = 5000;

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
    workspaces: i32,
    title_width: i32,
    battery: String,
    adapter: String,
    interface: String,
    icon_size: i32,
    clock_format: String,
    tick: Duration,
    volume_every: u64,
    network_every: u64,
    battery_every: u64,
    stats_every: u64,
    updates_every: u64,
    resync_every: u64,
    /// What the clock's hover and clicks run. All of them are programs the
    /// desktop already has, and every one of them is replaceable: the bar only
    /// ever *runs* them, so pointing one elsewhere is a config line.
    island_command: String,
    notifications_command: String,
    volume_command: String,
    session_command: String,
    terminal_command: String,
    /// The system popup the status pill unfolds. It is asked to open on hover,
    /// to toggle on click, and to switch the wireless radio on a right click.
    stats_command: String,
    /// What prints the pending-update list (`checkupdates`). Empty turns the
    /// update count off: the pill then shows only the three readings.
    updates_command: String,
}

impl Settings {
    fn load(config: &Config) -> Self {
        Settings {
            height: config.i32("height", DEFAULT_HEIGHT).max(1),
            margin_top: config.i32("margin_top", DEFAULT_MARGIN_TOP).max(0),
            margin_x: config.i32("margin_x", DEFAULT_MARGIN_X).max(0),
            exclusive: config.bool("exclusive", true),
            output: Some(config.string("output", "")).filter(|output| !output.is_empty()),
            workspaces: config.i32("workspaces", DEFAULT_WORKSPACES).max(0),
            title_width: config.i32("title_width", DEFAULT_TITLE_WIDTH).max(40),
            battery: config.string("battery", DEFAULT_BATTERY),
            adapter: config.string("adapter", DEFAULT_ADAPTER),
            interface: config.string("network_interface", DEFAULT_INTERFACE),
            icon_size: config.i32("tray_icon_size", DEFAULT_ICON_SIZE).max(8),
            clock_format: config.string("clock_format", DEFAULT_CLOCK_FORMAT),
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
            stats_every: config
                .millis("stats_every_ms", DEFAULT_STATS_EVERY_MS)
                .as_millis() as u64,
            updates_every: config
                .millis("updates_every_ms", DEFAULT_UPDATES_EVERY_MS)
                .as_millis() as u64,
            resync_every: config
                .millis("resync_every_ms", DEFAULT_RESYNC_EVERY_MS)
                .as_millis() as u64,
            island_command: config.string("island_command", "hypr-osd-island"),
            notifications_command: config.string("notifications_command", "swaync-client"),
            volume_command: config.string("volume_command", "hypr-osd-volume"),
            session_command: config.string("session_command", "hypr-osd-session"),
            terminal_command: config.string("terminal_command", "kitty"),
            stats_command: config.string("stats_command", "hypr-osd-stats"),
            updates_command: config.string("updates_command", "checkupdates"),
        }
    }
}

fn main() -> glib::ExitCode {
    let config = Config::load(ELEMENT);
    let settings = Rc::new(Settings::load(&config));

    let opts = Opts {
        app_id: APP_ID.to_string(),
        css: css::stylesheet(include_str!("bar.css")),
        namespace: NAMESPACE.to_string(),
        placement: Placement::Bar {
            height: settings.height,
            margin_top: settings.margin_top,
            margin_x: settings.margin_x,
            exclusive: settings.exclusive,
        },
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
    let bars = Rc::new_cyclic(|me| Bars::new(settings.clone(), me.clone()));

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

            // 1. The tray: a StatusNotifier host, kept for the session; dropping
            //    it releases the D-Bus names and the icons go with them. It is
            //    *hosted* by `Bars::adopt`, because it is a session-wide object
            //    with one set of icons and a widget can only be in one bar - so
            //    exactly one of the bars carries it.

            // 2. Hyprland: the workspace row and the window title, kept up to
            //    date by the event stream; the heartbeat re-reads them too, as
            //    the safety net for a socket that quietly died.
            let events = hypr::watch({
                let bars = bars.clone();
                let settings = settings.clone();
                move || bars.edit(|paint| paint.hypr = hypr::read(settings.workspaces))
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

            // 4. The system info pill. The reading is a delta between two ticks,
            //    so the pill appears on the second one; the update count is either
            //    in the cache or asked for in the background.
            if settings.updates_command.is_empty() {
                // Turned off in the config: no count, and nothing runs.
            } else if let Some(count) = stats::cached_updates() {
                // A fresh cache from the last half hour: no reason to ask again.
                bars.updates.set(count);
            } else {
                // Nothing usable cached, so ask once. Asynchronously, because
                // `checkupdates` syncs a pacman database first.
                stats::refresh_updates(&settings.updates_command, refresher(&bars));
            }

            // 5. The tick. One timer for everything periodic, so the bars have a
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
                    other => Err(format!(
                        "unknown command `{other}` (show | hide | toggle | refresh | status)"
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
    /// A weak self-reference, so the heartbeat can hand `checkupdates` a callback
    /// that keeps the bars alive until it answers.
    me: Weak<Bars>,
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
    /// The screen the tray *prefers* to live on (see [`Bars::new`]).
    tray_home: String,
    /// The screen it actually ended up on, so it can be moved if that screen
    /// goes away.
    tray_output: RefCell<Option<String>>,
    /// The system info pill's readings. The CPU number is a *delta* between two
    /// samples, so the sampler has to outlive a single reading.
    sampler: RefCell<stats::Sampler>,
    reading: RefCell<Option<Reading>>,
    updates: Cell<i32>,
    /// How many heartbeats have happened, for the "every N ticks" intervals.
    ticks: Cell<u64>,
    /// Whether the cache has been filled yet (see [`Bars::adopt`]).
    warm: Cell<bool>,
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
    /// The clock, already formatted - `now` is the only code that knows the
    /// configured format.
    clock: String,
    /// Whether each popup is unfolded, so its pill can wear the accent. Two
    /// flags, because the two panels are two processes and either can be up on
    /// its own.
    island_open: bool,
    status_open: bool,
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
    fn new(settings: Rc<Settings>, me: Weak<Bars>) -> Self {
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
            me,
            osd: RefCell::new(Weak::new()),
            views: RefCell::new(BTreeMap::new()),
            paint: RefCell::new(Paint::default()),
            tray: RefCell::new(None),
            tray_home,
            tray_output: RefCell::new(None),
            sampler: RefCell::new(stats::Sampler::new()),
            reading: RefCell::new(None),
            updates: Cell::new(0),
            ticks: Cell::new(0),
            warm: Cell::new(false),
        }
    }

    /// Remember the shell, so the heartbeat can ask it which bars are on screen.
    fn attach(&self, osd: &Rc<Osd>) {
        *self.osd.borrow_mut() = Rc::downgrade(osd);
    }

    /// The bar for one output - what the shell's per-output factory calls, once
    /// for every screen that gets one.
    fn adopt(&self, connector: &str) -> Rc<BarView> {
        let view = Rc::new(BarView::new(&self.settings, connector));

        // The tray goes on exactly one bar; see [`Bars::new`] for which. Until it
        // has a home, any bar that comes up takes it - unless the screen it
        // prefers is one this session has, in which case the bar that is coming up
        // now is not it and the tray keeps waiting.
        if self.tray.borrow().is_none()
            && (connector == self.tray_home || monitors::find(&self.tray_home).is_none())
        {
            *self.tray.borrow_mut() = Some(tray::Tray::host(
                view.tray_container(),
                self.settings.icon_size,
            ));
            *self.tray_output.borrow_mut() = Some(connector.to_owned());
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
        let paint = self.paint.borrow();
        let reading = self.reading.borrow();
        apply(view, &paint, reading.as_ref(), self.updates.get());
    }

    /// Paint every bar from the cached values.
    fn repaint(&self) {
        let paint = self.paint.borrow();
        let reading = self.reading.borrow();
        for view in self.views.borrow().values() {
            apply(view, &paint, reading.as_ref(), self.updates.get());
        }
    }

    /// Read every source into the cache and show it: the `refresh` verb, and how
    /// the first bar of the session paints itself.
    ///
    /// This is the *full* read. The heartbeat reads a slice of it at a time
    /// instead, each source on its own clock (see [`Bars::tick`]).
    fn refresh(&self) {
        let settings = self.settings.clone();
        let snapshot = hypr::read(settings.workspaces);
        let volume = sources::volume();
        let network = sources::network(&settings.interface);
        let battery = sources::battery(&settings.battery, &settings.adapter);
        let media = mpris::current().ok();
        let clock = now(&settings.clock_format);
        let island_open = state::read(&state::island_panel());
        let status_open = state::read(&state::stats_panel());
        self.edit(|paint| {
            paint.hypr = snapshot;
            paint.clock = clock;
            paint.island_open = island_open;
            paint.status_open = status_open;
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

        // The system info pill comes first, and outside the repaint: the *timer*
        // is what holds the previous CPU sample, because the pill is the delta
        // between two of them.
        if due(settings.stats_every) {
            *self.reading.borrow_mut() = Some(self.sampler.borrow_mut().read());
        }
        if due(settings.updates_every)
            && !settings.updates_command.is_empty()
            && stats::cached_updates().is_none()
        {
            // The count is believed for half an hour (see
            // `stats::UPDATES_MAX_AGE`), and this is what asks again once it goes
            // stale - in the background, so a slow mirror never touches the bars.
            if let Some(me) = self.me.upgrade() {
                stats::refresh_updates(&settings.updates_command, refresher(&me));
            }
        }

        self.edit(|paint| {
            paint.clock = now(&settings.clock_format);
            // While a popup is unfolded, its pill wears the accent. The flags are
            // files because each popup is a separate process (see `state`).
            paint.island_open = state::read(&state::island_panel());
            paint.status_open = state::read(&state::stats_panel());

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
            if resync {
                // The event socket is the real mechanism, so this is only the
                // safety net - for one that quietly died, or for Hyprland being
                // restarted under a running bar.
                paint.hypr = hypr::read(settings.workspaces);
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
            self.keep_tray();
        }
    }

    /// The one-line-per-pill summary the `status` verb prints.
    ///
    /// A status read should be true when it is printed, and the heartbeat may not
    /// have produced a reading yet (in the first second of the bars' life, or if
    /// the daemon was started a moment ago) - so take one now. That costs two
    /// samples 200 ms apart, which is only ever paid here.
    fn status(&self) -> String {
        *self.reading.borrow_mut() = Some(stats::read_now());
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

        [
            format!(
                "bar        {}px tall, {}px below the top, on {}",
                self.settings.height,
                self.settings.margin_top,
                if on_screen.is_empty() {
                    "no monitor".to_string()
                } else {
                    on_screen.join(", ")
                }
            ),
            format!("tray       on {tray}"),
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
            format!(
                "system     {}",
                describe_stats(&self.reading.borrow(), self.updates.get())
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
    /// A screen that is unplugged takes its bar - and the icons in it - with it,
    /// because the widget they lived in went with the surface. This only ever
    /// *moves* an orphaned tray: re-hosting one that is up would cost the icons,
    /// since an indicator registers with a watcher once, when it starts.
    fn keep_tray(&self) {
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
        self.release_tray();
        *self.tray.borrow_mut() = Some(tray::Tray::host(
            view.tray_container(),
            self.settings.icon_size,
        ));
        *self.tray_output.borrow_mut() = Some(view.connector().to_owned());
    }

    /// Give the tray's D-Bus names back (see [`tray::Tray::release`]).
    fn release_tray(&self) {
        if let Some(tray) = self.tray.borrow_mut().take() {
            tray.release();
        }
    }
}

/// Paint one bar from the cached values.
fn apply(view: &BarView, paint: &Paint, reading: Option<&Reading>, updates: i32) {
    view.render_hypr(&paint.hypr);
    view.render_clock(&paint.clock);
    view.set_island_open(paint.island_open);
    view.set_status_open(paint.status_open);
    view.render_media(paint.media.as_ref(), paint.playing);
    view.set_volume(paint.volume);
    if let Some(network) = paint.network.clone() {
        view.set_network(network);
    }
    view.set_battery(paint.battery);
    // The readings come from the heartbeat, which is the only thing that can take
    // two CPU samples. Before the first one there is nothing honest to draw, and
    // the pill stays away.
    if let Some(reading) = reading {
        view.render_stats(reading, updates);
    }
}

/// Whether this tick is the one for an interval.
fn every(count: u64, interval_ms: u64, tick: Duration) -> bool {
    let tick_ms = tick.as_millis().max(1) as u64;
    let ticks = (interval_ms / tick_ms).max(1);
    count.is_multiple_of(ticks)
}

/// What to do when `checkupdates` answers: remember the count, and repaint.
///
/// A closure rather than an inline block, because the same thing has to happen
/// for the start-up refresh and for the half-hourly one.
fn refresher(bars: &Rc<Bars>) -> impl FnOnce(i32) + 'static {
    let bars = bars.clone();
    move |count| {
        bars.updates.set(count);
        bars.repaint();
    }
}

/// The `status` line for the system info pill.
fn describe_stats(reading: &Option<stats::Reading>, updates: i32) -> String {
    let Some(reading) = reading else {
        return "no reading yet (the bar samples once a second)".to_string();
    };
    let temperature = match reading.temperature {
        Some(temperature) => format!("{temperature:.0}°C"),
        None => "no sensor".to_string(),
    };
    format!(
        "CPU {:.0}% · memory {:.0}% · {temperature} · {}",
        reading.cpu,
        reading.memory,
        match updates {
            0 => "no updates pending".to_string(),
            1 => "1 pending update".to_string(),
            count => format!("{count} pending updates"),
        }
    )
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
}

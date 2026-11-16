//! `hypr-osd-island` - the panel that unfolds from the bar's player pill.
//!
//! The bar's player pill is the handle; this is what comes out of it. Hovering
//! the pill asks this program to open (`hypr-osd-island open`), and from then on
//! the panel looks after itself: it samples the pointer until either the dwell is
//! up (open) or the pointer has been away long enough (close), because *leaving*
//! means moving onto the panel itself - somewhere the bar cannot see.
//!
//! It is a separate program from the bar on purpose. A GTK widget can never paint
//! outside its own window, so an "expansion" of a 40-pixel pill is necessarily a
//! second layer-shell surface - and a second surface inside the bar's process
//! would mean the bar carrying a calendar, a progress bar and a notification hub
//! for the whole session to draw them for a few seconds at a time.
//!
//! Verbs (`hypr-osd-island <verb>`):
//!
//! ```text
//!   open [connector] [x y w h]
//!                        start watching: the panel appears if the pointer stays.
//!                        The connector is the monitor the bar is on, which is
//!                        what the hot zone is measured against (the bar is
//!                        pinned to the output it started on, and that is not
//!                        always the focused one). The four numbers are the
//!                        player pill's rectangle inside the bar's card - the
//!                        bar measures it, because the pill is as wide as its
//!                        track.
//!   tap [connector] [x y w h]
//!                        a finger tapped the player: open *now*, pinned, with a
//!                        tap catcher around the card - a second tap (or one
//!                        anywhere outside the card) takes it away. Same
//!                        arguments as `open`, because it is the same request
//!                        from a different gesture.
//!   close                take it away now
//!   toggle               close it if it is up, otherwise show it
//!   show                 put it up and keep it there (no pointer tracking)
//!   status               print what the panel would show, up or not
//!                        (including the notifications the bus carried)
//!   (no verb)            start the daemon and wait for the bar
//! ```
//!
//! A **finger** never uses the hover machine at all: it has no pointer to rest
//! on the player and none to move away afterwards, and GDK turns its tap into an
//! emulated pointer that stays where the finger lifted. So the bar sends `tap`
//! instead of `open`, and this panel opens pinned with a *tap catcher* around
//! its card (see `Osd::tap_catcher`) - a dimmed surface whose four bands turn a
//! tap anywhere else into "close". Any other way out (a `close`, a `toggle`, the
//! bar's player tapped again) takes the catcher down with it; `Osd::hide` would
//! hide it as a backstop, but a card can be shut while its dim is still up, so
//! the close paths say so explicitly.

mod calendar;
mod notify;
mod view;

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::hover::{self, Action, BarGeometry, Delays, Machine, Side};
use hypr_osd_core::island::{Island, Origin};
use hypr_osd_core::{
    css, run, state, touch, Config, Content, Opts, Osd, Placement, CARD_PAD_X, SHADOW_PAD,
};

use notify::Notifications;
use view::IslandView;

/// D-Bus application id - and therefore the single-instance key: the `open` the
/// bar runs reaches the running panel instead of starting a second one.
const APP_ID: &str = "com.schells2.osd.island";
/// Layer-shell namespace, the collection's shared prefix.
const NAMESPACE: &str = "hypr-osd";
/// Element name: `~/.config/hypr-osd/island.conf`.
const ELEMENT: &str = "island";

// Defaults. The geometry ones mirror the bar's own defaults, because this panel
// has to know where the bar is without being told: it hangs itself below the
// bar's bottom edge, and its hot zone is the rectangle the bar sends.
const DEFAULT_BAR_HEIGHT: i32 = 40;
const DEFAULT_BAR_MARGIN_TOP: i32 = 0;
const DEFAULT_BAR_MARGIN_X: i32 = 0;
const DEFAULT_GAP: i32 = 6;
/// Only used for an `open` that arrives without a rectangle (a hand-typed one,
/// or a bar too old to send one): the width of the strip in the middle of the
/// bar that counts as "over the player pill". The bar normally measures the
/// pill and sends it, because the player is as wide as its track.
const DEFAULT_HOT_WIDTH: i32 = 280;
const DEFAULT_LEFT_WIDTH: i32 = 252;
const DEFAULT_RIGHT_WIDTH: i32 = 178;
const DEFAULT_OPEN_DELAY_MS: u64 = 120;
const DEFAULT_CLOSE_DELAY_MS: u64 = 300;
const DEFAULT_POLL_MS: u64 = 50;
const DEFAULT_TICK_MS: u64 = 1000;
/// How much taller the *bar* is while the machine is a tablet, in logical
/// pixels. The bar grows so a fingertip can hit its pills (44px targets in a
/// 52px strip); this panel hangs below it and has to know the same number,
/// because its own surface position and the hole it leaves in its tap catcher
/// are both measured from `island.conf`'s `bar_height` - which is the *laptop*
/// height. This must match `TOUCH_HEIGHT_EXTRA` in `hypr-osd-bar`: the two
/// programs never talk about their geometry, they both read the machine's state
/// and the config files, so a value that drifted would put the panel in the bar
/// or leave a gap under it.
const TOUCH_HEIGHT_EXTRA: i32 = 12;

/// How much wider the panel is while the machine is a tablet, in logical pixels.
///
/// The touch rules in `island.css` grow the state chips in the header and the
/// transport row, and those are horizontal: a card that kept its pointer width
/// would measure wider than the slot it is asked for (GTK then warns and the
/// size the tap catcher's hole is measured from stops matching the card the
/// shell draws). The width is one number for the whole card, so it grows by the
/// room the finger-sized controls needed.
const TOUCH_WIDTH_EXTRA: i32 = 96;
/// How often the panel re-reads what is playing while it is up. The follower in
/// the *bar* is what announces a new track; this is what keeps the progress bar
/// moving, because `playerctl --follow` deliberately only speaks when the
/// metadata changes - position is not metadata.
const DEFAULT_MEDIA_EVERY_MS: u64 = 700;
/// How often the list of players is re-read. Slower than the track: listing them
/// forks `playerctl` a second time, and the list only changes when a player comes
/// or goes.
const DEFAULT_PLAYERS_EVERY_MS: u64 = 3000;
/// How many notifications the tile lists. The hub remembers a few more than it
/// draws (see `notify`), so dismissing one promotes the next.
const DEFAULT_NOTIFICATION_ROWS: i32 = 3;

/// What the config file resolved to.
struct Settings {
    /// How the bar sits, in the pixels this panel is placed in.
    bar: BarGeometry,
    /// The width of the pill's hot zone, for an `open` that arrived without a
    /// rectangle: a strip in the middle of the bar.
    hot_width: f64,
    left_width: i32,
    right_width: i32,
    delays: Delays,
    /// How often to ask the compositor where the pointer is, while it matters.
    poll: Duration,
    tick: Duration,
    media_every: u64,
    /// How often the media tile re-reads the list of players it can follow.
    players_every: u64,
    /// How many notifications the tile lists. Zero is a legitimate answer: the
    /// count and the buttons then stand on their own.
    notification_rows: usize,
    notifications_command: String,
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
            media_every: config
                .millis("media_every_ms", DEFAULT_MEDIA_EVERY_MS)
                .as_millis() as u64,
            players_every: config
                .millis("players_every_ms", DEFAULT_PLAYERS_EVERY_MS)
                .as_millis() as u64,
            // Five is where the tile stops being a glance and starts being a
            // list; the hub does not remember more than a few anyway.
            notification_rows: config
                .i32("notification_rows", DEFAULT_NOTIFICATION_ROWS)
                .clamp(0, 5) as usize,
            notifications_command: config.string("notifications_command", "swaync-client"),
        }
    }

    /// The panel's width: the two columns, their gap, the padding this panel's
    /// own stylesheet adds, and the padding the island's body puts around the
    /// content. It is what `status` compares the measured card against - the
    /// shape itself is measured, not planned.
    /// The card's width for a machine in `tablet` state: the two columns and the
    /// padding, plus the room the touch-sized header and transport row need (see
    /// [`TOUCH_WIDTH_EXTRA`]).
    fn width(&self, tablet: bool) -> i32 {
        self.left_width
            + self.right_width
            + 10
            + 2 * CARD_PAD_X
            + 4
            + if tablet { TOUCH_WIDTH_EXTRA } else { 0 }
    }

    /// The bar's geometry for a machine in `tablet` state.
    ///
    /// `island.conf`'s `bar_height` is the laptop height (it has to match
    /// `bar.conf`); the bar grows while the machine is a tablet, so the panel's
    /// geometry has to grow with it. Everything the panel measures - the pill's
    /// hot zone, where its own card hangs, and the hole the tap catcher leaves -
    /// goes through this, so the three can never disagree about where the bar is.
    fn bar(&self, tablet: bool) -> BarGeometry {
        BarGeometry {
            height: self.bar.height
                + if tablet {
                    f64::from(TOUCH_HEIGHT_EXTRA)
                } else {
                    0.0
                },
            ..self.bar
        }
    }
}

/// The panel's state: what the hover machine says, and what is on the card.
struct Panel {
    machine: Machine,
    node: Rc<IslandView>,
    /// The shape the panel unfolds from, and the card it becomes (see
    /// `core::island`). The card's widgets live inside it; the surface is a
    /// transparent canvas the size of the screen.
    island: Rc<Island>,
    /// The connector the bar is on, when the bar said so with `open`. The bar is
    /// pinned to the output it started on, which is not necessarily the focused
    /// one - so the player's hot zone has to be measured on *that* screen, and
    /// the surface drawn there.
    monitor: RefCell<Option<String>>,
    /// The player pill's rectangle inside the bar's card, as the bar measured it
    /// with `open`. `None` for a hand-typed `open`, which then falls back to the
    /// derived centre strip - the pill is as wide as its track, so guessing it
    /// is the one thing this panel cannot do itself.
    pill: RefCell<Option<hover::Rect>>,
    notifications: RefCell<Notifications>,
    /// What the notifications *say*, read off the bus - the other half of the hub
    /// (see `notify`). The state above arrives without it; this fills the rows in.
    history: RefCell<notify::History>,
    /// The tablet state the panel's surface and hover geometry were built for.
    /// The bar grows when the machine folds, so this is what `sample` and the
    /// tap catcher's hole measure against - and what the `on_tablet_change`
    /// handler compares to, so a fold applies the new placement exactly once.
    tablet: Cell<bool>,
    /// Whether the tap catcher is up around the card. The catcher itself lives in
    /// the shell (`Osd::tap_catcher`); this is what the close paths and the
    /// heartbeat use to decide whether there is one to take down or re-fit.
    catcher: Cell<bool>,
}

impl Panel {
    /// The notification tile, drawn from both halves of the hub at once, so the
    /// two can never be out of step.
    fn render_notifications(&self) {
        let notifications = *self.notifications.borrow();
        let history = self.history.borrow();
        self.node.render_notifications(&notifications, &history);
    }
}

fn main() -> glib::ExitCode {
    let config = Config::load(ELEMENT);
    let settings = Rc::new(Settings::load(&config));
    // The machine's state when the panel starts decides the two numbers that
    // cannot change through CSS: where the card hangs and how wide it is.
    let tablet = touch::tablet();

    let opts = Opts {
        app_id: APP_ID.to_string(),
        css: css::stylesheet(include_str!("island.css")),
        namespace: NAMESPACE.to_string(),
        // A transparent canvas the size of the screen: the panel does not hang
        // under the top edge from here, it **unfolds out of the player pill** -
        // the shape's own rectangle comes from `core::island`, the bar geometry
        // and the pill the bar sends (see `Placement::Canvas`).
        placement: Placement::Canvas,
        ..Opts::default()
    };

    let panel: Rc<OnceCell<Rc<RefCell<Panel>>>> = Rc::new(OnceCell::new());

    let build = {
        let panel = panel.clone();
        let settings = settings.clone();
        Box::new(move |osd: &Rc<Osd>| {
            let node = IslandView::new(&settings);
            node.render_clock();
            // The card's widgets live inside the island for the whole session:
            // the shape is what the surface draws, and every repaint below goes on
            // changing these same widgets.
            let island = Island::new(&node.root);
            // The card's floor: what `Opts::width` used to be. A tablet's
            // touch-sized rows need more room than the same rows on a laptop, and
            // a card measured from its content would only be as wide as the
            // content asks.
            island.set_min_width(settings.width(tablet) - 2 * CARD_PAD_X);
            // A fresh process has no panel up, whatever the flag says. Without
            // this, an island that was killed while the panel was open would
            // leave the bar's player pill lit up for the rest of the session - the
            // flag is written on show/hide, and a `kill` never reaches either.
            state::write(&state::island_panel(), false);
            let panel_state = Rc::new(RefCell::new(Panel {
                machine: Machine::default(),
                node: node.clone(),
                island: island.clone(),
                monitor: RefCell::new(None),
                pill: RefCell::new(None),
                notifications: RefCell::new(Notifications::default()),
                history: RefCell::new(notify::History::new()),
                tablet: Cell::new(tablet),
                catcher: Cell::new(false),
            }));
            let _ = panel.set(panel_state.clone());

            // The fold: while the machine is a tablet the bar is taller (its
            // pills are finger-sized), and this panel hangs below it, so its own
            // placement moves with it. `begin_touch` hands the current answer
            // over once at start-up - which is what `tablet` above already used,
            // so the first call is a no-op and a bar that comes up folded needs
            // no second rebuild.
            {
                let panel = panel_state.clone();
                let settings = settings.clone();
                let shell = Rc::downgrade(osd);
                osd.on_tablet_change(move |tablet| {
                    if panel.borrow().tablet.replace(tablet) == tablet {
                        return;
                    }
                    let Some(osd) = shell.upgrade() else {
                        return;
                    };
                    // The bar just got taller (or shorter), so the shape it
                    // unfolds from moved: re-place a panel that is up on the new
                    // geometry - there is no surface to rebuild any more, the
                    // canvas is the screen either way.
                    panel
                        .borrow()
                        .island
                        .set_min_width(settings.width(tablet) - 2 * CARD_PAD_X);
                    if panel.borrow().machine.is_open() {
                        reshape(&panel, &settings);
                    }
                    // The card just moved; a catcher around it has to leave its
                    // hole where the card now is, or the dim would cover part of
                    // it (or fail to cover the part outside).
                    if panel.borrow().catcher.get() {
                        catch_taps(&panel, &osd, &settings);
                    }
                });
            }

            // The notification feed runs from start-up: a count that only started
            // being followed when the panel opened would always arrive empty.
            let follower = notify::follow({
                let panel_state = panel_state.clone();
                move |notifications| {
                    let panel = panel_state.borrow();
                    *panel.notifications.borrow_mut() = notifications;
                    panel.render_notifications();
                    // The chip is painted from the tile's own snapshot - the one
                    // the media poll left behind - so the two cannot disagree.
                    let playback = panel.node.playback();
                    panel.node.render_chips(playback.as_ref(), &notifications);
                }
            });

            // What the notifications say is a feed of its own, for the same
            // reason and from start-up as well: only what the bus carries while
            // this process is watching can be listed, so a late start would show
            // an empty tile however many notifications were waiting.
            let history_follower = notify::follow_history({
                let panel_state = panel_state.clone();
                move |line| {
                    let panel = panel_state.borrow();
                    // Only redraw when the picture changed: the reply to a `Notify`
                    // call fills in an id and nothing else.
                    if panel.history.borrow_mut().apply(line) {
                        panel.render_notifications();
                    }
                }
            });

            // One heartbeat for the slow things - the clock, and what is playing
            // while the panel is up.
            let ticks = Rc::new(Cell::new(0u64));
            {
                let panel = panel_state.clone();
                let settings = settings.clone();
                let osd = osd.clone();
                glib::timeout_add_local(settings.tick, move || {
                    let count = ticks.get() + 1;
                    ticks.set(count);
                    if panel.borrow().machine.is_open() {
                        panel.borrow().node.render_clock();
                        // A row that says "5 min" has to become "6 min" without
                        // anything having happened, so the ages are refreshed with
                        // the clock rather than with the list.
                        panel.borrow().node.render_ages();
                        if every(count, settings.media_every, settings.tick) {
                            // The list of players rides the same heartbeat, on a
                            // slower interval of its own.
                            poll_media(&panel, every(count, settings.players_every, settings.tick));
                        }
                        // A notification or a track arriving while the panel is up
                        // can make the card taller: the island is the screen, so it
                        // has to be told - the surface grew with its content by
                        // itself before this shape existed.
                        reshape(&panel, &settings);
                        // The tap catcher's hole is the card's rectangle, so it
                        // follows the same growth (see `catch_taps`).
                        if panel.borrow().catcher.get() {
                            catch_taps(&panel, &osd, &settings);
                        }
                    }
                    glib::ControlFlow::Continue
                });
            }

            // The pointer, on its own faster clock - and only while there is
            // something to watch. An idle island asks the compositor for nothing
            // at all, where the script this replaces polled for the whole session.
            {
                let panel = panel_state.clone();
                let settings = settings.clone();
                let osd = osd.clone();
                glib::timeout_add_local(settings.poll, move || {
                    if panel.borrow().machine.is_watching() {
                        let action = sample(&panel, &settings);
                        act(&panel, &osd, &settings, action);
                    }
                    glib::ControlFlow::Continue
                });
            }

            osd.on_shutdown(move || {
                follower.stop();
                history_follower.stop();
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
                    // What the bar's player pill runs when the pointer arrives.
                    // It only *arms* the panel: the dwell, and the decision to
                    // open, are the panel's own (see `hover`).
                    "open" => {
                        aim(panel, osd, &args[1..]);
                        panel.borrow_mut().machine.arm(Instant::now());
                        Ok(String::new())
                    }
                    // A finger tapped the bar's player. It is the same request as
                    // `open` - the same connector, the same pill - from a gesture
                    // that cannot hover: the panel opens at once, stays up
                    // (pinned), and a tap catcher around its card is the way back
                    // out. A second tap on the pill closes it, because the panel
                    // will not leave on its own any more.
                    "tap" => {
                        aim(panel, osd, &args[1..]);
                        if panel.borrow().machine.is_open() {
                            let action = panel.borrow_mut().machine.dismiss();
                            act(panel, osd, &settings, action);
                        } else {
                            panel.borrow_mut().machine.pin();
                            act(panel, osd, &settings, Action::Show);
                            // After the show: the card's size is only known once
                            // it has been laid out (see `card_size`), and a hole
                            // in the dim is worth nothing if it is not over the
                            // card.
                            catch_taps(panel, osd, &settings);
                        }
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
                    "status" => Ok(status(&panel.borrow(), &settings)),
                    other => Err(format!(
                        "unknown command `{other}` (open | tap | close | toggle | show | status)"
                    )),
                }
            },
        )
    };

    run(opts, build, handle)
}

/// Remember which bar this panel belongs to, and where its pill is.
///
/// The bar sends both with `open`/`tap`: the connector (the panel is drawn on
/// that screen) and the pill's rectangle inside the bar's card (that is the hot
/// zone, because the player pill is as wide as its track and cannot be derived
/// from the bar's edges). A hand-typed `open` may send neither, and then the
/// panel keeps its own fallbacks - the focused monitor and the derived strip.
fn aim(panel: &Rc<RefCell<Panel>>, osd: &Rc<Osd>, args: &[String]) {
    let (connector, rect) = hover::parse_open(args);
    let state = panel.borrow();
    if let Some(connector) = connector {
        osd.pin_output(Some(&connector));
        *state.monitor.borrow_mut() = Some(connector);
    }
    *state.pill.borrow_mut() = rect;
}

/// Show or hide the surface, and say so in the state flag while doing it.
///
/// The flag is what the bar's player pill reads to light up. It is written here
/// and nowhere else, so the two programs cannot disagree about whether the panel
/// is up.
fn act(panel: &Rc<RefCell<Panel>>, osd: &Rc<Osd>, settings: &Settings, action: Action) {
    match action {
        Action::None => {}
        Action::Show => {
            // Read what is playing *before* the surface appears, so the panel
            // opens with the track on it rather than with "Nothing playing" for
            // as long as the round trip takes.
            poll_media(panel, true);
            // The shape starts as the player pill the pointer came from, and the
            // surface comes up already holding it: a panel that appeared at its
            // full size for one frame and then grew would flash.
            if let Some(origin) = origin_of(&panel.borrow(), settings) {
                panel.borrow().island.unfold(&origin, settings.hot_width);
            }
            osd.show();
            state::write(&state::island_panel(), true);
        }
        Action::Hide => {
            // A card cannot be gone while the dim it opened still covers the
            // screen. `Osd::hide` would take the catcher away too as a backstop,
            // but the tap catcher is the element's, so the element says it here -
            // and every close path goes through this function.
            clear_catcher(panel, osd);
            // The flag goes out first so the bar starts retracting its pills
            // while the shape is still shrinking: they are one movement.
            state::write(&state::island_panel(), false);
            let osd = osd.clone();
            panel.borrow().island.fold(move || osd.hide());
        }
    }
}

/// Where this panel unfolds from, resolved the way every zone here is: against
/// the monitor the *bar* is on, not the focused one.
///
/// A hand-typed `open` may name neither, and then the focused output and the
/// derived centre strip are the fallbacks - the player pill is as wide as its
/// track, so how wide it is cannot be guessed.
fn origin_of(panel: &Panel, settings: &Settings) -> Option<Origin> {
    let monitor = panel
        .monitor
        .borrow()
        .as_deref()
        .and_then(hover::monitor_of)
        .or_else(hover::focused_monitor)?;
    Some(Origin {
        side: Side::Centre,
        bar: settings.bar(panel.tablet.get()),
        monitor,
        pill: *panel.pill.borrow(),
    })
}

/// Put the shape where the bar's geometry now says it belongs, without animating
/// it: what a fold (or an unfolding) needs, because the bar just grew taller and
/// the shape moved with it.
fn reshape(panel: &Rc<RefCell<Panel>>, settings: &Settings) {
    let state = panel.borrow();
    if let Some(origin) = origin_of(&state, settings) {
        state.island.refresh(&origin);
    }
}

/// Put the tap catcher around the panel's card (see `Osd::tap_catcher`).
///
/// The catcher is what a *finger*-opened panel gets instead of a hover: a dimmed
/// full-screen surface with the card's rectangle left open, so a tap anywhere
/// else is "close". It is armed from the `tap` verb, re-fitted by the heartbeat
/// (the card can grow when a notification or a track arrives while it is up) and
/// by a fold (the card moves down), and taken down by [`clear_catcher`] on every
/// close path.
fn catch_taps(panel: &Rc<RefCell<Panel>>, osd: &Rc<Osd>, settings: &Rc<Settings>) {
    let Some(origin) = origin_of(&panel.borrow(), settings) else {
        return;
    };
    let hole = panel.borrow().island.hole(&origin);
    let shell = Rc::downgrade(osd);
    let close_panel = panel.clone();
    let settings = settings.clone();
    osd.tap_catcher(Some(hole), move || {
        let Some(osd) = shell.upgrade() else {
            return;
        };
        let action = close_panel.borrow_mut().machine.dismiss();
        act(&close_panel, &osd, &settings, action);
    });
    panel.borrow().catcher.set(true);
}

/// Take the tap catcher down, if there is one. Nothing happens when there is
/// not - a panel opened by a pointer has no catcher, and closing it must not
/// disturb a card that is still up.
fn clear_catcher(panel: &Rc<RefCell<Panel>>, osd: &Rc<Osd>) {
    if panel.borrow().catcher.replace(false) {
        // The callback is required by the shell even when the catcher is only
        // being taken away; nothing can land on a hidden catcher to run it.
        osd.tap_catcher(None, || {});
    }
}

/// One pointer sample: read the cursor, work out whether it is over the pill or
/// over the panel, and let the machine decide what to do about it.
fn sample(panel: &Rc<RefCell<Panel>>, settings: &Rc<Settings>) -> Action {
    let Some(origin) = origin_of(&panel.borrow(), settings) else {
        return Action::None;
    };
    let Some(cursor) = hover::cursor() else {
        // The compositor did not answer. Leaving the state alone is the right
        // reaction: a panel that closed because a socket hiccupped would be worse
        // than one that stayed up a moment too long.
        return Action::None;
    };
    // The pill's hot zone is the rectangle the bar measured and sent with
    // `open`: the player is as wide as its track, so it cannot be derived. A
    // hand-typed `open` falls back to a strip in the middle of the bar, where
    // the player is. The island is the whole shape - the neck between the bar and
    // the card is material now, so a pointer on it belongs to the panel.
    let (pill, island) = panel.borrow().island.zones(&origin, settings.hot_width);
    panel.borrow_mut().machine.sample(
        pill.contains(cursor, 0.0),
        island.contains(cursor, 8.0),
        Instant::now(),
        settings.delays,
    )
}

/// Read what is playing and put it on the tile - and the header chip, which is
/// drawn from the tile's own snapshot so the two cannot disagree.
fn poll_media(panel: &Rc<RefCell<Panel>>, read_players: bool) {
    let panel = panel.borrow();
    panel.node.refresh_media(read_players);
    let playback = panel.node.playback();
    let notifications = *panel.notifications.borrow();
    panel.node.render_chips(playback.as_ref(), &notifications);
}

/// Whether this tick is the one for an interval (the bar's `every`, which this
/// panel needs for the same reason: the interval is in milliseconds and the
/// heartbeat may not be).
fn every(count: u64, interval_ms: u64, tick: Duration) -> bool {
    let tick_ms = tick.as_millis().max(1) as u64;
    count.is_multiple_of((interval_ms / tick_ms).max(1))
}

/// What `status` prints: what the panel would show, whether or not it is up.
fn status(panel: &Panel, settings: &Settings) -> String {
    // Read now rather than reporting the last poll: this verb's job is to say
    // what the panel *would* show, and the panel may have been down for hours.
    // It is the same read the heartbeat makes while the panel is up - and reading
    // the tile is also what leaves it up to date for the next time it opens.
    panel.node.refresh_media(true);
    let playback = panel.node.playback();
    // What the Media row's buttons are set to, and which player they are aimed at.
    let controls = match &playback {
        Some(playback) => format!(
            "shuffle {} · repeat {} · volume {}",
            if playback.shuffle { "on" } else { "off" },
            playback.repeat.verb().to_lowercase(),
            match playback.volume {
                Some(level) => format!("{}%", (level * 100.0).round() as i32),
                None => "not reported".to_string(),
            }
        ),
        None => "nothing to control".to_string(),
    };
    let players = panel.node.players_list();
    let following = match panel.node.target().name() {
        Some(name) => format!("following {name}"),
        None => "following the active player".to_string(),
    };
    let players_line = if players.is_empty() {
        "none running".to_string()
    } else {
        format!("{} · {following}", players.join(", "))
    };
    let notifications = *panel.notifications.borrow();
    let now = glib::DateTime::now_local().ok();
    let format = |pattern: &str| {
        now.as_ref()
            .and_then(|now| now.format(pattern).ok())
            .map(|text| text.to_string())
            .unwrap_or_default()
    };
    // The rows the tile would draw. Their ages are worked out here rather than
    // read back from the view, which only keeps them current while the panel is
    // up - this verb has to answer with the card down as well.
    let history = panel.history.borrow();
    let latest = if history.items().is_empty() {
        "nothing seen yet".to_string()
    } else {
        let now = Instant::now();
        history
            .items()
            .iter()
            .map(|item| format!("{} · {} · {}", item.app, item.title, item.age(now)))
            .collect::<Vec<_>>()
            .join("\n           ")
    };
    [
        match origin_of(panel, settings) {
            Some(origin) => {
                let (_, island) = panel.island.zones(&origin, settings.hot_width);
                format!(
                    "panel      {:.0},{:.0} {:.0}x{:.0} - one shape, from the bar's top edge",
                    island.x, island.y, island.width, island.height
                )
            }
            None => "panel      no shape - the compositor did not answer hyprctl".to_string(),
        },
        format!(
            "state      {}",
            if panel.machine.is_open() {
                "open"
            } else {
                "closed"
            }
        ),
        format!(
            "clock      {} · {}",
            format("%H:%M"),
            format("%A, %d %B %Y")
        ),
        format!(
            "media      {}",
            playback
                .as_ref()
                .map(|playback| {
                    let progress = if playback.length > 0 {
                        format!(" · {} / {}", playback.elapsed(), playback.total())
                    } else {
                        String::new()
                    };
                    format!(
                        "{}{}{progress}",
                        playback.track.summary(),
                        if playback.playing { "" } else { " (paused)" }
                    )
                })
                .unwrap_or_else(|| "nothing playing".to_string())
        ),
        format!("controls   {controls}"),
        format!("players    {players_line}"),
        format!(
            "notified   {} waiting, dnd {}, inhibited {}{}",
            notifications.count,
            if notifications.dnd { "on" } else { "off" },
            if notifications.inhibited { "yes" } else { "no" },
            if notifications.centre_open {
                ", control centre open"
            } else {
                ""
            }
        ),
        format!("latest     {latest}"),
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_interval_shorter_than_the_tick_is_every_tick() {
        let tick = Duration::from_millis(1000);
        // 700 ms cannot be honoured by a 1 s heartbeat, so it happens every tick
        // rather than never.
        assert!(every(1, 700, tick));
        // A whole number of ticks is counted properly.
        assert!(!every(1, 2000, tick));
        assert!(every(2, 2000, tick));
        assert!(every(7, 7000, tick));
    }

    /// The bar this panel hangs from, as `island.conf` and the bar's own defaults
    /// describe it on the laptop side of the fold.
    fn bar(height: f64) -> BarGeometry {
        BarGeometry {
            height,
            margin_top: 8.0,
            margin_x: 12.0,
            gap: 6.0,
            shadow_pad: 18.0,
        }
    }

    /// The monitor this desktop has: 1368x912 logical, *away from the layout
    /// origin* - which is the point, because the hole has to be monitor-relative.
    fn monitor() -> hover::Rect {
        hover::Rect {
            x: 1903.0,
            y: 1440.0,
            width: 1368.0,
            height: 912.0,
        }
    }

    /// The origin this panel would unfold from on that bar.
    fn place(height: f64) -> Origin {
        Origin {
            side: Side::Centre,
            bar: bar(height),
            monitor: monitor(),
            pill: None,
        }
    }

    #[test]
    fn the_tap_catcher_leaves_the_card_open_in_monitor_coordinates() {
        let hole = place(52.0).hole((476.0, 300.0));
        // The card's own size is what the element measured.
        assert_eq!(hole.width, 476);
        assert_eq!(hole.height, 300);
        // Centred on the monitor, measured from *its* top-left corner - a hole
        // in the layout's coordinates would be dim over the whole card.
        assert_eq!(hole.x, (1368 - 476) / 2);
        // And its top edge is past the tablet bar, the gap and the shadow ring.
        assert_eq!(hole.y, 8 + 52 + 6 + 18);
    }

    #[test]
    fn a_fold_moves_the_hole_down_with_the_bar() {
        // The same panel at the laptop's 40px bar and at the tablet's taller one:
        // the panel hangs lower, and so does the hole that must stay over it.
        let laptop = place(40.0).hole((476.0, 300.0));
        let folded = place(40.0 + f64::from(TOUCH_HEIGHT_EXTRA)).hole((476.0, 300.0));
        assert_eq!(folded.y - laptop.y, TOUCH_HEIGHT_EXTRA);
        assert_eq!(folded.height, laptop.height);
    }
}

//! `hypr-osd-island` - the panel that unfolds from the bar's clock.
//!
//! The bar's clock pill is the handle; this is what comes out of it. Hovering the
//! clock asks this program to open (`hypr-osd-island open`), and from then on the
//! panel looks after itself: it samples the pointer until either the dwell is up
//! (open) or the pointer has been away long enough (close), because *leaving*
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
//!   open [connector]     start watching: the panel appears if the pointer stays.
//!                        The connector is the monitor the bar is on, which is
//!                        what the hot zone is measured against (the bar is
//!                        pinned to the output it started on, and that is not
//!                        always the focused one).
//!   close                take it away now
//!   toggle               close it if it is up, otherwise show it
//!   show                 put it up and keep it there (no pointer tracking)
//!   status               print what the panel would show, up or not
//!                        (including the notifications the bus carried)
//!   (no verb)            start the daemon and wait for the bar
//! ```

mod calendar;
mod notify;
mod view;

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::hover::{self, Action, BarGeometry, Delays, Machine, Side};
use hypr_osd_core::{
    css, run, state, Config, Content, Opts, Osd, Placement, CARD_PAD_X, CARD_PAD_Y, SHADOW_PAD,
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
// has to know where the bar is without being told: it derives the pill's position
// from the monitor's centre, and hangs itself below the bar's bottom edge.
const DEFAULT_BAR_HEIGHT: i32 = 40;
const DEFAULT_BAR_MARGIN_TOP: i32 = 8;
const DEFAULT_BAR_MARGIN_X: i32 = 12;
const DEFAULT_GAP: i32 = 6;
const DEFAULT_HOT_WIDTH: i32 = 280;
const DEFAULT_LEFT_WIDTH: i32 = 252;
const DEFAULT_RIGHT_WIDTH: i32 = 178;
const DEFAULT_OPEN_DELAY_MS: u64 = 120;
const DEFAULT_CLOSE_DELAY_MS: u64 = 300;
const DEFAULT_POLL_MS: u64 = 50;
const DEFAULT_TICK_MS: u64 = 1000;
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
    /// The width of the pill's hot zone, centred on the bar.
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
    /// own stylesheet adds, and the padding the shell puts around the content.
    /// Getting this right is what makes the derived hover zone line up with the
    /// panel that is actually on screen.
    fn width(&self) -> i32 {
        self.left_width + self.right_width + 10 + 2 * CARD_PAD_X + 4
    }

    /// How far below the top edge of the screen the panel hangs: past the bar, its
    /// gap, and the transparent frame the shell keeps around a card so its shadow
    /// is not clipped.
    fn margin_top(&self) -> i32 {
        self.bar.margin_top as i32 + self.bar.height as i32 + self.bar.gap as i32 + SHADOW_PAD
    }
}

/// The panel's state: what the hover machine says, and what is on the card.
struct Panel {
    machine: Machine,
    node: Rc<IslandView>,
    /// The connector the bar is on, when the bar said so with `open`. The bar is
    /// pinned to the output it started on, which is not necessarily the focused
    /// one - so the clock's hot zone (the middle of the bar) has to be measured
    /// on *that* screen, and the surface drawn there.
    monitor: RefCell<Option<String>>,
    notifications: RefCell<Notifications>,
    /// What the notifications *say*, read off the bus - the other half of the hub
    /// (see `notify`). The state above arrives without it; this fills the rows in.
    history: RefCell<notify::History>,
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

    let opts = Opts {
        app_id: APP_ID.to_string(),
        css: css::stylesheet(include_str!("island.css")),
        namespace: NAMESPACE.to_string(),
        // A content-sized card hanging under the top edge: the shell knows how to
        // place one (see `Placement::TopCard`), which is what keeps the panel
        // centred on the bar without this program measuring anything.
        placement: Placement::TopCard {
            margin_top: settings.margin_top(),
        },
        width: settings.width(),
        ..Opts::default()
    };

    let panel: Rc<OnceCell<Rc<RefCell<Panel>>>> = Rc::new(OnceCell::new());

    let build = {
        let panel = panel.clone();
        let settings = settings.clone();
        Box::new(move |osd: &Rc<Osd>| {
            let node = IslandView::new(&settings);
            node.render_clock();
            // A fresh process has no panel up, whatever the flag says. Without
            // this, an island that was killed while the panel was open would
            // leave the bar's clock lit up for the rest of the session - the flag
            // is written on show/hide, and a `kill` never reaches either.
            // A fresh process has no panel up, whatever the flag says. Without
            // this, an island that was killed while the panel was open would
            // leave the bar's clock lit up for the rest of the session - the flag
            // is written on show/hide, and a `kill` never reaches either.
            state::write(&state::island_panel(), false);
            let panel_state = Rc::new(RefCell::new(Panel {
                machine: Machine::default(),
                node: node.clone(),
                monitor: RefCell::new(None),
                notifications: RefCell::new(Notifications::default()),
                history: RefCell::new(notify::History::new()),
            }));
            let _ = panel.set(panel_state.clone());

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
                        act(&panel, &osd, action);
                    }
                    glib::ControlFlow::Continue
                });
            }

            osd.on_shutdown(move || {
                follower.stop();
                history_follower.stop();
            });

            Content::Single(node.root.clone().upcast::<gtk::Widget>())
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
                    // What the bar's clock runs when the pointer arrives. It only
                    // *arms* the panel: the dwell, and the decision to open, are
                    // the panel's own (see `hover`).
                    "open" => {
                        // The bar may name the connector it is on; without one the
                        // panel falls back to the focused output, which is right
                        // on a single-monitor desktop.
                        let connector = args.get(1).filter(|arg| !arg.is_empty()).cloned();
                        if let Some(connector) = connector {
                            osd.pin_output(Some(&connector));
                            *panel.borrow().monitor.borrow_mut() = Some(connector);
                        }
                        panel.borrow_mut().machine.arm(Instant::now());
                        Ok(String::new())
                    }
                    "close" => {
                        let action = panel.borrow_mut().machine.dismiss();
                        act(panel, osd, action);
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
                        act(panel, osd, action);
                        Ok(String::new())
                    }
                    "show" => {
                        panel.borrow_mut().machine.pin();
                        act(panel, osd, Action::Show);
                        Ok(String::new())
                    }
                    "status" => Ok(status(&panel.borrow(), &settings)),
                    other => Err(format!(
                        "unknown command `{other}` (open | close | toggle | show | status)"
                    )),
                }
            },
        )
    };

    run(opts, build, handle)
}

/// Show or hide the surface, and say so in the state flag while doing it.
///
/// The flag is what the bar's clock pill reads to light up. It is written here
/// and nowhere else, so the two programs cannot disagree about whether the panel
/// is up.
fn act(panel: &Rc<RefCell<Panel>>, osd: &Rc<Osd>, action: Action) {
    match action {
        Action::None => {}
        Action::Show => {
            // Read what is playing *before* the surface appears, so the panel
            // opens with the track on it rather than with "Nothing playing" for
            // as long as the round trip takes.
            poll_media(panel, true);
            osd.show();
            state::write(&state::island_panel(), true);
        }
        Action::Hide => {
            osd.hide();
            state::write(&state::island_panel(), false);
        }
    }
}

/// One pointer sample: read the cursor, work out whether it is over the pill or
/// over the panel, and let the machine decide what to do about it.
fn sample(panel: &Rc<RefCell<Panel>>, settings: &Rc<Settings>) -> Action {
    // The monitor the *bar* is on, not the focused one: the panel unfolds from
    // the bar, and the clock it unfolds from is the middle of that screen.
    let monitor = panel
        .borrow()
        .monitor
        .borrow()
        .as_deref()
        .and_then(hover::monitor_of)
        .or_else(hover::focused_monitor);
    let Some(monitor) = monitor else {
        return Action::None;
    };
    let Some(cursor) = hover::cursor() else {
        // The compositor did not answer. Leaving the state alone is the right
        // reaction: a panel that closed because a socket hiccupped would be worse
        // than one that stayed up a moment too long.
        return Action::None;
    };
    let (width, height) = {
        let panel = panel.borrow();
        let root = panel.node.root.clone();
        // The card is the content *plus* the padding the shell puts around it,
        // which is what is really on screen.
        (
            f64::from(root.width() + 2 * CARD_PAD_X),
            f64::from(root.height() + 2 * CARD_PAD_Y),
        )
    };
    // The clock is the bar's centre child, so the pill's hot zone can be derived
    // from the monitor's centre without the bar having to say anything.
    let pill = hover::pill_rect(monitor, settings.bar, settings.hot_width, Side::Centre);
    let panel_rect = hover::panel_rect(monitor, settings.bar, Side::Centre, (width, height));
    panel.borrow_mut().machine.sample(
        pill.contains(cursor, 0.0),
        panel_rect.contains(cursor, 8.0),
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
        format!(
            "panel      {}px wide, {}px below the top",
            settings.width(),
            settings.margin_top()
        ),
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
}

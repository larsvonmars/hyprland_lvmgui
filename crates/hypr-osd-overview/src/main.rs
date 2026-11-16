//! `hypr-osd-overview` - element #5: the workspace overview.
//!
//! One tap of SUPER+SHIFT+TAB opens a picture of the whole desktop: every
//! workspace as a chip along the top, and every window as a tile - in the
//! window's own shape, scaled until all of them fit on the screen at once. It is
//! the macOS Mission Control gesture, for Hyprland's workspaces: you leave the
//! desktop you have, look at it, and click the window you actually wanted.
//!
//! The tiles are *captures*, not icons. One `grim -T` per window asks the
//! compositor for that window's own buffer, which is what makes the thing
//! possible at all: the capture is complete even when the window is behind
//! another one, and it exists even when the window is on a workspace that is not
//! on screen - which is where most of the windows in a workspace list are. (An
//! earlier plan was to grab the screen once and crop each window out of it,
//! which cannot show a covered window and has nothing to show for a hidden
//! workspace.) Where no picture can be had, the tile falls back to the
//! application's icon and says so with its frame.
//!
//! This is one of the three elements that take the keyboard. The switcher does it
//! to hear the Alt release that ends a switch; this one does it because a card you
//! opened on purpose owns Escape, the arrows and its digits while it is up, and
//! there is no way to hear those keys and also let them through to whatever is
//! behind the card. The launcher is the third, and it has to be typed into. All
//! three are cards you asked for; every other element still never takes a
//! keystroke.
//!
//! Verbs (`hypr-osd-overview <verb>`):
//!
//! ```text
//!   toggle               open it, or close it again (what SUPER+SHIFT+TAB runs)
//!   show                 open it
//!   close                close it without switching anything
//!   recapture | refresh  take fresh pictures of every window (what R does)
//!   move <window> <ws>   what dropping a tile on a workspace chip does - the
//!                        window is named by its address or by its position
//!   status               list the workspaces and windows, no card
//!   (no verb)            start the daemon and wait for the first tap
//! ```
//!
//! # Dragging a window onto a workspace
//!
//! A tile can be dragged onto a workspace chip - or onto another tile, which
//! means "wherever that one lives" - and the window is moved there without the
//! desktop following it: the card is where you arrange the workspaces, not where
//! you go. The card then *corrects its own picture* rather than taking a new one:
//! the only thing that changed is which workspace one window belongs to, so the
//! thumbnails it already holds are still the truth, and `move` exists because a
//! click cannot be scripted on a layer surface - it goes through the same
//! [`Overview::assign`] a drop does.
//!
//! # By finger
//!
//! The card is opened by `toggle`/`show` - the system's touch gesture calls one
//! of them - and it needs no new entry point: a tap on a tile is a click and
//! switches, a tap on a workspace chip switches workspace, and a tap on the empty
//! space around them closes the card. A touchscreen has no R key, so the strip
//! grows a **Recapture** chip ([`view::OverviewView::on_recapture_click`]) for the
//! one action that only a key could reach; `recapture` is the same thing from a
//! script, which is also how that chip is tested (a click cannot be scripted).
//!
//! The layout follows the machine too: the strip is taller on a tablet
//! (`view::TOUCH_STRIP`), so the tile area the plan solves for is worked out
//! against the mode the machine is in right now, and a fold under an open card
//! re-opens it rather than leaving a strip taller than its own plan. And every
//! press or scroll on the card is activity
//! ([`view::OverviewView::on_activity`]), so the idle-close watchdog does not take
//! the desktop away while a finger is on it - pointer motion alone would never see
//! a touch until it lands, and the overview is a card you *look* at.

mod layout;
mod thumbs;
mod view;

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::monitors::{self, Output};
use hypr_osd_core::touch;
use hypr_osd_core::windows::{self, Window, Workspace};
use hypr_osd_core::{
    css, hyprctl, run, Config, Content, Keyboard, Opts, Osd, Placement, CARD_PAD_X, CARD_PAD_Y,
};

use layout::{Area, Limits, Plan};
use view::{Drop, OverviewView};

/// D-Bus application id - and therefore the single-instance key.
const APP_ID: &str = "com.schells2.osd.overview";
/// Layer-shell namespace; the collection shares the `hypr-osd` prefix so one
/// layer rule in `osd.lua` covers every element.
const NAMESPACE: &str = "hypr-osd";
/// Element name: `~/.config/hypr-osd/overview.conf`.
const ELEMENT: &str = "overview";

// Defaults; every one is overridable in the config file.
/// Smallest and largest tile height, and the space between tiles.
const DEFAULT_MIN_HEIGHT: i32 = 84;
const DEFAULT_MAX_HEIGHT: i32 = 300;
const DEFAULT_GAP: i32 = 18;
/// Icon size where a tile has no picture.
const DEFAULT_ICON_SIZE: i32 = 64;
/// How long the card may stand untouched before it closes itself. A card that
/// holds the keyboard must never be able to hold it forever - and it can end up
/// unreachable, because a session lock takes the keyboard away from every other
/// surface, this one included.
const DEFAULT_IDLE_CLOSE_MS: u64 = 60_000;

/// How often the idle watchdog looks at the clock. The card closes after
/// `idle_close_ms` of silence, so a second of granularity is invisible - and one
/// timer that reads a clock costs nothing next to the timer-per-pointer-event it
/// replaced.
const IDLE_TICK: Duration = Duration::from_secs(1);

/// What to assume when hyprctl cannot say how big the screen is. Everything else
/// this element draws also comes from hyprctl, so this is only reachable if
/// Hyprland answered `clients` but not `monitors` - and then a guess that is
/// slightly too small beats a card with no tiles on it.
const FALLBACK_OUTPUT: Output = Output {
    width: 1366,
    height: 768,
    scale: 1.0,
};

/// What the config file resolved to.
struct Settings {
    /// How large a tile may be. `layout.rs` solves for a size between the two.
    limits: Limits,
    /// Space between tiles, and between rows of them.
    gap: i32,
    /// Icon size in a tile that has no picture.
    icon_size: i32,
    /// Whether to capture the windows at all. Off, the card is a workspace
    /// switcher drawn with application icons: much cheaper, and what someone
    /// running this on a compositor without per-window capture would want.
    thumbnails: bool,
    /// Close the card after this long without a key or a mouse movement over it
    /// (`None` disables the safety net).
    idle_close: Option<std::time::Duration>,
}

impl Settings {
    fn load(config: &Config) -> Self {
        let tile_min = config.i32("tile_min_height", DEFAULT_MIN_HEIGHT).max(48);
        // Never below the minimum: `plan` has to have a range to search.
        let tile_max = config
            .i32("tile_max_height", DEFAULT_MAX_HEIGHT)
            .max(tile_min);
        let gap = config.i32("gap", DEFAULT_GAP).clamp(0, 64);
        let idle = config.millis("idle_close_ms", DEFAULT_IDLE_CLOSE_MS);
        Settings {
            limits: Limits {
                tile_min,
                tile_max,
                gap,
                title: view::TITLE,
            },
            gap,
            icon_size: config.i32("icon_size", DEFAULT_ICON_SIZE).max(16),
            thumbnails: config.bool("thumbnails", true),
            idle_close: if idle.is_zero() { None } else { Some(idle) },
        }
    }
}

/// One look at the desktop: everything the card draws, read in one go.
///
/// It is kept for as long as the card is up, because the card is a *picture*: a
/// window that appears while it is up does not appear on it (`R` takes a new
/// look), and a window that is dragged onto another workspace is corrected in the
/// picture rather than read back from the compositor.
#[derive(Clone)]
struct Look {
    /// Every window, in the order the card draws them (grouped by workspace).
    windows: Vec<Window>,
    /// The workspaces that exist, for the strip.
    workspaces: Vec<Workspace>,
    /// The workspace in focus - the chip that wears the accent.
    focused: String,
    /// The address of the window in focus.
    current: Option<String>,
    /// The screen the card is on: the tile arithmetic and the capture detail both
    /// hang off it.
    output: Output,
}

impl Look {
    /// Read the desktop as it is now. `None` when there is nothing to put on a
    /// card at all.
    fn gather() -> Option<Look> {
        let mut windows = windows::list();
        let workspaces = workspaces_in(&windows);
        if workspaces.is_empty() {
            return None;
        }
        // Which window you are in: `list` is most-recently-used first, so it is
        // the first one - read before the ordering below moves it.
        let current = windows.first().map(|window| window.address.clone());
        let focused = windows::focused_workspace().unwrap_or_default();
        layout::order(&mut windows, &focused);
        let output = monitors::focused_output().unwrap_or(FALLBACK_OUTPUT);
        Some(Look {
            windows,
            workspaces,
            focused,
            current,
            output,
        })
    }

    /// The tile the selection should start on: the window you are in, so that
    /// opening the card and pressing Enter does nothing at all - rather than
    /// jumping somewhere because you pressed the key twice.
    fn selection(&self) -> usize {
        self.current
            .as_ref()
            .and_then(|address| {
                self.windows
                    .iter()
                    .position(|window| &window.address == address)
            })
            .unwrap_or(0)
    }

    fn len(&self) -> usize {
        self.windows.len()
    }
}

/// The overview's state: what is on the card, and which tile is selected.
struct Overview {
    osd: Rc<Osd>,
    view: Rc<OverviewView>,
    settings: Rc<Settings>,
    /// The picture of the desktop the card is drawing, in drawing order. Read on
    /// the way out (to focus a window) and when a drop rearranges it.
    look: RefCell<Option<Look>>,
    /// The pictures the card already holds, by capture id, for the card that is
    /// currently up.
    ///
    /// A drop repaints the card without taking new pictures: nothing about a
    /// window's *contents* changed, and one `grim` per window is the most
    /// expensive thing this element does. A fresh look (`open`, `R`) starts from
    /// an empty map, because a photograph of the desktop is only a photograph.
    pictures: RefCell<HashMap<String, gdk::Texture>>,
    /// The windows, grouped into the rows they were drawn in. This is what makes
    /// "down" a row rather than a tile.
    rows: RefCell<Vec<Vec<usize>>>,
    selected: Cell<usize>,
    /// The idle-close watchdog (see [`Overview::arm_idle`]).
    idle: RefCell<Option<glib::SourceId>>,
    /// When the card last heard from anyone - a key, a press, a pointer moving
    /// over it.
    activity: Cell<Option<Instant>>,
}

impl Overview {
    fn new(osd: &Rc<Osd>, view: Rc<OverviewView>, settings: Rc<Settings>) -> Rc<Self> {
        Rc::new(Overview {
            osd: osd.clone(),
            view,
            settings,
            look: RefCell::new(None),
            pictures: RefCell::new(HashMap::new()),
            rows: RefCell::new(Vec::new()),
            selected: Cell::new(0),
            idle: RefCell::new(None),
            activity: Cell::new(None),
        })
    }

    /// Open the card, or close it if it is already up - what SUPER+SHIFT+TAB
    /// runs, because one tap should be able to undo itself.
    fn toggle(self: &Rc<Self>) {
        if self.osd.is_visible() {
            self.close();
        } else {
            self.open();
        }
    }

    /// Look at the desktop: take the window list as it is now, work out how the
    /// tiles fit, and draw them.
    ///
    /// Everything is decided here and now, and nothing about the desktop is
    /// watched: the card is a photograph, and a second look - `R`, or the key
    /// again - is how you take a new one.
    fn open(self: &Rc<Self>) {
        let Some(look) = Look::gather() else {
            eprintln!("hypr-osd-overview: no workspaces to show");
            return;
        };
        let selected = look.selection();
        self.draw(&look, selected, true);
    }

    /// Take the pictures again - and with them the window list, which may well
    /// have changed while the card was up.
    fn refresh(self: &Rc<Self>) {
        self.open();
    }

    /// Draw the card from `look`.
    ///
    /// `recapture` is the difference between *looking* at the desktop and
    /// *changing* it: a drop moves one window between workspaces, so the
    /// thumbnails the card already holds are still what those windows look like,
    /// and taking them again would cost one `grim` per window for a card that did
    /// not move on screen.
    fn draw(self: &Rc<Self>, look: &Look, selected: usize, recapture: bool) {
        // The strip's height is part of the area the tiles get, and it is a
        // different height on a tablet - so the plan is solved for the machine as
        // it is at this very moment.
        let strip = view::OverviewView::strip_height(touch::tablet());
        let plan = layout::plan(
            &look.windows,
            area(look.output, &self.settings, strip),
            self.settings.limits,
        );
        self.view.render(
            &plan,
            &look.windows,
            &look.workspaces,
            &look.focused,
            &self.settings,
        );
        // A repaint happens with the card already up, and re-attaching the same
        // widget to the same card would only cost a layout pass.
        if !self.osd.is_visible() {
            self.osd.set_content(&self.view.root);
            self.osd.show();
        }

        self.selected.set(selected);
        self.view.select(selected);
        if self.settings.thumbnails {
            self.capture(&look.windows, &plan, look.output.scale, recapture);
        }
        // Kept for the arrow keys: the rows the plan decided on are the rows
        // "up" and "down" walk, so the view and the keys cannot disagree about
        // where a row begins.
        *self.rows.borrow_mut() = plan.rows;
        *self.look.borrow_mut() = Some(look.clone());
        self.arm_idle();
    }

    /// Switch to the selected window and take the card down.
    fn commit(self: &Rc<Self>) {
        let address = self
            .look
            .borrow()
            .as_ref()
            .and_then(|look| look.windows.get(self.selected.get()))
            .map(|window| window.address.clone());
        self.close();
        if let Some(address) = address {
            if !hyprctl::focus_window(&address) {
                eprintln!("hypr-osd-overview: could not focus {address}");
            }
        }
    }

    /// Switch to a workspace and take the card down. What a chip and a digit key
    /// both do.
    fn switch_workspace(self: &Rc<Self>, name: &str) {
        self.close();
        if !hyprctl::focus_workspace(name) {
            eprintln!("hypr-osd-overview: could not switch to workspace {name}");
        }
    }

    /// A tile was dropped on something: put that window there.
    ///
    /// The compositor is asked once - a drop is the only thing this card *does* -
    /// and then the card corrects its own picture: the windows are exactly where
    /// they were, one of them changed workspace, and a new capture of all of them
    /// would answer a question that has not changed.
    ///
    /// The window does **not** take the desktop with it (`hyprctl::move_window`
    /// does not follow): the card is where workspaces are arranged, not where you
    /// go. It stays up so that several windows can be put away in one look.
    fn assign(self: &Rc<Self>, index: usize, drop: Drop) -> Result<String, String> {
        let Some(mut look) = self.look.borrow().clone() else {
            return Err("the card is not showing anything".to_string());
        };
        let Some(window) = look.windows.get(index).cloned() else {
            return Err(format!("there is no tile {index}"));
        };
        let target = match drop {
            Drop::Workspace(name) => name,
            // Dropped on a window: the workspace that one lives on is meant.
            Drop::Window(other) => match look.windows.get(other) {
                Some(other) => other.workspace.clone(),
                None => return Err(format!("there is no tile {other}")),
            },
        };
        if target == window.workspace {
            return Ok(format!(
                "{} is already on workspace {target}",
                window.label()
            ));
        }
        if !hyprctl::move_window(&window.address, &target) {
            return Err(format!(
                "Hyprland did not move {} to workspace {target}",
                window.label()
            ));
        }
        if let Some(entry) = look.windows.get_mut(index) {
            entry.workspace = target.clone();
        }
        // The chips carry a window count in their tooltip; move one from where
        // the window was to where it went, so the card keeps describing itself
        // truthfully. (Both workspaces are in the strip by construction: the
        // target because a drop can only land on a chip or on a window, and the
        // source because a window on it is drawn on the card.)
        for workspace in &mut look.workspaces {
            if workspace.name == window.workspace {
                workspace.windows = workspace.windows.saturating_sub(1);
            }
            if workspace.name == target {
                workspace.windows += 1;
            }
        }
        // The grouping is the workspace, so the corrected window goes wherever
        // the layout puts it - which is also why the selection follows the window
        // rather than the tile number.
        let focused = look.focused.clone();
        layout::order(&mut look.windows, &focused);
        let selected = look
            .windows
            .iter()
            .position(|other| other.address == window.address)
            .unwrap_or(0);
        self.draw(&look, selected, false);
        Ok(format!("{} → workspace {target}", window.label()))
    }

    fn close(&self) {
        if let Some(source) = self.idle.borrow_mut().take() {
            source.remove();
        }
        self.activity.set(None);
        self.osd.hide();
    }

    /// Close the card if nothing happens for a while.
    ///
    /// The card normally ends because you ended it - Escape, a click on the card
    /// itself, or the key again. This is the case where you *cannot*: a session
    /// lock takes the keyboard from every other surface, this one included, so a
    /// card that was up when the screen locked is up when it unlocks, behind a
    /// locker that heard the Escape you pressed. After a minute of no keys and no
    /// pointer movement over the card, it takes itself away; any interaction
    /// re-arms it, so looking at the card is not disturbed by it.
    ///
    /// The watchdog is one repeating timer that reads a clock, not a one-shot that
    /// is cancelled and rebuilt on every event: pointer motion arrives hundreds of
    /// times a second, and every one of those used to destroy a `GSource` and
    /// build another.
    fn arm_idle(self: &Rc<Self>) {
        self.activity.set(Some(Instant::now()));
        if self.idle.borrow().is_some() {
            return;
        }
        let Some(duration) = self.settings.idle_close else {
            return;
        };
        let me = Rc::clone(self);
        let source = glib::timeout_add_local(IDLE_TICK, move || {
            if !me.osd.is_visible() {
                return glib::ControlFlow::Break;
            }
            let Some(last) = me.activity.get() else {
                return glib::ControlFlow::Break;
            };
            if last.elapsed() < duration {
                return glib::ControlFlow::Continue;
            }
            // Take the id *before* closing: `close` would otherwise remove a
            // source that is in the middle of firing, which panics. Returning
            // `Break` is what destroys it.
            me.idle.borrow_mut().take();
            me.close();
            glib::ControlFlow::Break
        });
        *self.idle.borrow_mut() = Some(source);
    }

    /// Select a tile, if there is one.
    fn select(&self, index: usize) {
        let count = self.look.borrow().as_ref().map_or(0, Look::len);
        if index >= count {
            return;
        }
        self.selected.set(index);
        self.view.select(index);
    }

    /// Walk the tiles, wrapping around either end.
    fn step(&self, delta: i32) {
        let count = self.look.borrow().as_ref().map_or(0, Look::len) as i32;
        if count == 0 {
            return;
        }
        self.select((self.selected.get() as i32 + delta).rem_euclid(count) as usize);
    }

    /// Walk up or down a row, staying in the same column where there is one.
    ///
    /// Rows hold different numbers of tiles (they are packed by width, and
    /// windows have different shapes), so "the tile below this one" is the one at
    /// the same index in the row below - and the last one of that row when it is
    /// shorter, which is where the eye would look anyway.
    fn move_row(&self, delta: i32) {
        let rows = self.rows.borrow();
        if rows.is_empty() {
            return;
        }
        let selected = self.selected.get();
        let Some(row) = rows.iter().position(|row| row.contains(&selected)) else {
            return;
        };
        let column = rows[row]
            .iter()
            .position(|index| *index == selected)
            .unwrap_or(0);
        let target = (row as i32 + delta).rem_euclid(rows.len() as i32) as usize;
        let index = rows[target]
            .get(column)
            .or_else(|| rows[target].last())
            .copied()
            .unwrap_or(0);
        self.select(index);
    }

    /// Fill the tiles with pictures of the windows they stand for.
    ///
    /// One `grim` per window, all of them started at once and each answering
    /// when it is ready: the card is complete and usable - and closable - from
    /// the moment it is up, and the pictures fade in as they arrive.
    ///
    /// A picture the card already holds is put back instead of taken again
    /// (`recapture = false`, i.e. a repaint after a drop): the same window on the
    /// same screen looks the same, and its tile is a new widget that simply has
    /// nothing drawn in it yet.
    fn capture(self: &Rc<Self>, windows: &[Window], plan: &Plan, scale: f64, recapture: bool) {
        if recapture {
            self.pictures.borrow_mut().clear();
        }
        let tiles = self.view.tiles();
        let total = tiles.len();
        // (finished, captured): the element reports the one failure worth
        // reporting - not a single window could be captured at all, which is
        // what a missing `grim` looks like from here.
        let tally = Rc::new(Cell::new((0usize, 0usize)));
        for (index, tile) in tiles.into_iter().enumerate() {
            let Some(window) = windows.get(index) else {
                continue;
            };
            let width = plan.widths.get(index).copied().unwrap_or(240);
            if let Some(texture) = self.pictures.borrow().get(&window.stable_id) {
                tile.set_picture(texture);
                continue;
            }
            let window = window.clone();
            let me = Rc::clone(self);
            let tally = tally.clone();
            // The capture id is the cache key, so the closure can hold it rather
            // than the window it came from.
            let id = window.stable_id.clone();
            thumbs::capture(&window, scale, width, move |texture| {
                let (finished, captured) = tally.get();
                let captured = captured + usize::from(texture.is_some());
                tally.set((finished + 1, captured));
                match texture {
                    Some(texture) => {
                        // Kept for the card that is up. A window with no capture
                        // id has nothing to key it by, and one `grim` for it
                        // would fail anyway.
                        if !id.is_empty() {
                            me.pictures.borrow_mut().insert(id.clone(), texture.clone());
                        }
                        tile.set_picture(&texture);
                    }
                    None if finished + 1 == total && captured == 0 => {
                        eprintln!(
                            "hypr-osd-overview: not one window could be captured - thumbnails \
                             need grim, and a compositor that can capture a single toplevel"
                        );
                    }
                    None => {}
                }
            });
        }
    }
}

fn main() -> glib::ExitCode {
    let config = Config::load(ELEMENT);
    let settings = Rc::new(Settings::load(&config));

    let opts = Opts {
        app_id: APP_ID.to_string(),
        css: css::stylesheet(include_str!("overview.css")),
        namespace: NAMESPACE.to_string(),
        // The whole screen, because the overview *is* the desktop while it is
        // up: that is what makes it possible to see all of it at once.
        placement: Placement::Fill,
        // Escape, the arrows and the digits are the card's own keys while it is
        // up - see the module comment.
        keyboard: Keyboard::Exclusive,
        // `width` and `bottom_margin` are unused by a card that fills the
        // screen, and stay at the shell's defaults.
        ..Opts::default()
    };

    // The widgets are built inside the application's start-up (GTK does not
    // exist before that) and the command handler is installed before it, so the
    // two meet here. Built once: the window list and the selection have to
    // survive between commands, because `toggle` twice is one card opening and
    // closing, not two cards.
    let state: Rc<OnceCell<Rc<Overview>>> = Rc::new(OnceCell::new());

    let build = {
        let state = state.clone();
        let settings = settings.clone();
        Box::new(move |osd: &Rc<Osd>| {
            let view = Rc::new(OverviewView::new(&settings));
            let overview = Overview::new(osd, view.clone(), settings.clone());

            // A click on a tile is a switch, exactly like pressing Enter on it:
            // the card takes the pointer for the same reason it takes the
            // keyboard.
            view.on_tile_click({
                let overview = overview.clone();
                move |index| {
                    overview.select(index);
                    overview.commit();
                }
            });
            // A tile dragged onto a workspace chip (or onto another tile) says
            // where that window belongs. The card stays up: arranging workspaces
            // is something you do several times in one look.
            view.on_drop({
                let overview = overview.clone();
                move |index, drop| {
                    if let Err(reason) = overview.assign(index, drop) {
                        eprintln!("hypr-osd-overview: {reason}");
                    }
                }
            });
            view.on_workspace_click({
                let overview = overview.clone();
                move |name| overview.switch_workspace(name)
            });
            view.on_background_click({
                let overview = overview.clone();
                move || overview.close()
            });
            // The R key's hand on a touchscreen: a chip in the strip takes the
            // pictures again. Deferred by one idle turn, because the chip is part
            // of the strip a render replaces - a widget must not be torn down
            // inside its own click signal.
            view.on_recapture_click({
                let overview = overview.clone();
                move || {
                    let overview = overview.clone();
                    glib::idle_add_local_once(move || overview.refresh());
                }
            });
            // Moving the mouse over the card is looking at it, and looking at it
            // is not being idle.
            view.on_motion({
                let overview = overview.clone();
                move || overview.arm_idle()
            });
            // A finger cannot move a pointer that is not there until it lands:
            // presses and scrolls count as looking too, or the idle-close would
            // take the desktop away while it is being read.
            view.on_activity({
                let overview = overview.clone();
                move || overview.arm_idle()
            });

            // The daemon starts in whatever mode the machine is in, and the shell
            // only calls the tablet handlers on a *change*: the starting answer
            // has to be applied by hand.
            view.apply_touch(touch::tablet());

            // Folded or unfolded under the card: the chips and the hint follow
            // the window's `touch` class on their own, the strip's height is the
            // plan's input and does not - so the card takes a new plan, exactly
            // as `R` would.
            let watch = osd.clone();
            let watched = view.clone();
            let overview_watch = overview.clone();
            osd.on_tablet_change(move |tablet| {
                watched.apply_touch(tablet);
                if watch.is_visible() {
                    overview_watch.refresh();
                }
            });

            install_keys(osd, overview.clone());
            let root = view.root.clone().upcast::<gtk::Widget>();
            state.set(overview).ok();
            Content::Single(root)
        })
    };

    let handle = {
        let state = state.clone();
        Rc::new(
            move |_osd: &Rc<Osd>, args: &[String]| -> Result<String, String> {
                let overview = state
                    .get()
                    .ok_or("the overview has not finished starting up")?;
                let Some(verb) = args.first().map(String::as_str) else {
                    // No verb: started by Hyprland's autostart, so wait for the
                    // first tap of SUPER+SHIFT+TAB.
                    return Ok(String::new());
                };
                match verb {
                    "toggle" => overview.toggle(),
                    "show" => overview.open(),
                    "close" => overview.close(),
                    // The R key and the Recapture chip are the same action; this
                    // is how a script reaches it (a layer surface cannot be
                    // clicked from one).
                    "recapture" | "refresh" => overview.refresh(),
                    // A drag cannot be scripted any more than a click can, so the
                    // drop itself is reachable: it goes through the very code a
                    // drop does.
                    "move" => return move_verb(overview, &args[1..]),
                    "status" => return Ok(status_lines()),
                    other => {
                        return Err(format!(
                            "unknown verb `{other}` (toggle | show | close | recapture | \
                             move <window> <workspace> | status)"
                        ))
                    }
                }
                Ok(String::new())
            },
        )
    };

    run(opts, build, handle)
}

/// `move <window> <workspace>`: the drop without a pointer.
///
/// The window is named by its address (`0x…`, with or without the prefix) or by
/// its position in the tile order - which is what a `status` line's order is,
/// and what an arrow key walks. Exactly the same code a dragged tile ends in, so
/// this is how that gesture gets checked without a pointer.
fn move_verb(overview: &Rc<Overview>, args: &[String]) -> Result<String, String> {
    const USAGE: &str = "move needs a window and a workspace: `move <address|index> <workspace>`";
    let Some(which) = args.first() else {
        return Err(USAGE.to_string());
    };
    let Some(workspace) = args.get(1) else {
        return Err(USAGE.to_string());
    };
    let Some(look) = overview.look.borrow().clone() else {
        return Err("the card is not showing anything (open it first)".to_string());
    };
    let bare = |address: &str| address.trim_start_matches("0x").to_ascii_lowercase();
    let index = match which.parse::<usize>() {
        Ok(index) if index < look.windows.len() => index,
        _ => look
            .windows
            .iter()
            .position(|window| bare(&window.address) == bare(which))
            .ok_or_else(|| format!("no window `{which}` on the card"))?,
    };
    overview.assign(index, Drop::Workspace(workspace.clone()))
}

/// Wire the keys the card lives on.
///
/// Installed through the shell rather than on a widget here, because the shell
/// rebuilds the surface when the card moves to another monitor - a
/// keyboard-driven element that lost its keys after a monitor change would be a
/// bug nobody could explain.
fn install_keys(osd: &Rc<Osd>, overview: Rc<Overview>) {
    osd.on_keys(
        {
            move |key, state| {
                if !overview.osd.is_visible() {
                    return glib::Propagation::Proceed;
                }
                let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
                // `gdk::Key` is a newtype around the keyval rather than an enum,
                // so the arms below match on the value, not on a reference.
                let key = *key;
                let handled = match key {
                    gdk::Key::Tab => {
                        overview.step(if shift { -1 } else { 1 });
                        true
                    }
                    gdk::Key::ISO_Left_Tab => {
                        overview.step(-1);
                        true
                    }
                    gdk::Key::Right => {
                        overview.step(1);
                        true
                    }
                    gdk::Key::Left => {
                        overview.step(-1);
                        true
                    }
                    // A field of rows, so up and down are rows.
                    gdk::Key::Down => {
                        overview.move_row(1);
                        true
                    }
                    gdk::Key::Up => {
                        overview.move_row(-1);
                        true
                    }
                    gdk::Key::Escape => {
                        overview.close();
                        true
                    }
                    gdk::Key::Return | gdk::Key::KP_Enter => {
                        overview.commit();
                        true
                    }
                    // Letters by keyval *name*: `gdk::Key` is a newtype over the
                    // keyval, and its per-letter variants are the binding's
                    // business rather than a card's. `h`/`j`/`k`/`l` are the keys
                    // a Hyprland user's hands already know.
                    other => match other.name().as_deref() {
                        Some("h") => {
                            overview.step(-1);
                            true
                        }
                        Some("l") => {
                            overview.step(1);
                            true
                        }
                        Some("j") => {
                            overview.move_row(1);
                            true
                        }
                        Some("k") => {
                            overview.move_row(-1);
                            true
                        }
                        Some("r") => {
                            overview.refresh();
                            true
                        }
                        Some("q") => {
                            overview.close();
                            true
                        }
                        // 1-9 jump to a workspace, and 0 is the tenth - the way
                        // the example config's workspace binds number them. The
                        // workspace does not have to exist yet; Hyprland makes it.
                        Some(name) => match name.parse::<i32>() {
                            Ok(digit) if (0..=9).contains(&digit) => {
                                let workspace = if digit == 0 { 10 } else { digit };
                                overview.switch_workspace(&workspace.to_string());
                                true
                            }
                            _ => false,
                        },
                        None => false,
                    },
                };
                if handled {
                    // Anything you do counts as not being idle.
                    overview.arm_idle();
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        },
        // No release handling: unlike the switcher, this card is not watching a
        // key being held - it is a place you leave.
        |_| {},
    );
}

/// How much of the screen the tiles get.
///
/// Everything that is not a tile is subtracted: the card's padding, this
/// element's own padding, the strip, the hint line, and the gaps between the
/// three. The two fixed heights come from `view.rs`, where they are *given* to
/// the widgets rather than measured from them, so this number is not a guess
/// about font metrics. `strip` is passed in because it is not fixed: the
/// workspace chips are finger-sized on a tablet, and a strip planned at the
/// pointer's height would push the bottom row of tiles off the screen there.
fn area(output: Output, settings: &Settings, strip: i32) -> Area {
    let horizontal = 2 * (CARD_PAD_X + view::PADDING);
    let vertical = 2 * (CARD_PAD_Y + view::PADDING) + strip + view::HINT + 2 * settings.gap;
    Area {
        width: (output.width - horizontal).max(160),
        height: (output.height - vertical).max(120),
    }
}

/// The workspaces to draw in the strip: the ones Hyprland knows about, plus any
/// workspace a window claims that the list did not have - a window whose
/// workspace has just been created, say. A tile whose badge names a workspace
/// that is not in the strip would be the one thing on this card that does not
/// add up.
fn workspaces_in(windows: &[Window]) -> Vec<Workspace> {
    let mut list = windows::workspaces();
    for window in windows {
        if list
            .iter()
            .any(|workspace| workspace.name == window.workspace)
        {
            continue;
        }
        let count = windows
            .iter()
            .filter(|other| other.workspace == window.workspace)
            .count();
        list.push(Workspace {
            // Sorted last: it is the one we know least about.
            id: i32::MAX,
            name: window.workspace.clone(),
            windows: count as i32,
        });
    }
    list.sort_by_key(|workspace| workspace.id);
    list
}

/// What `status` answers: the workspaces and the windows the card would draw.
///
/// It exists for the two questions this element invites - "why is that window
/// not in the overview" and "why has that tile no picture" - and both are
/// answered by the ids Hyprland hands out: the workspace a window is on, and the
/// capture id its picture is asked for by.
fn status_lines() -> String {
    let focused = windows::focused_workspace().unwrap_or_default();
    let windows = windows::list();
    let workspaces = workspaces_in(&windows);

    let mut lines = vec![format!("focused workspace: {focused}")];
    if workspaces.is_empty() {
        lines.push("no workspaces".to_string());
    }
    for workspace in &workspaces {
        lines.push(format!(
            "workspace {} · {} windows{}",
            workspace.name,
            workspace.windows,
            if workspace.name == focused {
                " · current"
            } else {
                ""
            }
        ));
    }
    if windows.is_empty() {
        lines.push("no windows".to_string());
    }
    for window in &windows {
        let capture = if window.stable_id.is_empty() {
            "no capture id".to_string()
        } else {
            format!("capture {}", window.stable_id)
        };
        lines.push(format!("  {} · {capture}", window.describe()));
    }
    lines.join("\n")
}

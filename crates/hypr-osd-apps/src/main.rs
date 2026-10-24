//! `hypr-osd-apps` - element #7: the applications panel.
//!
//! The other half of the launcher. The launcher is the *search* card: it appears
//! in the middle of the screen when you already know what you want. This is the
//! *browser*: a panel that unfolds from the button at the bar's left end, with
//! every installed application in a grid, a sidebar of drawers to look through,
//! and a filter for the case where you half-know. It is the element that takes
//! over the main window of the Tauri/Next.js launcher this collection replaced
//! (its spotlight overlay became `hypr-osd-launcher`).
//!
//! Verbs (`hypr-osd-apps <verb>`):
//!
//! ```text
//!   open [connector] [x y w h]
//!                        what the bar's left-end button runs: the pointer has
//!                        arrived, so start watching for the dwell (see
//!                        `core::hover`)
//!   close | hide         take it away
//!   toggle               open the panel (pinned), or close it again
//!   show [query]         open it (pinned), with an empty filter or the one given
//!   refresh              re-scan the installed applications
//!   drawer <name>        show a drawer ("All", "Pinned", or a category's name)
//!   pin <id>             pin an application (its desktop file id)
//!   unpin <id>           unpin it
//!   apps                 list every application with the drawer it landed in
//!   status               what was scanned, what is pinned, open or closed
//!   (no verb)            start the daemon and wait for the button
//! ```
//!
//! Three things about how it behaves are deliberate:
//!
//! * **It unfolds from the button, like the island and the system popup.** The
//!   pointer arriving on the bar's left-end button *asks* the panel to open;
//!   this element watches the pointer itself (see `core::hover`) and takes the
//!   panel away once it has left both the button and the panel. The bar can only
//!   ever ask - a pinned panel is not watched, so a bar that sent `toggle` could
//!   leave it up for good.
//! * **It takes the keyboard while it is up.** The filter has to be typed into,
//!   and a layer-shell surface has no other way to be typed into (see
//!   `Keyboard::Exclusive`). Escape and the arrows are therefore the panel's own
//!   keys, and - like the launcher - it lets go of the keyboard by itself after
//!   `idle_close_ms` of silence, which is what makes that safe across a screen
//!   lock.
//! * **The card is one size, always.** The grid lives in a fixed-height scroller,
//!   so picking a different drawer cannot change the panel's height - see `view`.
//!
//! Pinned applications are the old launcher's favourites, in a plain text file
//! rather than JSON (see `pins`): right-click a tile to pin it, and the sidebar
//! grows a "Pinned" drawer to hold them.

mod pins;
mod view;

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::apps::{self, App};
use hypr_osd_core::hover::{self, Action, BarGeometry, Delays, Machine, Rect, Side};
use hypr_osd_core::timer::Timer;
use hypr_osd_core::{
    css, icons, run, state, text, Config, Content, Keyboard, Opts, Osd, Placement, CARD_PAD_X,
    CARD_PAD_Y,
};

use view::{AppsView, Row, Tile};

/// D-Bus application id - and therefore the single-instance key.
const APP_ID: &str = "com.schells2.osd.apps";
/// Layer-shell namespace; the collection shares the `hypr-osd` prefix so one
/// layer rule in `osd.lua` covers every element.
const NAMESPACE: &str = "hypr-osd";
/// Element name: `~/.config/hypr-osd/apps.conf`.
const ELEMENT: &str = "apps";

// Defaults; every one is overridable in the config file.
const DEFAULT_WIDTH: i32 = 760;
const DEFAULT_SIDEBAR_WIDTH: i32 = 168;
// Four rows: `4 * tile_height + 3 * tile_gap`. A height that is not a whole
// number of rows leaves the last one sliced in half at the pane's edge, which
// reads as a bug rather than as "there is more below".
const DEFAULT_GRID_HEIGHT: i32 = 402;
const DEFAULT_TILE_WIDTH: i32 = 100;
const DEFAULT_TILE_HEIGHT: i32 = 96;
const DEFAULT_ICON_SIZE: i32 = 40;
const DEFAULT_TILE_GAP: i32 = 6;
const DEFAULT_GAP: i32 = 6;
const DEFAULT_BAR_HEIGHT: i32 = 40;
const DEFAULT_BAR_MARGIN_TOP: i32 = 8;
const DEFAULT_BAR_MARGIN_X: i32 = 12;
// The strip at the bar's left end that counts as the button when a hand-typed
// `open` does not send the button's rectangle. The bar normally measures it, so
// this is only for a `hypr-osd-apps open` typed into a terminal.
const DEFAULT_HOT_WIDTH: i32 = 60;
const DEFAULT_OPEN_DELAY_MS: u64 = 120;
const DEFAULT_CLOSE_DELAY_MS: u64 = 300;
const DEFAULT_POLL_MS: u64 = 50;
const DEFAULT_IDLE_CLOSE_MS: u64 = 60_000;

/// What the config file resolved to.
struct Settings {
    /// The panel's width in pixels. The sidebar takes a slice of it and the tile
    /// grid gets the rest (see [`Settings::grid_width`]).
    width: i32,
    sidebar_width: i32,
    /// The height of the tile pane - and therefore the height the card settles
    /// at, whatever the drawer holds.
    grid_height: i32,
    tile_width: i32,
    tile_height: i32,
    icon_size: i32,
    /// Space between tiles.
    tile_gap: i32,
    /// Where the bar is, so the panel can hang below it, line its left edge up
    /// with the bar's and measure the two hover zones. These have to match
    /// `~/.config/hypr-osd/bar.conf`: the bar tells the panel nothing, it works
    /// the geometry out from these numbers (`gap` is the distance between the
    /// bar's bottom edge and the panel's top).
    bar: BarGeometry,
    /// The strip at the bar's left end used as the button when `open` sent no
    /// rectangle - a hand-typed `open` rather than the bar's own request, which
    /// carries the measured button.
    hot_width: f64,
    /// How long the pointer rests on the button before the panel opens, and how
    /// long it may be away before it closes again (see `hover::Delays`).
    delays: Delays,
    /// How often the pointer is sampled while the panel is being tracked.
    poll: Duration,
    /// Close after this long without a key press or pointer movement (`0` never).
    idle_close: Option<Duration>,
}

impl Settings {
    fn load(config: &Config) -> Self {
        let idle = config.millis("idle_close_ms", DEFAULT_IDLE_CLOSE_MS);
        Settings {
            width: config.i32("width", DEFAULT_WIDTH).max(420),
            sidebar_width: config.i32("sidebar_width", DEFAULT_SIDEBAR_WIDTH).max(96),
            grid_height: config.i32("grid_height", DEFAULT_GRID_HEIGHT).max(120),
            tile_width: config.i32("tile_width", DEFAULT_TILE_WIDTH).max(64),
            tile_height: config.i32("tile_height", DEFAULT_TILE_HEIGHT).max(56),
            icon_size: config.i32("icon_size", DEFAULT_ICON_SIZE).clamp(16, 96),
            tile_gap: config.i32("tile_gap", DEFAULT_TILE_GAP).clamp(0, 40),
            bar: BarGeometry {
                height: config.i32("bar_height", DEFAULT_BAR_HEIGHT).max(1) as f64,
                margin_top: config.i32("bar_margin_top", DEFAULT_BAR_MARGIN_TOP).max(0) as f64,
                margin_x: config.i32("bar_margin_x", DEFAULT_BAR_MARGIN_X).max(0) as f64,
                gap: config.i32("gap", DEFAULT_GAP).clamp(0, 60) as f64,
                // A `TopLeft` card keeps its shadow inset only below and to its
                // right - its top edge sits on the margin the placement asks
                // for - so the panel's visible top is exactly `margin_top()`.
                shadow_pad: 0.0,
            },
            hot_width: config.i32("hot_width", DEFAULT_HOT_WIDTH).max(1) as f64,
            delays: Delays {
                open: config.millis("open_delay_ms", DEFAULT_OPEN_DELAY_MS),
                close: config.millis("close_delay_ms", DEFAULT_CLOSE_DELAY_MS),
            },
            poll: config.millis("poll_ms", DEFAULT_POLL_MS),
            idle_close: if idle.is_zero() { None } else { Some(idle) },
        }
    }

    /// The width the shell leaves inside the card: `Opts::width` less the padding
    /// every card's content is inset by.
    fn content_width(&self) -> i32 {
        self.width - 2 * hypr_osd_core::CARD_PAD_X
    }

    /// The width the tile pane gets: what is left of the card after the sidebar
    /// and the gap between the two panes.
    fn grid_width(&self) -> i32 {
        (self.content_width() - self.sidebar_width - view::BODY_GAP).max(120)
    }

    /// How many tiles fit in a row. The element's arrow keys and the view's grid
    /// both come from here, so they cannot disagree about what "down" means.
    fn columns(&self) -> i32 {
        ((self.grid_width() + self.tile_gap) / (self.tile_width + self.tile_gap)).clamp(1, 10)
    }

    /// Where the card hangs: one gap below the bar, its left edge lined up with
    /// the bar's own margin.
    fn margin_top(&self) -> i32 {
        self.bar.margin_top as i32 + self.bar.height as i32 + self.bar.gap as i32
    }
}

/// A drawer in the sidebar: the two that are always there, and one per category
/// an application actually landed in.
#[derive(Clone, PartialEq, Eq)]
enum Drawer {
    All,
    Pinned,
    Category(String),
}

impl Drawer {
    fn label(&self) -> &str {
        match self {
            Drawer::All => "All",
            Drawer::Pinned => "Pinned",
            Drawer::Category(name) => name,
        }
    }

    /// Whether an application belongs in this drawer.
    fn holds(&self, app: &App, pins: &[String]) -> bool {
        match self {
            Drawer::All => true,
            Drawer::Pinned => pins.iter().any(|id| id == &app.id),
            Drawer::Category(name) => &app.category == name,
        }
    }
}

/// The element's state: what was scanned, what is shown, what is selected.
struct Panel {
    osd: Rc<Osd>,
    view: Rc<AppsView>,
    settings: Rc<Settings>,
    /// The hover's state machine (see `core::hover`): the bar's button only
    /// *asks*, and this decides when the panel has been dwelt on long enough to
    /// appear and when the pointer has been away long enough to take it down
    /// again.
    machine: RefCell<Machine>,
    /// The connector the bar is on, when it said so with `open`. The bar is
    /// pinned to the output it started on, which is not necessarily the focused
    /// one - so every coordinate this panel works with, and the surface it draws
    /// on, come from here rather than from "the focused output".
    monitor: RefCell<Option<String>>,
    /// The button's rectangle inside the bar's card, when the bar sent it. The
    /// bar is the only program that can measure the button, and its hot zone has
    /// to be the button: `hot_width` is only the fallback for a hand-typed open.
    pill: RefCell<Option<Rect>>,
    /// Every application on the machine, scanned once at start-up.
    apps: RefCell<Vec<App>>,
    /// The sidebar, in the order it is drawn.
    drawers: RefCell<Vec<Drawer>>,
    /// The drawer being shown.
    drawer: RefCell<Drawer>,
    /// The pinned desktop ids, in the order they were pinned.
    pins: RefCell<Vec<String>>,
    /// The filter as it is typed.
    query: RefCell<String>,
    /// The applications currently drawn, as indices into `apps` - parallel to the
    /// tiles the view holds.
    shown: RefCell<Vec<usize>>,
    selected: Cell<usize>,
    /// Icon lookups, so a reflow does not ask the icon theme for the same
    /// pictures again.
    icon_cache: RefCell<HashMap<String, Option<gdk::Paintable>>>,
    /// The idle-close watchdog (see [`Panel::arm_idle`]).
    idle: Rc<Timer>,
}

impl Panel {
    fn new(osd: &Rc<Osd>, view: Rc<AppsView>, settings: Rc<Settings>) -> Rc<Self> {
        Rc::new(Panel {
            osd: osd.clone(),
            view,
            settings,
            machine: RefCell::new(Machine::default()),
            monitor: RefCell::new(None),
            pill: RefCell::new(None),
            apps: RefCell::new(Vec::new()),
            drawers: RefCell::new(Vec::new()),
            drawer: RefCell::new(Drawer::All),
            pins: RefCell::new(Vec::new()),
            query: RefCell::new(String::new()),
            shown: RefCell::new(Vec::new()),
            selected: Cell::new(0),
            icon_cache: RefCell::new(HashMap::new()),
            idle: Rc::new(Timer::new()),
        })
    }

    /// The pointer arrived on the bar's button: start watching it for the dwell
    /// (`hover::Machine::arm`).
    ///
    /// This is all the bar ever says - it cannot take the panel away again, and
    /// it must not pin it either: a pinned panel is not watched, so a `toggle`
    /// from a hover would leave it up for good (see the bar's status pill's
    /// note). The connector and the button's rectangle come with the request
    /// because neither can be worked out here: the panel knows the bar's
    /// margins, but not which of the bars the pointer came from, nor how wide
    /// its own button is.
    fn ask(self: &Rc<Self>, connector: Option<&str>, pill: Option<Rect>) {
        if let Some(connector) = connector {
            // Where the bar is, so this panel can be there too - the shell moves
            // the surface if it is not.
            self.osd.pin_output(Some(connector));
            *self.monitor.borrow_mut() = Some(connector.to_string());
        }
        *self.pill.borrow_mut() = pill;
        self.machine.borrow_mut().arm(Instant::now());
    }

    /// Open the panel (pinned), or close it if it is already up - for a script;
    /// the bar's button sends `open` instead, because a pinned panel is not
    /// watched.
    fn toggle(self: &Rc<Self>) {
        let action = {
            let mut machine = self.machine.borrow_mut();
            if machine.is_open() {
                machine.dismiss()
            } else {
                machine.pin();
                Action::Show
            }
        };
        if action == Action::Show {
            self.present(None);
        }
        self.act(action);
    }

    /// Open the panel (pinned) on the "All" drawer with an empty filter, or the
    /// one given - the `show <query>` verb.
    fn show(self: &Rc<Self>, query: Option<&str>) {
        self.machine.borrow_mut().pin();
        self.present(query);
        self.act(Action::Show);
    }

    /// Take the panel away - `close`/`hide`, Escape, the idle watchdog and the
    /// pointer leaving it all end up here, so there is one way out.
    fn close(self: &Rc<Self>) {
        let action = self.machine.borrow_mut().dismiss();
        self.act(action);
    }

    /// Paint the panel: reset to the "All" drawer with the filter given, and put
    /// the finished grid in the card.
    ///
    /// Opening resets the view on purpose: the panel is a place you go to look
    /// for something, and one that reopened where you left it would make the
    /// button mean something different every time.
    fn present(self: &Rc<Self>, query: Option<&str>) {
        let query = query.unwrap_or("");

        *self.drawer.borrow_mut() = Drawer::All;
        self.selected.set(0);
        self.rebuild_drawers();

        self.osd.set_content(&self.view.root);
        *self.query.borrow_mut() = query.to_string();
        // `set_query` fires the change handler, which is what paints the grid;
        // the explicit call covers the case where the field already held this
        // text and GTK therefore saw no change at all.
        self.view.set_query(query);
        self.changed(query);
    }

    /// Show or hide the surface, and say so in the state flag while doing it.
    ///
    /// The flag is what the bar's button reads to light up. It is written here
    /// and nowhere else, so the two programs cannot disagree about whether the
    /// panel is up.
    fn act(self: &Rc<Self>, action: Action) {
        match action {
            Action::None => {}
            Action::Show => {
                self.osd.show();
                state::write(&state::apps_panel(), true);
                self.view.focus_entry();
                // Again once GTK has laid the panel out: a freshly mapped
                // surface can refuse focus until it has been allocated.
                let view = self.view.clone();
                glib::timeout_add_local_once(Duration::ZERO, move || view.focus_entry());
                self.arm_idle();
            }
            Action::Hide => {
                self.idle.cancel();
                state::write(&state::apps_panel(), false);
                self.osd.hide();
            }
        }
    }

    /// Re-scan the installed applications and paint the result.
    fn refresh(self: &Rc<Self>) {
        *self.apps.borrow_mut() = apps::scan();
        *self.pins.borrow_mut() = pins::load();
        self.rebuild_drawers();
        self.recompute();
        self.arm_idle();
    }

    /// The filter changed: narrow the grid.
    fn changed(self: &Rc<Self>, query: &str) {
        *self.query.borrow_mut() = query.to_string();
        self.selected.set(0);
        self.recompute();
        self.arm_idle();
    }

    /// Work out the sidebar from the applications and the pins.
    fn rebuild_drawers(self: &Rc<Self>) {
        let mut drawers = vec![Drawer::All];
        if !self.pins.borrow().is_empty() {
            drawers.push(Drawer::Pinned);
        }
        for name in apps::categories_in(&self.apps.borrow()) {
            drawers.push(Drawer::Category(name));
        }
        // A drawer that is gone (the last pin was unpinned) falls back to "All"
        // rather than leaving the panel showing nothing.
        let current = self.drawer.borrow().clone();
        if !drawers.contains(&current) {
            *self.drawer.borrow_mut() = Drawer::All;
        }
        *self.drawers.borrow_mut() = drawers;
    }

    /// Build the sidebar and the grid for the current drawer and filter, and draw
    /// them.
    fn recompute(self: &Rc<Self>) {
        let query = self.query.borrow().trim().to_lowercase();
        let drawer = self.drawer.borrow().clone();

        let shown: Vec<usize> = {
            let apps = self.apps.borrow();
            let pins = self.pins.borrow();
            apps.iter()
                .enumerate()
                .filter(|(_, app)| drawer.holds(app, &pins) && apps::matches(app, &query))
                .map(|(index, _)| index)
                .collect()
        };

        let rows: Vec<Row> = {
            let apps = self.apps.borrow();
            let pins = self.pins.borrow();
            self.drawers
                .borrow()
                .iter()
                .map(|entry| Row {
                    label: entry.label().to_string(),
                    count: apps.iter().filter(|app| entry.holds(app, &pins)).count(),
                    selected: *entry == drawer,
                })
                .collect()
        };

        let tiles: Vec<Tile> = {
            let apps = self.apps.borrow();
            shown
                .iter()
                .map(|index| self.tile(*index, &apps[*index]))
                .collect()
        };

        // The selection is a position in a list that just changed length.
        let selected = self.selected.get().min(shown.len().saturating_sub(1));
        self.selected.set(selected);
        *self.shown.borrow_mut() = shown;

        self.view.set_hint(if tiles.is_empty() {
            "Nothing here matches that."
        } else {
            view::KEYS_HINT
        });
        self.view.render_sidebar(&rows);
        self.view.render_tiles(&tiles, selected);
    }

    /// The tile for one application: its icon (or its first letter), its name, and
    /// whether it is pinned.
    fn tile(&self, index: usize, app: &App) -> Tile {
        Tile {
            key: index,
            icon: app.icon.as_deref().and_then(|name| self.icon(name)),
            glyph: text::initial(&app.name),
            label: app.name.clone(),
            pinned: self.pins.borrow().iter().any(|id| id == &app.id),
        }
    }

    /// A themed icon by name, remembered between reflows - the same pictures come
    /// back on every keystroke, and `by_name` walks the icon theme's directories
    /// on a miss.
    fn icon(&self, name: &str) -> Option<gdk::Paintable> {
        if let Some(found) = self.icon_cache.borrow().get(name) {
            return found.clone();
        }
        let found = icons::by_name(name, self.settings.icon_size);
        self.icon_cache
            .borrow_mut()
            .insert(name.to_string(), found.clone());
        found
    }

    /// The application at a position in the grid, if there is one.
    fn app_at(&self, index: usize) -> Option<App> {
        let app_index = *self.shown.borrow().get(index)?;
        self.apps.borrow().get(app_index).cloned()
    }

    /// Select a drawer by its index in the sidebar.
    fn select_drawer(self: &Rc<Self>, index: usize) {
        let Some(drawer) = self.drawers.borrow().get(index).cloned() else {
            return;
        };
        *self.drawer.borrow_mut() = drawer;
        self.selected.set(0);
        self.recompute();
        self.arm_idle();
    }

    /// Walk the sidebar - `Tab` and `Shift + Tab`.
    fn next_drawer(self: &Rc<Self>, delta: i32) {
        let count = self.drawers.borrow().len() as i32;
        if count == 0 {
            return;
        }
        let current = self
            .drawers
            .borrow()
            .iter()
            .position(|drawer| *drawer == *self.drawer.borrow())
            .unwrap_or(0) as i32;
        self.select_drawer((current + delta).rem_euclid(count) as usize);
    }

    /// Select a position in the grid, clamped to the tiles there are.
    fn select(self: &Rc<Self>, index: usize) {
        let count = self.shown.borrow().len();
        if count == 0 {
            return;
        }
        let index = index.min(count - 1);
        self.selected.set(index);
        self.view.select(index);
    }

    /// Walk the grid. `delta` is in tiles: `±1` is a step along a row, `±columns`
    /// a row up or down.
    ///
    /// Walking off an edge stays on the grid rather than wrapping: a grid that
    /// jumped from the last tile back to the first would move the whole pane under
    /// the pointer.
    fn step(self: &Rc<Self>, delta: i32) {
        let count = self.shown.borrow().len() as i32;
        if count == 0 {
            return;
        }
        self.select((self.selected.get() as i32 + delta).clamp(0, count - 1) as usize);
        self.arm_idle();
    }

    fn activate_selected(self: &Rc<Self>) {
        self.activate(self.selected.get());
    }

    /// Start the application at `index` and take the panel down.
    ///
    /// The panel goes first: an application can take a moment to appear, and
    /// leaving a grid over the window it just opened makes the button look stuck.
    fn activate(self: &Rc<Self>, index: usize) {
        let Some(app) = self.app_at(index) else {
            return;
        };
        self.close();
        apps::launch(&app);
    }

    /// Pin the application at `index`, or unpin it - what a right click does.
    fn toggle_pin(self: &Rc<Self>, index: usize) {
        let Some(app) = self.app_at(index) else {
            return;
        };
        {
            let mut pins = self.pins.borrow_mut();
            pins::toggle(&mut pins, &app.id);
            pins::save(&pins);
        }
        // The sidebar may have grown or lost its "Pinned" drawer, and the grid is
        // being looked at - so it is repainted where it stands.
        self.rebuild_drawers();
        self.recompute();
        self.arm_idle();
    }

    /// Close the panel if nothing happens for a while.
    ///
    /// Escape and the button are how the panel normally ends, but neither works
    /// after the screen has been locked - a session lock takes the keyboard from
    /// every other surface, this one included, so a panel that was up when the
    /// screen locked hears nothing. After a minute of no keys and no pointer
    /// movement it takes itself away; every interaction re-arms it.
    fn arm_idle(self: &Rc<Self>) {
        let Some(duration) = self.settings.idle_close else {
            return;
        };
        let me = Rc::clone(self);
        self.idle.arm(duration, move || {
            if me.osd.is_visible() {
                me.close();
            }
        });
    }

    // -- what the verbs answer without drawing a panel -----------------------

    fn status_lines(&self) -> String {
        let drawer = self.drawer.borrow();
        // What the machine makes of the pointer - and in particular whether the
        // panel is still being watched. A panel that is up and *not* watched can
        // only be closed by a verb, which is the answer to "why will it not go
        // away" (`show` and `toggle` pin it; the bar never does).
        let state = {
            let machine = self.machine.borrow();
            match (machine.is_open(), machine.is_watching()) {
                (false, _) => "closed".to_string(),
                (true, true) => "open, watching the pointer".to_string(),
                (true, false) => "open, pinned - only a verb closes this".to_string(),
            }
        };
        format!(
            "apps {}\ndrawers {}\nshowing {}\npinned {}\nstate {state}\n{}\nsize {}x{} ({} columns of {}x{})\nmeasured: {}",
            self.apps.borrow().len(),
            self.drawers.borrow().len(),
            drawer.label(),
            self.pins.borrow().len(),
            self.zone_line(),
            self.settings.width,
            self.settings.grid_height,
            self.settings.columns(),
            self.settings.tile_width,
            self.settings.tile_height,
            self.view.measured(),
        )
    }

    /// Where the two hover zones ended up, in the layout.
    ///
    /// There is no way to move a pointer from here, so this is how "the panel
    /// does not open" (or "will not close") gets diagnosed: the button's zone has
    /// to be the button, and the panel's zone the panel.
    fn zone_line(&self) -> String {
        let Some((pill, panel)) = zones(self, &self.settings) else {
            return "zones none - the compositor did not answer hyprctl".to_string();
        };
        format!(
            "zones pill {:.0},{:.0} {:.0}x{:.0}{}  panel {:.0},{:.0} to {:.0},{:.0}",
            pill.x,
            pill.y,
            pill.width,
            pill.height,
            if self.pill.borrow().is_some() {
                " (from the bar)"
            } else {
                " (fallback strip)"
            },
            panel.x,
            panel.y,
            panel.x + panel.width,
            panel.bottom(),
        )
    }

    /// Every application with the drawer it landed in - how "why is that app in
    /// Utilities" is answered.
    fn app_lines(&self) -> String {
        let apps = self.apps.borrow();
        if apps.is_empty() {
            return "no applications".to_string();
        }
        apps.iter()
            .map(|app| format!("{} \u{2014} {} [{}]", app.category, app.name, app.id))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Pin or unpin by id, for a script (`pin firefox`).
    fn set_pin(self: &Rc<Self>, id: &str, wanted: bool) {
        {
            let mut pins = self.pins.borrow_mut();
            if pins.iter().any(|known| known == id) == wanted {
                return;
            }
            pins::toggle(&mut pins, id);
            pins::save(&pins);
        }
        self.rebuild_drawers();
        self.recompute();
    }

    /// Show a drawer by name, for a script - `drawer Games`, `drawer Pinned`,
    /// `drawer All` (or a number, which is the sidebar's order).
    fn show_drawer(self: &Rc<Self>, name: &str) -> Result<(), String> {
        let index = if let Ok(number) = name.parse::<usize>() {
            number.saturating_sub(1)
        } else {
            let wanted = name.to_lowercase();
            self.drawers
                .borrow()
                .iter()
                .position(|drawer| drawer.label().to_lowercase() == wanted)
                .ok_or_else(|| format!("no drawer named `{name}`"))?
        };
        if index >= self.drawers.borrow().len() {
            return Err(format!("no drawer at position {name}"));
        }
        self.select_drawer(index);
        Ok(())
    }
}

/// One pointer sample: read the cursor, work out whether it is over the button
/// or over the panel, and let the machine decide what to do about it.
fn sample(panel: &Rc<Panel>, settings: &Settings) -> Action {
    let Some((pill, panel_rect)) = zones(panel, settings) else {
        return Action::None;
    };
    let Some(cursor) = hover::cursor() else {
        // The compositor did not answer. Leaving the state alone is the right
        // reaction: a panel that closed because a socket hiccupped would be worse
        // than one that stayed up a moment too long.
        return Action::None;
    };
    panel.machine.borrow_mut().sample(
        pill.contains(cursor, 0.0),
        panel_rect.contains(cursor, 8.0),
        Instant::now(),
        settings.delays,
    )
}

/// The two rectangles the machine reasons about, in the layout: the button's hot
/// zone at the left end of the bar, and where the panel itself is.
///
/// `None` when the compositor cannot say where the monitor is - which is also
/// when nothing should open, because neither rectangle means anything without it.
///
/// The panel's own position needs no query: `TopLeft` anchoring plus the known
/// margins make it computable. The button's is the rectangle the bar sent with
/// `open`, translated out of the bar's card into the layout - the card is the one
/// origin both programs can agree on, and the bar is the only one that can
/// measure its own button.
fn zones(panel: &Panel, settings: &Settings) -> Option<(Rect, Rect)> {
    // The monitor the *bar* is on, not the focused one: the panel unfolds from
    // the bar, so every zone it has is measured in the bar's coordinates. The
    // focused output is the fallback for a hand-typed `open`, and for a bar too
    // old to say where it is.
    let monitor = panel
        .monitor
        .borrow()
        .as_deref()
        .and_then(hover::monitor_of)
        .or_else(hover::focused_monitor)?;
    // The card is the content *plus* the padding the shell puts around it, which
    // is what is really on screen.
    let (width, height) = {
        let root = &panel.view.root;
        (
            f64::from(root.width() + 2 * CARD_PAD_X),
            f64::from(root.height() + 2 * CARD_PAD_Y),
        )
    };
    let pill = panel
        .pill
        .borrow()
        .map(|rect| Rect {
            x: monitor.x + settings.bar.margin_x + rect.x,
            y: monitor.y + settings.bar.margin_top + rect.y,
            ..rect
        })
        .unwrap_or_else(|| hover::pill_rect(monitor, settings.bar, settings.hot_width, Side::Left));
    let panel_rect = hover::panel_rect(monitor, settings.bar, Side::Left, (width, height));
    Some((pill, panel_rect))
}

/// The `open` arguments: the connector the bar is on, and the button's rectangle
/// inside the bar's card.
///
/// Both are optional, and positional, so the connector is found by looking for
/// the first number instead of by counting: the bar sends
/// `open eDP-1 0 5 44 28`, and a hand-typed `open eDP-1` (or `open 0 5 44 28`)
/// does the sensible thing too.
fn parse_open(args: &[String]) -> (Option<String>, Option<Rect>) {
    let start = args.iter().position(|arg| arg.parse::<f64>().is_ok());
    let rect = start.and_then(|index| parse_rect(&args[index..]));
    let connector = match start {
        // Everything before the numbers is the connector; it has no spaces, so
        // the word right in front of the rectangle is the one.
        Some(index) => index
            .checked_sub(1)
            .and_then(|index| args.get(index))
            .cloned(),
        // No rectangle at all: the first word is the connector (`open eDP-1`).
        None => args.first().filter(|arg| !arg.is_empty()).cloned(),
    };
    (connector, rect)
}

/// The four numbers the bar sends with `open`, or `None` when it did not.
fn parse_rect(args: &[String]) -> Option<Rect> {
    let numbers: Vec<f64> = args
        .iter()
        .filter_map(|value| value.parse().ok())
        .take(4)
        .collect();
    let [x, y, width, height] = <[f64; 4]>::try_from(numbers).ok()?;
    Some(Rect {
        x,
        y,
        width,
        height,
    })
}

/// Wire the keys the panel lives on.
///
/// Installed through the shell rather than on the widgets here, because the shell
/// rebuilds the surface when the card moves to another monitor - a keyboard card
/// that lost its keys after a monitor change would be a bug nobody could explain.
///
/// The handler takes only what the *panel* has an opinion about and lets
/// everything else through: the filter is the entry's job, and swallowing letters
/// here would make it untypeable.
fn install_keys(osd: &Rc<Osd>, panel: Rc<Panel>) {
    osd.on_keys(
        {
            let panel = panel.clone();
            move |key, state| {
                if !panel.osd.is_visible() {
                    return glib::Propagation::Proceed;
                }
                // With Ctrl held, the arrows and Home/End are the entry's own
                // editing keys, not the grid's.
                let control = state.contains(gdk::ModifierType::CONTROL_MASK);
                let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
                // `gdk::Key` is a newtype around the keyval rather than an enum,
                // so the arms below match on the value, not on a reference.
                let key = *key;
                // What "down" means comes from the view, which is the half that
                // actually laid the tiles out.
                let columns = panel.view.columns();
                let page = panel.view.page();
                let handled = match key {
                    gdk::Key::Return | gdk::Key::KP_Enter => {
                        panel.activate_selected();
                        true
                    }
                    gdk::Key::Escape => {
                        panel.close();
                        true
                    }
                    // The sidebar is walked with Tab rather than with a second
                    // focus chain: the field keeps the keyboard (that is the whole
                    // point), so there is nothing for GTK to walk between.
                    gdk::Key::Tab => {
                        panel.next_drawer(if shift { -1 } else { 1 });
                        true
                    }
                    gdk::Key::ISO_Left_Tab => {
                        panel.next_drawer(-1);
                        true
                    }
                    gdk::Key::Right if !control => {
                        panel.step(1);
                        true
                    }
                    gdk::Key::Left if !control => {
                        panel.step(-1);
                        true
                    }
                    gdk::Key::Down if !control => {
                        panel.step(columns);
                        true
                    }
                    gdk::Key::Up if !control => {
                        panel.step(-columns);
                        true
                    }
                    gdk::Key::Page_Down if !control => {
                        panel.step(page);
                        true
                    }
                    gdk::Key::Page_Up if !control => {
                        panel.step(-page);
                        true
                    }
                    gdk::Key::Home if !control => {
                        panel.select(0);
                        true
                    }
                    gdk::Key::End if !control => {
                        panel.select(usize::MAX);
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
        // No release handling: this panel is a place you leave, not a key you
        // hold (that is the switcher's shape).
        |_| {},
    );
}

fn main() -> glib::ExitCode {
    let config = Config::load(ELEMENT);
    let settings = Rc::new(Settings::load(&config));

    let opts = Opts {
        app_id: APP_ID.to_string(),
        css: css::stylesheet(include_str!("apps.css")),
        namespace: NAMESPACE.to_string(),
        width: settings.width,
        // Under the bar's left end, where the button that opens it is: the panel
        // is what came out of that button, not a card in the middle of the screen.
        placement: Placement::TopLeft {
            margin_top: settings.margin_top(),
            margin_left: settings.bar.margin_x as i32,
        },
        // It has to be typed into - see the module comment.
        keyboard: Keyboard::Exclusive,
        // Unused by a panel hanging off the bar, kept at the shell's default.
        bottom_margin: 0.12,
        // Cards follow the focused monitor; only the bar pins itself.
        ..Opts::default()
    };

    // The widgets are built inside the application's start-up (GTK does not exist
    // before that) and the command handler is installed before it, so the two meet
    // here. Built once: the scan, the pins and the selection have to survive
    // between commands, because `toggle` twice is one panel opening and closing.
    let state_cell: Rc<OnceCell<Rc<Panel>>> = Rc::new(OnceCell::new());

    let build = {
        let state_cell = state_cell.clone();
        let settings = settings.clone();
        Box::new(move |osd: &Rc<Osd>| {
            let view = Rc::new(AppsView::new(&settings));
            let panel = Panel::new(osd, view.clone(), settings.clone());

            // The scan is the one slow-ish thing here (a few hundred small files),
            // and it happens once, while nobody is waiting: at start-up, so the
            // first click on the button paints immediately.
            *panel.apps.borrow_mut() = apps::scan();
            *panel.pins.borrow_mut() = pins::load();
            panel.rebuild_drawers();
            panel.recompute();
            // A `kill`ed panel leaves "open" behind, which would light the button
            // up with nothing behind it - so the flag starts closed, exactly as the
            // island's does, and is cleared again on the way out.
            state::write(&state::apps_panel(), false);
            osd.on_shutdown(|| state::write(&state::apps_panel(), false));

            view.on_query({
                let panel = panel.clone();
                move |query| panel.changed(query)
            });
            view.on_drawer({
                let panel = panel.clone();
                move |index| panel.select_drawer(index)
            });
            view.on_tile({
                let panel = panel.clone();
                move |index| panel.activate(index)
            });
            view.on_pin({
                let panel = panel.clone();
                move |index| panel.toggle_pin(index)
            });
            // Moving the pointer over the panel is looking at it, and looking at
            // it is not being idle.
            view.on_motion({
                let panel = panel.clone();
                move || panel.arm_idle()
            });

            // The pointer, on its own faster clock - and only while the panel is
            // being tracked. An idle panel asks the compositor for nothing at
            // all, where a poll for the whole session would be a socket
            // round-trip every 50 ms for as long as the element lives.
            {
                let panel = panel.clone();
                let settings = settings.clone();
                glib::timeout_add_local(settings.poll, move || {
                    if !panel.machine.borrow().is_watching() {
                        return glib::ControlFlow::Continue;
                    }
                    let action = sample(&panel, &settings);
                    if action == Action::Show {
                        // Painted before the surface appears, so the first frame
                        // is the finished grid rather than an empty card filling
                        // in behind the pointer.
                        panel.present(None);
                    }
                    panel.act(action);
                    glib::ControlFlow::Continue
                });
            }

            install_keys(osd, panel.clone());
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
                    .ok_or("the applications panel has not finished starting up")?;
                let Some(verb) = args.first().map(String::as_str) else {
                    // No verb: started by Hyprland's autostart, so wait for the
                    // button.
                    return Ok(String::new());
                };
                match verb {
                    // What the bar's left-end button runs when the pointer
                    // arrives. It only *arms* the panel: the dwell, and the
                    // decision to open, are the panel's own (see `hover`).
                    "open" => {
                        let (connector, rect) = parse_open(&args[1..]);
                        panel.ask(connector.as_deref(), rect);
                    }
                    "toggle" => panel.toggle(),
                    "show" => panel.show(args.get(1).map(String::as_str)),
                    "hide" | "close" => panel.close(),
                    "refresh" => panel.refresh(),
                    "drawer" => {
                        let name = args[1..].join(" ");
                        panel.show_drawer(&name)?;
                    }
                    "pin" | "unpin" => {
                        let id = args.get(1).ok_or(format!("`{verb}` needs a desktop id"))?;
                        panel.set_pin(id, verb == "pin");
                    }
                    "apps" => return Ok(panel.app_lines()),
                    "status" => return Ok(panel.status_lines()),
                    other => {
                        return Err(format!(
                            "unknown verb `{other}` (open [connector] [x y w h] | toggle | \
                             show [query] | hide | refresh | drawer <name> | pin <id> | \
                             unpin <id> | apps | status)"
                        ))
                    }
                }
                Ok(String::new())
            },
        )
    };

    run(opts, build, handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str, category: &str) -> App {
        App {
            id: id.to_string(),
            name: name.to_string(),
            category: category.to_string(),
            ..App::default()
        }
    }

    #[test]
    fn a_drawer_holds_what_it_says_it_does() {
        let browser = app("firefox", "Firefox", "Internet & Networking");
        let pins = vec!["firefox".to_string()];

        assert!(Drawer::All.holds(&browser, &[]));
        assert!(Drawer::Category("Internet & Networking".into()).holds(&browser, &[]));
        assert!(!Drawer::Category("Games".into()).holds(&browser, &[]));
        assert!(Drawer::Pinned.holds(&browser, &pins));
        assert!(!Drawer::Pinned.holds(&browser, &[]));
    }

    #[test]
    fn the_settings_agree_with_themselves_about_the_grid() {
        let settings = Settings {
            width: 760,
            sidebar_width: 168,
            grid_height: 372,
            tile_width: 100,
            tile_height: 96,
            icon_size: 40,
            tile_gap: 6,
            bar: BarGeometry {
                height: 40.0,
                margin_top: 8.0,
                margin_x: 12.0,
                gap: 6.0,
                shadow_pad: 0.0,
            },
            hot_width: 60.0,
            delays: Delays {
                open: Duration::from_millis(120),
                close: Duration::from_millis(300),
            },
            poll: Duration::from_millis(50),
            idle_close: None,
        };
        // 760 - 2*16 = 728 of content; less the sidebar and the pane gap.
        assert_eq!(settings.content_width(), 728);
        assert_eq!(settings.grid_width(), 728 - 168 - view::BODY_GAP);
        // 550 of pane fits five 100px tiles and four 6px gaps.
        assert_eq!(settings.columns(), 5);
        // The panel hangs one gap below the bar's bottom edge.
        assert_eq!(settings.margin_top(), 40 + 8 + 6);
    }

    /// The bar sends `open <connector> <x> <y> <w> <h>`; the arguments are
    /// optional, so both halves have to survive being left out - the connector is
    /// found by looking for the first number, not by counting words.
    #[test]
    fn the_open_arguments_are_positional() {
        let words = |args: &[&str]| args.iter().map(|word| word.to_string()).collect::<Vec<_>>();

        let (connector, pill) = parse_open(&words(&["eDP-1", "0", "5", "44", "28"]));
        assert_eq!(connector.as_deref(), Some("eDP-1"));
        assert_eq!(
            pill,
            Some(Rect {
                x: 0.0,
                y: 5.0,
                width: 44.0,
                height: 28.0,
            })
        );

        // Hand-typed, without a rectangle: the connector is still the connector.
        let (connector, pill) = parse_open(&words(&["DP-1"]));
        assert_eq!(connector.as_deref(), Some("DP-1"));
        assert_eq!(pill, None);

        // Hand-typed, without a connector: the fallback strip is the zone.
        let (connector, pill) = parse_open(&words(&["0", "5", "44", "28"]));
        assert_eq!(connector, None);
        assert!(pill.is_some());

        // Neither: nothing to place, nothing to measure.
        assert_eq!(parse_open(&[]), (None, None));
    }
}

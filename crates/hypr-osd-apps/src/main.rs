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
//!   toggle               open the panel, or close it again (what the bar's
//!                        left-end button runs)
//!   show [query]         open it, with an empty filter or the one given
//!   hide | close         take it away
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
//! * **It is a panel you asked for, not one you hover.** Clicking the button
//!   opens it and it stays put until Escape, the button again, the application you
//!   picked, or the idle watchdog. The island and the system popup follow the
//!   pointer because they *inform*; a grid you are reading would be unusable if it
//!   slid away every time the pointer wandered off it.
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
use std::time::Duration;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::apps::{self, App};
use hypr_osd_core::timer::Timer;
use hypr_osd_core::{
    css, icons, run, state, text, Config, Content, Keyboard, Opts, Osd, Placement,
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
    /// Space between the bar's bottom edge and the panel's top edge.
    gap: i32,
    /// Where the bar is, so the panel can hang below it and line its left edge up
    /// with the bar's. These have to match `~/.config/hypr-osd/bar.conf`: the bar
    /// tells the panel nothing, it works the geometry out from these numbers.
    bar_height: i32,
    bar_margin_top: i32,
    bar_margin_x: i32,
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
            gap: config.i32("gap", DEFAULT_GAP).clamp(0, 60),
            bar_height: config.i32("bar_height", DEFAULT_BAR_HEIGHT).max(1),
            bar_margin_top: config.i32("bar_margin_top", DEFAULT_BAR_MARGIN_TOP).max(0),
            bar_margin_x: config.i32("bar_margin_x", DEFAULT_BAR_MARGIN_X).max(0),
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
        self.bar_height + self.bar_margin_top + self.gap
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

    /// Open the panel, or close it if it is already up - what the bar's button
    /// runs, because one click should be able to undo itself.
    fn toggle(self: &Rc<Self>) {
        if self.osd.is_visible() {
            self.close();
        } else {
            self.open(None);
        }
    }

    /// Open the panel at the "All" drawer with an empty filter (or the one given,
    /// which is for `show <query>` rather than for the button).
    ///
    /// Opening resets the view on purpose: the panel is a place you go to look for
    /// something, and one that reopened where you left it would make the button
    /// mean something different every time.
    fn open(self: &Rc<Self>, query: Option<&str>) {
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

        self.osd.show();
        // The bar's button reads this to light up while the panel is up (see
        // `state`).
        state::write(&state::apps_panel(), true);

        self.view.focus_entry();
        // Again once GTK has laid the panel out: a freshly mapped surface can
        // refuse focus until it has been allocated.
        let view = self.view.clone();
        glib::timeout_add_local_once(Duration::ZERO, move || view.focus_entry());
    }

    fn close(self: &Rc<Self>) {
        self.idle.cancel();
        state::write(&state::apps_panel(), false);
        self.osd.hide();
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
        let state = if self.osd.is_visible() {
            "open"
        } else {
            "closed"
        };
        format!(
            "apps {}\ndrawers {}\nshowing {}\npinned {}\nstate {state}\nsize {}x{} ({} columns of {}x{})\nmeasured: {}",
            self.apps.borrow().len(),
            self.drawers.borrow().len(),
            drawer.label(),
            self.pins.borrow().len(),
            self.settings.width,
            self.settings.grid_height,
            self.settings.columns(),
            self.settings.tile_width,
            self.settings.tile_height,
            self.view.measured(),
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
            margin_left: settings.bar_margin_x,
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
                    "toggle" => panel.toggle(),
                    "show" => panel.open(args.get(1).map(String::as_str)),
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
                            "unknown verb `{other}` (toggle | show [query] | hide | refresh | \
                             drawer <name> | pin <id> | unpin <id> | apps | status)"
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
            gap: 6,
            bar_height: 40,
            bar_margin_top: 8,
            bar_margin_x: 12,
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
}

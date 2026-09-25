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
//! This is the second element that takes the keyboard. The switcher does it to
//! hear the Alt release that ends a switch; this one does it because a card you
//! opened on purpose owns Escape, the arrows and its digits while it is up, and
//! there is no way to hear those keys and also let them through to whatever is
//! behind the card. Both are cards you asked for; every other element still
//! never takes a keystroke.
//!
//! Verbs (`hypr-osd-overview <verb>`):
//!
//! ```text
//!   toggle               open it, or close it again (what SUPER+SHIFT+TAB runs)
//!   show                 open it
//!   close                close it without switching anything
//!   status               list the workspaces and windows, no card
//!   (no verb)            start the daemon and wait for the first tap
//! ```

mod layout;
mod thumbs;
mod view;

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::monitors::{self, Output};
use hypr_osd_core::windows::{self, Window, Workspace};
use hypr_osd_core::{
    css, hyprctl, run, Config, Content, Keyboard, Opts, Osd, Placement, CARD_PAD_X, CARD_PAD_Y,
};

use layout::{Area, Limits, Plan};
use view::OverviewView;

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

/// The overview's state: what is on the card, and which tile is selected.
struct Overview {
    osd: Rc<Osd>,
    view: Rc<OverviewView>,
    settings: Rc<Settings>,
    /// The windows as they are drawn, in drawing order. Read on the way out (to
    /// focus one) and when a picture arrives (to find the tile it belongs to).
    windows: RefCell<Vec<Window>>,
    /// The same windows, grouped into the rows they were drawn in. This is what
    /// makes "down" a row rather than a tile.
    rows: RefCell<Vec<Vec<usize>>>,
    selected: Cell<usize>,
    /// The idle-close watchdog (see [`Overview::arm_idle`]).
    idle: RefCell<Option<glib::SourceId>>,
}

impl Overview {
    fn new(osd: &Rc<Osd>, view: Rc<OverviewView>, settings: Rc<Settings>) -> Rc<Self> {
        Rc::new(Overview {
            osd: osd.clone(),
            view,
            settings,
            windows: RefCell::new(Vec::new()),
            rows: RefCell::new(Vec::new()),
            selected: Cell::new(0),
            idle: RefCell::new(None),
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
        let mut list = windows::list();
        let workspaces = workspaces_in(&list);
        if workspaces.is_empty() {
            eprintln!("hypr-osd-overview: no workspaces to show");
            return;
        }
        // Which window you are in: `list` is most-recently-used first, so it is
        // the first one - read before the ordering below moves it.
        let current = list.first().map(|window| window.address.clone());
        let focused = windows::focused_workspace().unwrap_or_default();
        layout::order(&mut list, &focused);

        let output = monitors::focused_output().unwrap_or(FALLBACK_OUTPUT);
        let plan = layout::plan(&list, area(output, &self.settings), self.settings.limits);
        self.view
            .render(&plan, &list, &workspaces, &focused, &self.settings);
        self.osd.set_content(&self.view.root);
        self.osd.show();

        // The selection starts on the window you are in, so that opening the
        // card and pressing Enter does nothing at all - rather than jumping
        // somewhere because you pressed the key twice.
        let selected = current
            .and_then(|address| list.iter().position(|window| window.address == address))
            .unwrap_or(0);
        self.selected.set(selected);
        self.view.select(selected);

        *self.windows.borrow_mut() = list;
        if self.settings.thumbnails {
            self.capture(&plan, output.scale);
        }
        // Kept for the arrow keys: the rows the plan decided on are the rows
        // "up" and "down" walk, so the view and the keys cannot disagree about
        // where a row begins.
        *self.rows.borrow_mut() = plan.rows;
        self.arm_idle();
    }

    /// Take the pictures again - and with them the window list, which may well
    /// have changed while the card was up.
    fn refresh(self: &Rc<Self>) {
        self.open();
    }

    /// Switch to the selected window and take the card down.
    fn commit(self: &Rc<Self>) {
        let address = self
            .windows
            .borrow()
            .get(self.selected.get())
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

    fn close(&self) {
        if let Some(source) = self.idle.borrow_mut().take() {
            source.remove();
        }
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
    fn arm_idle(self: &Rc<Self>) {
        if let Some(source) = self.idle.borrow_mut().take() {
            source.remove();
        }
        let Some(duration) = self.settings.idle_close else {
            return;
        };
        let me = Rc::clone(self);
        let source = glib::timeout_add_local_once(duration, move || {
            if me.osd.is_visible() {
                me.close();
            }
        });
        *self.idle.borrow_mut() = Some(source);
    }

    /// Select a tile, if there is one.
    fn select(&self, index: usize) {
        if index >= self.windows.borrow().len() {
            return;
        }
        self.selected.set(index);
        self.view.select(index);
    }

    /// Walk the tiles, wrapping around either end.
    fn step(&self, delta: i32) {
        let count = self.windows.borrow().len() as i32;
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
    fn capture(self: &Rc<Self>, plan: &Plan, scale: f64) {
        let windows = self.windows.borrow().clone();
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
            let tally = tally.clone();
            thumbs::capture(window, scale, width, move |texture| {
                let (finished, captured) = tally.get();
                let captured = captured + usize::from(texture.is_some());
                tally.set((finished + 1, captured));
                if let Some(texture) = texture {
                    tile.set_picture(&texture);
                } else if finished + 1 == total && captured == 0 {
                    eprintln!(
                        "hypr-osd-overview: not one window could be captured - thumbnails \
                         need grim, and a compositor that can capture a single toplevel"
                    );
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
            view.on_workspace_click({
                let overview = overview.clone();
                move |name| overview.switch_workspace(name)
            });
            view.on_background_click({
                let overview = overview.clone();
                move || overview.close()
            });
            // Moving the mouse over the card is looking at it, and looking at it
            // is not being idle.
            view.on_motion({
                let overview = overview.clone();
                move || overview.arm_idle()
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
                    "status" => return Ok(status_lines()),
                    other => {
                        return Err(format!(
                            "unknown verb `{other}` (toggle | show | close | status)"
                        ))
                    }
                }
                Ok(String::new())
            },
        )
    };

    run(opts, build, handle)
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
/// about font metrics.
fn area(output: Output, settings: &Settings) -> Area {
    let horizontal = 2 * (CARD_PAD_X + view::PADDING);
    let vertical = 2 * (CARD_PAD_Y + view::PADDING) + view::STRIP + view::HINT + 2 * settings.gap;
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

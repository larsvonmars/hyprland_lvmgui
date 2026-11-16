//! The card's grid of windows.
//!
//! A grid rather than a row: with a dozen windows a row would be a strip across
//! the screen with unreadable labels, while a grid stays a block you can take in
//! at a glance. A tile is the application's icon over two lines of the window's
//! title, and the selected one wears the accent.
//!
//! The cards are a *view*: they draw a list and report which one was clicked.
//! Choosing and switching is the element's business (see `main.rs`), which is
//! why nothing here knows about windows beyond what to draw.
//!
//! # By finger
//!
//! A finger chooses by tapping a tile - the same click a mouse makes - and the
//! hint under the grid says so when the machine is a tablet, because the keyboard
//! line it says on a laptop ("Tab moves · Enter switches · Esc cancels") is a
//! description of keys that are not there. The view also counts the presses that
//! are down on the card ([`SwitcherView::is_pressed`]): the element's idle-commit
//! watchdog asks before it fires, so a card cannot switch windows while a finger
//! is still on it. Counting is done by a *legacy* controller on the root, in the
//! capture phase, for the one property that matters here - it observes every
//! press without claiming it, so the tiles' own clicks are untouched (a
//! `GestureClick` on the root would race the button underneath and could eat the
//! tap).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::windows::Window;
use hypr_osd_core::{icons, text};

use crate::Settings;

/// Gap between tiles in pixels. `Settings::width` computes the card's width from
/// the same number, so the grid always fits what it is handed.
pub const GAP: i32 = 8;

/// The hint under the grid on a machine with a pointer: the switcher is the only
/// element here whose keys are not obvious from looking at it.
pub const HINT_POINTER: &str = "Tab moves · Enter or releasing Alt switches · Esc cancels";

/// And the hint on a tablet, where there is no Tab, no Enter and no Alt - only a
/// finger, and the way out the shell's tap catcher provides.
pub const HINT_TOUCH: &str = "Tap a window to switch · tap outside to cancel";

/// What a click on a tile does, with the tile's index.
type ClickHandler = Rc<dyn Fn(usize)>;

pub struct SwitcherView {
    /// The card's content, handed to the shell.
    pub root: gtk::Box,
    grid: gtk::Grid,
    tiles: RefCell<Vec<Rc<Tile>>>,
    hint: gtk::Label,
    /// How many mouse buttons or fingers are down on the card right now. The
    /// element asks before its idle watchdog commits, so a card that is being
    /// touched does not switch windows underneath the touch.
    pressed: Rc<Cell<i32>>,
    /// What a click on a tile should do, installed once by the element.
    on_click: RefCell<Option<ClickHandler>>,
}

struct Tile {
    button: gtk::Button,
}

impl SwitcherView {
    pub fn new() -> Self {
        let grid = gtk::Grid::new();
        grid.set_column_spacing(GAP as u32);
        grid.set_row_spacing(GAP as u32);
        grid.set_halign(gtk::Align::Center);
        grid.set_valign(gtk::Align::Center);

        // One line, faint: the switcher is the only element here whose keys are
        // not obvious from looking at it, so it says them. It costs nothing
        // after the first week. Which line it is depends on the machine - see
        // `set_hint`.
        let hint = gtk::Label::new(Some(HINT_POINTER));
        hint.add_css_class("hint");

        let root = gtk::Box::new(gtk::Orientation::Vertical, GAP * 2);
        root.append(&grid);
        root.append(&hint);

        // Count the presses on the card without taking them from the tiles: a
        // legacy controller in the capture phase sees the press first and is
        // free to pass it on, which is exactly what the idle watchdog needs.
        // (A `GestureClick` here would claim the sequence before the tile's
        // button could, and a tap would stop switching.)
        let pressed = Rc::new(Cell::new(0_i32));
        let counter = pressed.clone();
        let events = gtk::EventControllerLegacy::new();
        events.set_propagation_phase(gtk::PropagationPhase::Capture);
        events.connect_event(move |_, event| {
            // Both spellings of "down": GDK reports a finger as a touch event and
            // (when no gesture claimed it) as an emulated button event, and which
            // of the two a legacy controller sees is not something to bet the
            // watchdog on - so the count follows either, and a tap that arrives as
            // both counts up and down twice.
            match event.event_type() {
                gdk::EventType::ButtonPress | gdk::EventType::TouchBegin => {
                    counter.set(counter.get() + 1)
                }
                gdk::EventType::ButtonRelease
                | gdk::EventType::TouchEnd
                | gdk::EventType::TouchCancel => counter.set((counter.get() - 1).max(0)),
                // A grab that went away (a compositor-side cancel, a drag out of
                // the surface) releases no button here; the count would be
                // stuck above zero and the watchdog would never fire again.
                gdk::EventType::GrabBroken => counter.set(0),
                _ => {}
            }
            glib::Propagation::Proceed
        });
        root.add_controller(events);

        SwitcherView {
            root,
            grid,
            tiles: RefCell::new(Vec::new()),
            hint,
            pressed,
            on_click: RefCell::new(None),
        }
    }

    /// Say what a finger (or a pointer) can do with the card.
    ///
    /// Set when the card opens, and again when the machine is folded or unfolded
    /// under it: a switcher opened by a gesture on a tablet must not go on
    /// describing Tab and Escape.
    pub fn set_hint(&self, tablet: bool) {
        self.hint
            .set_text(if tablet { HINT_TOUCH } else { HINT_POINTER });
    }

    /// Whether a finger or a button is down on the card right now.
    ///
    /// The idle-commit watchdog asks this rather than its own timer: a touch that
    /// is still down is a user who has not finished, and a card that commits
    /// (and switches windows) while they are still aiming is the one thing this
    /// card must not do.
    pub fn is_pressed(&self) -> bool {
        self.pressed.get() > 0
    }

    /// What to do when a tile is clicked, with the tile's index.
    pub fn on_click(&self, handler: impl Fn(usize) + 'static) {
        *self.on_click.borrow_mut() = Some(Rc::new(handler));
    }

    /// Draw `list`. Called once per time the card opens: the list cannot change
    /// while the card holds the keyboard, so walking it never rebuilds anything.
    pub fn render(&self, list: &[Window], settings: &Settings) {
        self.clear();
        let mut tiles = self.tiles.borrow_mut();
        for (index, window) in list.iter().enumerate() {
            let tile = Rc::new(Tile::new(window, settings));
            if let Some(on_click) = self.on_click.borrow().clone() {
                tile.button.connect_clicked(move |_| on_click(index));
            }
            let column = index as i32 % settings.columns;
            let row = index as i32 / settings.columns;
            self.grid.attach(&tile.button, column, row, 1, 1);
            tiles.push(tile);
        }
        drop(tiles);
        // With a single window there is nothing to walk, and the keys would only
        // be noise.
        self.hint.set_visible(list.len() > 1);
    }

    /// Put the accent on one tile.
    pub fn select(&self, index: usize) {
        for (position, tile) in self.tiles.borrow().iter().enumerate() {
            if position == index {
                tile.button.add_css_class("selected");
            } else {
                tile.button.remove_css_class("selected");
            }
        }
    }

    fn clear(&self) {
        for tile in self.tiles.borrow_mut().drain(..) {
            self.grid.remove(&tile.button);
        }
    }
}

impl Tile {
    fn new(window: &Window, settings: &Settings) -> Self {
        let size = settings.icon_size;

        let icon = gtk::Image::new();
        icon.set_pixel_size(size);

        // What is drawn when the theme has no icon for the application: its
        // first letter, in the tile's own shape. Better than a generic glyph -
        // it still says *which* application this is.
        let letter = gtk::Label::new(Some(&text::initial(&window.class)));
        letter.add_css_class("icon-letter");
        letter.set_size_request(size, size);

        let art = gtk::Stack::new();
        art.add_named(&icon, Some("icon"));
        art.add_named(&letter, Some("letter"));
        art.set_visible_child_name("letter");
        if let Some(paintable) = icons::paintable(&window.class, size) {
            icon.set_paintable(Some(&paintable));
            art.set_visible_child_name("icon");
        }

        let title = gtk::Label::new(Some(window.label()));
        title.add_css_class("tile-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_justify(gtk::Justification::Center);
        title.set_lines(2);
        title.set_wrap(true);
        title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        text::cap_width(&title, settings.tile_width - 12);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&art);
        content.append(&title);

        let button = gtk::Button::new();
        button.add_css_class("tile");
        button.set_child(Some(&content));
        button.set_size_request(settings.tile_width, -1);
        button.set_tooltip_text(Some(&window.describe()));

        Tile { button }
    }
}

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

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

use crate::icons;
use crate::windows::Window;
use crate::Settings;

/// Gap between tiles in pixels. `Settings::width` computes the card's width from
/// the same number, so the grid always fits what it is handed.
pub const GAP: i32 = 8;

/// What a click on a tile does, with the tile's index.
type ClickHandler = Rc<dyn Fn(usize)>;

pub struct SwitcherView {
    /// The card's content, handed to the shell.
    pub root: gtk::Box,
    grid: gtk::Grid,
    tiles: RefCell<Vec<Rc<Tile>>>,
    hint: gtk::Label,
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
        // after the first week.
        let hint = gtk::Label::new(Some(
            "Tab moves · Enter or releasing Alt switches · Esc cancels",
        ));
        hint.add_css_class("hint");

        let root = gtk::Box::new(gtk::Orientation::Vertical, GAP * 2);
        root.append(&grid);
        root.append(&hint);

        SwitcherView {
            root,
            grid,
            tiles: RefCell::new(Vec::new()),
            hint,
            on_click: RefCell::new(None),
        }
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
        let letter = gtk::Label::new(Some(&initial(&window.class)));
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
        cap_width(&title, settings.tile_width - 12);

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

/// The first letter of a class, for the tile that stands in for an icon.
fn initial(class: &str) -> String {
    class
        .chars()
        .find(|character| character.is_alphanumeric())
        .map(|character| character.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string())
}

/// Cap a label so that it cannot widen the card it sits in.
///
/// An ellipsised label still reports its *full* text as its natural width, so
/// without this one long window title would decide how wide the whole card is.
/// The media card learned the same lesson for the same reason.
fn cap_width(label: &gtk::Label, pixels: i32) {
    const SAMPLE: &str = "0000000000";
    let (sample_width, _) = label.create_pango_layout(Some(SAMPLE)).pixel_size();
    let char_width = f64::from(sample_width) / SAMPLE.chars().count() as f64;
    if char_width <= 0.0 {
        return;
    }
    label.set_max_width_chars((f64::from(pixels) / char_width).floor().max(1.0) as i32);
}

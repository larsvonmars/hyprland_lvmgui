//! The panel: a search field, a sidebar of drawers, and a grid of applications.
//!
//! A *view*: it draws the rows and tiles it is handed and reports which one was
//! clicked; which drawer is selected, what matches and what happens on Enter are
//! the element's business (`main.rs`). Nothing here knows what an application is.
//!
//! Two things about the grid are deliberate:
//!
//! * **Tiles are built once and then re-attached.** The grid reflows on every
//!   keystroke - the filter narrows, so the same tiles sit in different places -
//!   and building two hundred widget trees per key would be work for nothing: the
//!   icon, the name and the pin mark of an application do not change when the
//!   query does. So a tile is made the first time its application is shown, and
//!   afterwards only moved, with `Grid::remove` and `Grid::attach` on a widget
//!   that already exists.
//! * **The grid lives in a fixed-size scroller.** A mapped layer surface grows
//!   with its content but does not shrink with it (measured - the launcher hit the
//!   same wall), so a grid that sized itself to its tiles would leave the panel as
//!   tall as the largest drawer of the session. A pinned scroller makes the panel
//!   one size, always - which is also why picking a drawer does not move it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use hypr_osd_core::text;

use crate::Settings;

/// What separates the two panes of the body. Public because the element works the
/// tile pane's width out from it (`Settings::grid_width`), and the two must not be
/// able to disagree.
pub const BODY_GAP: i32 = 10;

/// The keys, said once. The element restores this line after having replaced it
/// with an answer ("nothing here matches that").
pub const KEYS_HINT: &str = "\u{21b5} open \u{b7} arrows walk \u{b7} tab changes drawer \u{b7} \
     right-click pins \u{b7} esc close";

/// One entry in the sidebar: a drawer, how many applications are in it, and
/// whether it is the one being shown.
pub struct Row {
    pub label: String,
    pub count: usize,
    pub selected: bool,
}

/// One application in the grid.
///
/// `key` is what makes the tile cache work: the element's index into its own
/// application list, so a tile can be found again after the grid reflows.
pub struct Tile {
    pub key: usize,
    pub icon: Option<gdk::Paintable>,
    /// The letter that stands in for an application the icon theme has nothing
    /// for - the same stand-in the switcher and the media card use.
    pub glyph: String,
    pub label: String,
    pub pinned: bool,
}

/// The widgets of one tile, kept between renders (see the module comment).
struct TileWidgets {
    button: gtk::Button,
    pin: gtk::Label,
    pinned: Cell<bool>,
    /// Where the tile sits in the *current* rendering. The click handler is
    /// connected once, when the tile is built, so this is how it learns which row
    /// it is on now - a plain `Cell` rather than the widgets themselves, because a
    /// closure owning the tile it is connected to would be a reference cycle.
    position: Rc<Cell<usize>>,
}

impl TileWidgets {
    fn set_pinned(&self, pinned: bool) {
        if self.pinned.get() != pinned {
            self.pinned.set(pinned);
            self.pin.set_visible(pinned);
        }
    }
}

type ClickHandler = Rc<dyn Fn(usize)>;
type ChangeHandler = Rc<dyn Fn(&str)>;
type SignalHandler = Rc<dyn Fn()>;

pub struct AppsView {
    /// The element hands this to the shell as the card's content.
    pub root: gtk::Box,
    /// The field you type into. Public because the element puts the keyboard in
    /// it when the panel opens.
    pub entry: gtk::Entry,
    sidebar: gtk::Box,
    grid: gtk::Grid,
    scroller: gtk::ScrolledWindow,
    hint: gtk::Label,
    /// One tile per application, keyed by the element's application index.
    tiles: RefCell<HashMap<usize, Rc<TileWidgets>>>,
    /// The keys currently attached to the grid, in visual order - so a reflow
    /// knows what to take out, and a position can be turned into a tile.
    attached: RefCell<Vec<usize>>,
    /// The sidebar buttons currently drawn, so a click can be reported by index.
    drawer_buttons: RefCell<Vec<gtk::Button>>,
    /// The numbers the element and the view must agree about, taken from one
    /// `Settings` so they cannot drift.
    columns: i32,
    tile_width: i32,
    tile_height: i32,
    gap: i32,
    icon_size: i32,
    sidebar_width: i32,
    on_query: Rc<RefCell<Option<ChangeHandler>>>,
    on_drawer: Rc<RefCell<Option<ClickHandler>>>,
    on_tile: Rc<RefCell<Option<ClickHandler>>>,
    on_pin: Rc<RefCell<Option<ClickHandler>>>,
    on_motion: Rc<RefCell<Option<SignalHandler>>>,
}

impl AppsView {
    /// Build the widgets from the same settings the element laid the card out
    /// with, so the two cannot disagree about how wide the sidebar is.
    pub fn new(settings: &Settings) -> Self {
        let search_glyph = gtk::Label::new(Some("\u{f002}"));
        search_glyph.add_css_class("search-glyph");

        let entry = gtk::Entry::new();
        entry.add_css_class("search");
        entry.set_placeholder_text(Some("Search applications\u{2026}"));
        entry.set_hexpand(true);

        let search_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        search_row.add_css_class("search-row");
        search_row.append(&search_glyph);
        search_row.append(&entry);

        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 2);
        sidebar.add_css_class("sidebar");
        sidebar.set_size_request(settings.sidebar_width, -1);
        sidebar.set_valign(gtk::Align::Start);

        let grid = gtk::Grid::new();
        grid.add_css_class("grid");
        grid.set_column_spacing(settings.gap as u32);
        grid.set_row_spacing(settings.gap as u32);
        // A grid that is given room hands the slack to its children; these tiles
        // are the size the layout was built around, so the grid keeps its natural
        // size and the pane decides where it sits.
        grid.set_halign(gtk::Align::Start);
        grid.set_valign(gtk::Align::Start);

        let scroller = gtk::ScrolledWindow::new();
        scroller.add_css_class("tiles");
        scroller.set_child(Some(&grid));
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        // The pinned size, and *not* the child's natural height: this is what keeps
        // the panel one size whatever the drawer holds (see the module comment).
        scroller.set_size_request(settings.grid_width(), settings.grid_height);
        scroller.set_propagate_natural_height(false);

        let body = gtk::Box::new(gtk::Orientation::Horizontal, BODY_GAP);
        body.add_css_class("body");
        body.append(&sidebar);
        body.append(&scroller);

        let hint = gtk::Label::new(Some(KEYS_HINT));
        hint.add_css_class("hint");
        hint.set_xalign(0.0);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("apps-panel");
        root.append(&search_row);
        root.append(&body);
        root.append(&hint);

        let view = AppsView {
            root,
            entry,
            sidebar,
            grid,
            scroller,
            hint,
            tiles: RefCell::new(HashMap::new()),
            attached: RefCell::new(Vec::new()),
            drawer_buttons: RefCell::new(Vec::new()),
            columns: settings.columns(),
            tile_width: settings.tile_width,
            tile_height: settings.tile_height,
            gap: settings.gap,
            icon_size: settings.icon_size,
            sidebar_width: settings.sidebar_width,
            on_query: Rc::new(RefCell::new(None)),
            on_drawer: Rc::new(RefCell::new(None)),
            on_tile: Rc::new(RefCell::new(None)),
            on_pin: Rc::new(RefCell::new(None)),
            on_motion: Rc::new(RefCell::new(None)),
        };
        view.install_signals();
        view
    }

    /// Wire the widget signals to whatever the element installs with the `on_*`
    /// methods below; the two halves meet through the shared cells, so the order
    /// does not matter.
    fn install_signals(&self) {
        let changed = self.on_query.clone();
        self.entry.connect_changed(move |entry| {
            if let Some(handler) = changed.borrow().as_ref() {
                handler(entry.text().as_str());
            }
        });

        let motion_handler = self.on_motion.clone();
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(move |_, _, _| {
            if let Some(handler) = motion_handler.borrow().as_ref() {
                handler();
            }
        });
        self.root.add_controller(motion);
    }

    /// What to do when the query changes (the element filters and redraws).
    pub fn on_query(&self, handler: impl Fn(&str) + 'static) {
        *self.on_query.borrow_mut() = Some(Rc::new(handler));
    }

    /// What to do when a drawer is clicked, with its index in the sidebar.
    pub fn on_drawer(&self, handler: impl Fn(usize) + 'static) {
        *self.on_drawer.borrow_mut() = Some(Rc::new(handler));
    }

    /// What to do when a tile is clicked, with its position in the grid.
    pub fn on_tile(&self, handler: impl Fn(usize) + 'static) {
        *self.on_tile.borrow_mut() = Some(Rc::new(handler));
    }

    /// What to do when a tile is right-clicked - the same position.
    pub fn on_pin(&self, handler: impl Fn(usize) + 'static) {
        *self.on_pin.borrow_mut() = Some(Rc::new(handler));
    }

    /// What to do when the pointer moves over the panel (the element counts that
    /// as "not idle").
    pub fn on_motion(&self, handler: impl Fn() + 'static) {
        *self.on_motion.borrow_mut() = Some(Rc::new(handler));
    }

    /// How many tiles fit in a row - the number the element's arrow keys need.
    pub fn columns(&self) -> i32 {
        self.columns
    }

    /// How many tiles one page of the grid holds: what Page Up/Down walk.
    pub fn page(&self) -> i32 {
        let height = if self.scroller.height() > 0 {
            self.scroller.height()
        } else {
            self.tile_height * 4
        };
        let stride = (self.tile_height + self.gap).max(1);
        (height / stride).max(1) * self.columns
    }

    /// Draw the sidebar.
    pub fn render_sidebar(&self, rows: &[Row]) {
        for button in self.drawer_buttons.borrow_mut().drain(..) {
            self.sidebar.remove(&button);
        }
        let mut buttons = self.drawer_buttons.borrow_mut();
        for (index, row) in rows.iter().enumerate() {
            let button = gtk::Button::new();
            button.add_css_class("drawer");
            button.set_has_frame(false);
            button.set_focus_on_click(false);
            button.set_can_focus(false);
            if row.selected {
                button.add_css_class("selected");
            }

            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let name = gtk::Label::new(Some(&row.label));
            name.add_css_class("drawer-name");
            name.set_xalign(0.0);
            name.set_hexpand(true);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            // The sidebar is a fixed width, so the name is capped to what fits
            // beside the count - an ellipsised label still asks for its full width
            // otherwise.
            text::cap_width(&name, self.drawer_name_width());
            let count = gtk::Label::new(Some(&row.count.to_string()));
            count.add_css_class("drawer-count");
            content.append(&name);
            content.append(&count);
            button.set_child(Some(&content));

            if let Some(handler) = self.on_drawer.borrow().clone() {
                button.connect_clicked(move |_| handler(index));
            }
            self.sidebar.append(&button);
            buttons.push(button);
        }
    }

    /// How wide a drawer's name may be: the sidebar, less its own padding and the
    /// count that sits at its right end.
    fn drawer_name_width(&self) -> i32 {
        (self.sidebar_width - 56).max(40)
    }

    /// Draw the tiles, with `selected` wearing the accent.
    ///
    /// Every tile in `tiles` ends up attached to the grid in order; the ones that
    /// were attached before and are not in the list are taken out again.
    pub fn render_tiles(&self, tiles: &[Tile], selected: usize) {
        let mut attached = self.attached.borrow_mut();
        for key in attached.drain(..) {
            if let Some(widgets) = self.tiles.borrow().get(&key) {
                self.grid.remove(&widgets.button);
            }
        }

        for (position, spec) in tiles.iter().enumerate() {
            // Looked up and let go of in one statement: a tile that does not exist
            // yet is built *below*, and building it writes to the same map - which
            // a borrow still alive from the lookup would turn into a panic.
            let existing = self.tiles.borrow().get(&spec.key).cloned();
            let widgets = match existing {
                Some(widgets) => widgets,
                None => {
                    let widgets = Rc::new(self.tile_widgets(spec));
                    self.tiles.borrow_mut().insert(spec.key, widgets.clone());
                    widgets
                }
            };
            widgets.set_pinned(spec.pinned);
            widgets.position.set(position);
            self.grid.attach(
                &widgets.button,
                position as i32 % self.columns,
                position as i32 / self.columns,
                1,
                1,
            );
            attached.push(spec.key);
        }
        drop(attached);

        self.select(selected);
    }

    /// Put the accent on one tile and make sure it is on screen.
    ///
    /// Always re-applies rather than trusting the last selection, because the
    /// tiles are reused: after a reflow the tile that wore the accent may be a
    /// different application's.
    pub fn select(&self, index: usize) {
        let count = {
            let attached = self.attached.borrow();
            let tiles = self.tiles.borrow();
            for (position, key) in attached.iter().enumerate() {
                let Some(widgets) = tiles.get(key) else {
                    continue;
                };
                if position == index {
                    widgets.button.add_css_class("selected");
                } else {
                    widgets.button.remove_css_class("selected");
                }
            }
            attached.len()
        };
        if index < count {
            self.scroll_to(index);
        }
    }

    /// Move the scroller so the tile at `index` is visible, moving as little as
    /// possible - a tile that is already on screen must not jump the list around.
    fn scroll_to(&self, index: usize) {
        let row = (index as i32 / self.columns).max(0);
        let stride = f64::from(self.tile_height + self.gap);
        let top = f64::from(row) * stride;
        let bottom = top + f64::from(self.tile_height);
        let adjustment = self.scroller.vadjustment();
        let value = adjustment.value();
        let page = adjustment.page_size();
        if top < value {
            adjustment.set_value(top);
        } else if bottom > value + page {
            adjustment.set_value(bottom - page);
        }
    }

    /// The sizes the panel actually came out at: the content widget, and the two
    /// panes inside it. `status` prints them, which is how a width that disagrees
    /// with the plan - a label that would not be capped, a tile that asked for
    /// more than it was given - is found without measuring a screenshot.
    pub fn measured(&self) -> String {
        if self.root.width() == 0 {
            return "not drawn yet".to_string();
        }
        format!(
            "content {}x{}, sidebar {}, tiles pane {}",
            self.root.width(),
            self.root.height(),
            self.sidebar.width(),
            self.scroller.width(),
        )
    }

    /// Put the keyboard into the field. Called when the panel opens, so that the
    /// first keystroke is the first character of the query and not something that
    /// had to be clicked first.
    pub fn focus_entry(&self) {
        self.entry.grab_focus();
    }

    /// Replace the query (`""` on opening), which repaints through the change
    /// handler like any other edit.
    pub fn set_query(&self, text: &str) {
        self.entry.set_text(text);
        self.entry.set_position(-1);
    }

    /// Say something under the grid instead of the keys - the element uses it for
    /// "nothing here matches that", the one state the keys cannot explain.
    pub fn set_hint(&self, text: &str) {
        self.hint.set_label(text);
    }

    /// One tile: the icon (or its letter), the name, and a pin mark when it is
    /// pinned.
    fn tile_widgets(&self, spec: &Tile) -> TileWidgets {
        let button = gtk::Button::new();
        button.add_css_class("tile");
        button.set_has_frame(false);
        button.set_focus_on_click(false);
        button.set_can_focus(false);
        button.set_size_request(self.tile_width, self.tile_height);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
        content.add_css_class("tile-content");
        content.set_valign(gtk::Align::Center);
        content.append(&self.tile_icon(spec));

        let label = gtk::Label::new(None);
        label.add_css_class("tile-label");
        label.set_justify(gtk::Justification::Center);
        label.set_lines(2);
        // The name is broken into its two lines *here* rather than left to Pango's
        // wrapping, which would measure the tile from the whole unwrapped name -
        // and with it the grid, the scroller and the card (see `text::two_lines`).
        // `cap_width` does two things at once: it caps the label (so a long line
        // cannot widen the tile either way) and answers with how many characters
        // fit, which is what the break needs to know.
        let capacity = text::cap_width(&label, self.tile_width - 10);
        let capacity = if capacity > 4 { capacity as usize } else { 12 };
        label.set_text(&text::two_lines(&spec.label, capacity));
        content.append(&label);

        // The pin mark rides in the corner of the tile; an application that is not
        // pinned does not draw it, so the tile is otherwise the same either way.
        let pin = gtk::Label::new(Some("\u{f08d}"));
        pin.add_css_class("pin");
        pin.set_halign(gtk::Align::End);
        pin.set_valign(gtk::Align::Start);
        pin.set_visible(spec.pinned);

        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&content));
        overlay.add_overlay(&pin);
        button.set_child(Some(&overlay));

        // Connected once, here, reading where the tile *is* from a shared cell:
        // a reflow then moves the tile without stacking a second handler on it,
        // and a click still reports the row it is on now.
        let position = Rc::new(Cell::new(0));
        {
            let on_tile = self.on_tile.clone();
            let position = position.clone();
            button.connect_clicked(move |_| {
                if let Some(handler) = on_tile.borrow().as_ref() {
                    handler(position.get());
                }
            });
        }
        {
            let on_pin = self.on_pin.clone();
            let position = position.clone();
            let gesture = gtk::GestureClick::new();
            gesture.set_button(gdk::BUTTON_SECONDARY);
            gesture.connect_pressed(move |_, _, _, _| {
                if let Some(handler) = on_pin.borrow().as_ref() {
                    handler(position.get());
                }
            });
            button.add_controller(gesture);
        }

        TileWidgets {
            button,
            pin,
            pinned: Cell::new(spec.pinned),
            position,
        }
    }

    /// The icon, or the letter that stands in for it, at a fixed size so every
    /// tile's name starts at the same height - exactly the size the layout was
    /// built around.
    fn tile_icon(&self, spec: &Tile) -> gtk::Widget {
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        holder.set_size_request(self.icon_size, self.icon_size);
        holder.set_halign(gtk::Align::Center);

        match &spec.icon {
            Some(paintable) => {
                let image = gtk::Image::new();
                image.set_paintable(Some(paintable));
                image.set_pixel_size(self.icon_size);
                holder.append(&image);
            }
            None => {
                let label = gtk::Label::new(Some(&spec.glyph));
                label.add_css_class("tile-glyph");
                holder.append(&label);
            }
        }
        holder.upcast()
    }
}

//! The card: a search field over a list of results.
//!
//! A *view*: it draws the rows it is handed and reports which one was clicked,
//! which one is selected is the element's business (`main.rs`), and nothing here
//! knows what an application or a file is. That is what keeps the row drawing in
//! one place while the element decides what a row *means*.
//!
//! The list is rebuilt on every keystroke rather than filtered in place: at most
//! a handful of rows exist at a time (the element caps the result list), and a
//! rebuilt list cannot show a stale row, which is the bug a diffing list always
//! eventually grows.
//!
//! It is also **padded to a fixed number of rows**. That is not decoration: a
//! mapped layer surface grows with its content but does not shrink with it, so a
//! card that sized itself to the results would stay as tall as the widest search
//! of the session (typing one letter into an empty field and then narrowing it to
//! one result left a card 566 px tall and one row of text - measured). A fixed
//! list height gives the card a stable size, which is also calmer to type into:
//! the thing you are reading does not slide up and down under the pointer.
//!
//! The padded list sits in a **scroller** for the same reason, one step further:
//! the rows are taller on a tablet (the `.touch` rules in `launcher.css`), and a
//! finger scrolls a list by dragging it. The viewport keeps the height the list
//! had when it was first painted (`calibrate`), so the card stays the size it
//! always is and the taller rows scroll instead of stretching it off the screen;
//! the arrow keys move the selection *and* the scroller (`scroll_to`), which is
//! what keeps keyboard navigation working across that change.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use hypr_osd_core::icons::{self, names};
use hypr_osd_core::{text, CARD_PAD_X};

/// How large the search symbol is drawn, and what stands in for a row the icon
/// theme has nothing for.
const SEARCH_ICON: i32 = 16;

/// What a row draws when the icon theme has nothing for it.
///
/// Two kinds, because the two kinds of row are answered differently: an
/// application stands on its first letter - which still says *which* one it is,
/// and no generic mark can - while a file gets the Lucide mark for what it is,
/// which is always there and needs no theme.
pub enum Stand {
    Letter(String),
    Icon(&'static str),
}

/// One drawn result.
pub struct Row {
    /// The application's (or file type's) icon, when the theme has one.
    pub icon: Option<gdk::Paintable>,
    /// What to draw instead when there is no icon.
    pub glyph: Stand,
    pub title: String,
    /// What sits under the title: a generic name, or a file's directory.
    pub subtitle: String,
}

type ClickHandler = Rc<dyn Fn(usize)>;
type ChangeHandler = Rc<dyn Fn(&str)>;
type SignalHandler = Rc<dyn Fn()>;

pub struct LauncherView {
    /// The element hands this to the shell as the card's content.
    pub root: gtk::Box,
    /// The field you type into. Public because the element has to put the
    /// keyboard into it when the card opens.
    pub entry: gtk::Entry,
    list: gtk::Box,
    /// The list's viewport: the height is pinned by [`LauncherView::calibrate`]
    /// so the card keeps its size while the rows under a finger get taller.
    scroller: gtk::ScrolledWindow,
    /// The height the viewport was pinned to (`0` until it has been measured).
    /// Shared, because the one retry after a first frame has to write it too.
    list_height: Rc<Cell<i32>>,
    empty: gtk::Label,
    rows: RefCell<Vec<gtk::Button>>,
    /// How wide a row's text may be before it is ellipsised. An ellipsised label
    /// still reports its full text as its natural width, so without a cap one
    /// long file name would decide how wide the whole card is.
    title_width: i32,
    icon_size: i32,
    /// How many rows the list always holds, results or not (see the module
    /// comment). It is the element's `max_results`.
    capacity: usize,
    // The handler cells are shared with the widget signals installed once in
    // `new`, which is why they are `Rc`s rather than plain fields.
    on_click: Rc<RefCell<Option<ClickHandler>>>,
    on_changed: Rc<RefCell<Option<ChangeHandler>>>,
    on_activate: Rc<RefCell<Option<SignalHandler>>>,
    /// What counts as "not idle on this card": the pointer moving over it, and -
    /// on a touchscreen, where a pointer does not move - a tap or a scroll.
    on_activity: Rc<RefCell<Option<SignalHandler>>>,
}

impl LauncherView {
    /// Build the widgets. `width` is the card's own width (`Opts::width`), which
    /// is what the text caps are derived from so the two cannot drift, and
    /// `capacity` is how many rows the list reserves room for.
    pub fn new(width: i32, icon_size: i32, capacity: usize) -> Self {
        // A Lucide search symbol rather than the icon theme's: it is the field's
        // own furniture and it should look the same as the one in the
        // applications panel, whatever theme the desktop wears.
        let search = icons::lucide(names::SEARCH, SEARCH_ICON);
        search.add_css_class("search-icon");
        search.set_valign(gtk::Align::Center);

        let entry = gtk::Entry::new();
        entry.add_css_class("search");
        entry.set_placeholder_text(Some("Search applications and files\u{2026}"));
        entry.set_hexpand(true);

        let search_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        search_row.add_css_class("search-row");
        search_row.append(&search);
        search_row.append(&entry);

        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        list.add_css_class("results");
        list.set_halign(gtk::Align::Fill);

        // The viewport the results scroll in. Its height is *not* the list's
        // natural height at every instant - it is the padded list's height when
        // it was first painted (see the module comment and `calibrate`).
        let scroller = gtk::ScrolledWindow::new();
        scroller.add_css_class("results-view");
        scroller.set_child(Some(&list));
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_propagate_natural_height(false);
        // A touchscreen scrolls by dragging the list; the fling after the lift is
        // the only scrolling there is, so it has to be kinetic.
        scroller.set_kinetic_scrolling(true);

        let empty = gtk::Label::new(Some("No matches"));
        empty.add_css_class("empty");
        empty.set_visible(false);

        let hint = gtk::Label::new(Some(
            "\u{21b5} open \u{b7} \u{2191}\u{2193} move \u{b7} esc close",
        ));
        hint.add_css_class("hint");
        hint.set_xalign(0.0);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("launcher");
        root.append(&search_row);
        root.append(&scroller);
        root.append(&empty);
        root.append(&hint);

        // The icon, the gap after it and the row's own padding, all of which the
        // title shares the width with.
        let title_width = (width - 2 * CARD_PAD_X - icon_size - 12 - 16).max(80);

        let view = LauncherView {
            root,
            entry,
            list,
            scroller,
            list_height: Rc::new(Cell::new(0)),
            empty,
            rows: RefCell::new(Vec::new()),
            title_width,
            icon_size,
            capacity,
            on_click: Rc::new(RefCell::new(None)),
            on_changed: Rc::new(RefCell::new(None)),
            on_activate: Rc::new(RefCell::new(None)),
            on_activity: Rc::new(RefCell::new(None)),
        };
        view.install_signals();
        view
    }

    /// Wire the widget signals to whatever the element installs with the `on_*`
    /// methods below.
    ///
    /// The two halves meet through the shared `Rc<RefCell<Option<…>>>` cells, so
    /// the order does not matter: an element may install its handlers before or
    /// after the card is built, which is what lets `run` call the element's
    /// `build` whenever GTK is ready.
    fn install_signals(&self) {
        let changed = self.on_changed.clone();
        self.entry.connect_changed(move |entry| {
            if let Some(handler) = changed.borrow().as_ref() {
                handler(entry.text().as_str());
            }
        });

        let activated = self.on_activate.clone();
        self.entry.connect_activate(move |_| {
            if let Some(handler) = activated.borrow().as_ref() {
                handler();
            }
        });

        // Pointer movement over the card is the element's sign of life (it
        // re-arms its idle timer), the same signal the overview watches for.
        let motion_handler = self.on_activity.clone();
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(move |_, _, _| {
            if let Some(handler) = motion_handler.borrow().as_ref() {
                handler();
            }
        });
        self.root.add_controller(motion);

        // A finger has no pointer to move, so the two signals a touchscreen
        // gives instead are a press and a scroll. Both are *observed* and let
        // through: the rows' own clicks and the list's drag-to-scroll still do
        // their work, this only counts them as activity. The press gesture runs
        // in the bubble phase, like the one `Osd::stay_open_while_hovered` puts
        // on a card's content: a button-0 click gesture on an ancestor does not
        // steal a child button's press, it only sees it.
        let press_handler = self.on_activity.clone();
        let press = gtk::GestureClick::new();
        // Button 0 is every button: a row click, a long press and a two-finger
        // scroll's touch all count.
        press.set_button(0);
        press.connect_pressed(move |_, _, _, _| {
            if let Some(handler) = press_handler.borrow().as_ref() {
                handler();
            }
        });
        self.root.add_controller(press);

        let scroll_handler = self.on_activity.clone();
        let scroll = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::HORIZONTAL,
        );
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        scroll.connect_scroll(move |_, _, _| {
            if let Some(handler) = scroll_handler.borrow().as_ref() {
                handler();
            }
            gtk::glib::Propagation::Proceed
        });
        self.root.add_controller(scroll);
    }

    /// What to do when the query changes (the element searches and redraws).
    pub fn on_changed(&self, handler: impl Fn(&str) + 'static) {
        *self.on_changed.borrow_mut() = Some(Rc::new(handler));
    }

    /// What to do when the field is activated - Enter reaches the element
    /// through the key controller too, so this is the safety net for a Return
    /// that got past it (an IME commit, a default widget).
    pub fn on_activate(&self, handler: impl Fn() + 'static) {
        *self.on_activate.borrow_mut() = Some(Rc::new(handler));
    }

    /// What to do when a row is clicked, with the row's index.
    pub fn on_click(&self, handler: impl Fn(usize) + 'static) {
        *self.on_click.borrow_mut() = Some(Rc::new(handler));
    }

    /// What to do when the card is *used*: the pointer moves over it, a finger
    /// taps it, a wheel or the list scrolls under it. The element counts every
    /// one of those as "not idle", because a card that closed itself while
    /// somebody was reading it - which is what motion-only counting did on a
    /// touchscreen - is the bug this exists to prevent.
    pub fn on_activity(&self, handler: impl Fn() + 'static) {
        *self.on_activity.borrow_mut() = Some(Rc::new(handler));
    }

    /// Draw `rows`, with `selected` wearing the accent, and fill the rest of the
    /// list with empty rows so the card keeps its size.
    pub fn render(&self, rows: &[Row], selected: usize) {
        self.clear();
        let mut buttons = self.rows.borrow_mut();
        for (index, row) in rows.iter().take(self.capacity).enumerate() {
            let button = self.row_button(row);
            if let Some(on_click) = self.on_click.borrow().clone() {
                button.connect_clicked(move |_| on_click(index));
            }
            self.list.append(&button);
            buttons.push(button);
        }
        for _ in rows.len().min(self.capacity)..self.capacity {
            self.list.append(&self.empty_row());
        }
        drop(buttons);
        self.empty.set_visible(rows.is_empty());
        // The padded list is complete now, so this is the frame its height can be
        // measured from - the one thing that pins the card's size.
        self.calibrate();
        self.select(selected);
    }

    /// Pin the list's viewport to the height of the padded list, once.
    ///
    /// The list always holds `capacity` rows (see the module comment), so its
    /// natural height is the card's result height - and it is measured *after*
    /// the padding rows are in, which is what makes it the whole list rather than
    /// one row. Taking it once and keeping it is the point: when the `.touch`
    /// rules make the rows taller, the content grows past this viewport and
    /// scrolls instead of making the card taller than the screen has room for.
    ///
    /// A widget that has never been laid out still measures - GTK's measure phase
    /// needs no allocation - so the first paint has a number; the idle callback
    /// is only for the pathological case of a display with no fonts resolved yet.
    fn calibrate(&self) {
        if self.list_height.get() > 0 {
            return;
        }
        let natural = self.natural_list_height();
        if natural > 0 {
            self.list_height.set(natural);
            self.scroller.set_size_request(-1, natural);
            return;
        }
        let list = self.list.clone();
        let scroller = self.scroller.clone();
        let height = self.list_height.clone();
        gtk::glib::idle_add_local_once(move || {
            if height.get() > 0 {
                return;
            }
            let (_, natural, _, _) = list.measure(gtk::Orientation::Vertical, -1);
            if natural > 0 {
                height.set(natural);
                scroller.set_size_request(-1, natural);
            }
        });
    }

    /// The padded list's natural height, or `0` when it cannot be measured yet.
    fn natural_list_height(&self) -> i32 {
        self.list.measure(gtk::Orientation::Vertical, -1).1
    }

    /// Put the accent on one row, and make sure it is on screen.
    pub fn select(&self, index: usize) {
        for (position, button) in self.rows.borrow().iter().enumerate() {
            if position == index {
                button.add_css_class("selected");
            } else {
                button.remove_css_class("selected");
            }
        }
        self.scroll_to(index);
    }

    /// Move the viewport so row `index` is visible, moving as little as possible.
    ///
    /// This is what keeps the arrow keys working when the viewport is shorter
    /// than the padded list - which is the normal state on a tablet, where the
    /// rows are finger-sized and the card is not. A row that is already on screen
    /// is left alone, so walking two rows down does not jump the list around.
    fn scroll_to(&self, index: usize) {
        let height = f64::from(self.list_height.get());
        if height <= 0.0 {
            return;
        }
        let row = height / self.capacity.max(1) as f64;
        let top = index as f64 * row;
        let bottom = top + row;
        let adjustment = self.scroller.vadjustment();
        let value = adjustment.value();
        let page = adjustment.page_size();
        if top < value {
            adjustment.set_value(top);
        } else if bottom > value + page {
            adjustment.set_value(bottom - page);
        }
    }

    /// The sizes the card actually came out at: the content widget, and the
    /// result viewport inside it. `status` prints them, which is how a list that
    /// grew the card - the thing the padded height and the touch rules exist to
    /// prevent - is found without measuring a screenshot.
    pub fn measured(&self) -> String {
        if self.root.width() == 0 {
            return "not drawn yet".to_string();
        }
        format!(
            "content {}x{}, results viewport {}px tall",
            self.root.width(),
            self.root.height(),
            self.list_height.get(),
        )
    }

    /// Put the keyboard into the field. Called when the card opens, so that the
    /// first keystroke is the first character of the query and not something
    /// that had to be clicked first.
    pub fn focus_entry(&self) {
        self.entry.grab_focus();
    }

    /// Replace the query (`""` on opening), which repaints the list through the
    /// change handler like any other edit.
    pub fn set_query(&self, text: &str) {
        self.entry.set_text(text);
        // A reused entry keeps its old selection, and the caret is where the
        // next keystroke lands; put it at the end of what was just set.
        self.entry.set_position(-1);
    }

    fn clear(&self) {
        self.rows.borrow_mut().clear();
        // The padding rows are never in `self.rows` (they are not results and
        // cannot be selected), so the list is emptied by walking it.
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
    }

    /// One of the invisible rows that keeps the list at its full height.
    ///
    /// Laid out exactly like a result row and then made transparent, because the
    /// *height* is the whole point: a space-only label is measured 2 px shorter
    /// than a real one (Pango falls back to different metrics for a string with
    /// no glyphs in it - measured, 52 px against 54), and an empty label draws a
    /// stray one-pixel mark. Transparent text keeps the metrics and shows
    /// nothing.
    ///
    /// `can_target(false)` is what keeps it from being hovered or clicked - it
    /// exists to hold space, and a row that reacted to the pointer while showing
    /// nothing would be a bug you could feel but not see.
    fn empty_row(&self) -> gtk::Button {
        let button = gtk::Button::new();
        button.add_css_class("result");
        button.add_css_class("blank");
        button.set_halign(gtk::Align::Fill);
        button.set_can_target(false);

        let texts = self.texts(&Row {
            icon: None,
            glyph: Stand::Letter(String::new()),
            // A glyph, so the line is measured like a real row's - and then not
            // painted at all.
            title: "0".to_string(),
            subtitle: "0".to_string(),
        });
        texts.set_opacity(0.0);
        button.set_child(Some(&texts));
        button
    }

    /// One clickable row: icon, then a title over a subtitle.
    fn row_button(&self, row: &Row) -> gtk::Button {
        let button = gtk::Button::new();
        button.add_css_class("result");
        button.set_halign(gtk::Align::Fill);

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        content.append(&self.icon(row));
        content.append(&self.texts(row));
        button.set_child(Some(&content));
        button
    }

    /// The title over the subtitle.
    ///
    /// Both labels are always there, even when empty, so that every row is
    /// exactly as tall as every other one - which is what makes the padded list
    /// a fixed height without measuring anything.
    fn texts(&self, row: &Row) -> gtk::Box {
        let texts = gtk::Box::new(gtk::Orientation::Vertical, 1);
        texts.set_hexpand(true);
        texts.set_valign(gtk::Align::Center);

        let title = gtk::Label::new(Some(&row.title));
        title.add_css_class("result-title");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text::cap_width(&title, self.title_width);
        texts.append(&title);

        let subtitle = gtk::Label::new(Some(&row.subtitle));
        subtitle.add_css_class("result-sub");
        subtitle.set_xalign(0.0);
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        text::cap_width(&subtitle, self.title_width);
        texts.append(&subtitle);

        texts
    }

    /// The icon, or the mark that stands in for it, in a box of a fixed size so
    /// every row's title starts at the same place.
    fn icon(&self, row: &Row) -> gtk::Box {
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        holder.set_size_request(self.icon_size, self.icon_size);
        holder.set_halign(gtk::Align::Center);
        holder.set_valign(gtk::Align::Center);

        match &row.icon {
            Some(paintable) => {
                let image = gtk::Image::new();
                image.set_paintable(Some(paintable));
                image.set_pixel_size(self.icon_size);
                image.set_halign(gtk::Align::Center);
                image.set_valign(gtk::Align::Center);
                holder.append(&image);
            }
            None => match &row.glyph {
                Stand::Letter(letter) => {
                    let label = gtk::Label::new(Some(letter));
                    label.add_css_class("row-letter");
                    label.set_halign(gtk::Align::Center);
                    label.set_valign(gtk::Align::Center);
                    holder.append(&label);
                }
                Stand::Icon(icon) => {
                    let image = icons::lucide(icon, self.icon_size);
                    image.add_css_class("row-icon");
                    image.set_halign(gtk::Align::Center);
                    image.set_valign(gtk::Align::Center);
                    holder.append(&image);
                }
            },
        }
        holder
    }
}

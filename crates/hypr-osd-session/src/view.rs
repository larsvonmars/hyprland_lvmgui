//! The session card: a title, one row per action, and a line saying how the card
//! closes.
//!
//! The rows are the bar's session menu (`~/.config/waybar/scripts/popup.py`) as
//! an OSD: glyph plus label, a hover tint, `@crit` when a row is armed or
//! destructive - and the "click again" rule, which is the one thing here that has
//! to be right. A card that appears under the pointer because a hardware key was
//! pressed must not be able to shut the machine down with a single click.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use hypr_osd_core::Osd;

use crate::actions::{self, Action};

/// How long the card stays up once a row has been armed. The card's own duration
/// (config `duration_ms`) may be shorter than this, and an armed row must not
/// vanish before it can be answered - so arming re-arms the timer with this.
const CONFIRM_GRACE: Duration = Duration::from_secs(6);

pub struct SessionView {
    /// The card's content, handed to the shell.
    pub root: gtk::Box,
    rows: Vec<Rc<Row>>,
}

impl SessionView {
    pub fn new() -> Self {
        let title = gtk::Label::new(Some("Session"));
        title.add_css_class("title");
        title.set_xalign(0.0);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
        root.append(&title);

        let rows: Vec<Rc<Row>> = actions::all()
            .into_iter()
            .map(|action| {
                let row = Rc::new(Row::new(action));
                root.append(&row.button);
                row
            })
            .collect();

        let hint = gtk::Label::new(Some("Closes by itself · destructive actions ask twice"));
        hint.add_css_class("hint");
        hint.set_xalign(0.0);
        root.append(&hint);

        SessionView { root, rows }
    }

    /// Wire the rows: a click either arms the action or runs it.
    pub fn hook(self: &Rc<Self>, osd: &Rc<Osd>) {
        for row in &self.rows {
            let row = Rc::clone(row);
            let view = Rc::clone(self);
            let osd = osd.clone();
            // The button is the connection target, `row` is what the click needs
            // to see - so the handle is cloned out before the closure takes it.
            let button = row.button.clone();
            button.connect_clicked(move |_| view.click(&row, &osd));
        }
        osd.stay_open_while_hovered(&self.root);
    }

    /// Forget every armed row. Called whenever the card is (re)shown: an armed
    /// row that survived a hide would be a loaded gun nobody remembers loading.
    pub fn disarm_all(&self) {
        for row in &self.rows {
            row.disarm();
        }
    }

    /// Re-ask the world what each row can do.
    ///
    /// The answer can change while the daemon runs, and installing a screen
    /// locker is the case that actually happens - and it happens right before the
    /// user looks at this card, which is how they find out it worked. Cheap (a
    /// handful of `stat` calls) and only run when the card is about to be shown,
    /// so there is nothing to cache.
    pub fn refresh(&self) {
        for row in &self.rows {
            row.refresh();
        }
    }

    fn click(self: &Rc<Self>, row: &Rc<Row>, osd: &Rc<Osd>) {
        // Copied out of the borrow first: `arm` and the other rows' `disarm`
        // touch cells of their own, and holding one borrow across them is how a
        // panic gets written.
        let confirm = row.action.borrow().confirm_label;
        if let Some(confirm) = confirm {
            if !row.armed.get() {
                // One armed row at a time, exactly like the bar's menu.
                for other in &self.rows {
                    if !Rc::ptr_eq(other, row) {
                        other.disarm();
                    }
                }
                row.arm(confirm);
                // Give the confirmation its own timeout instead of the card's
                // remaining sliver: the card may be seconds away from hiding.
                osd.reveal(CONFIRM_GRACE);
                return;
            }
        }
        self.fire(row, osd);
    }

    fn fire(&self, row: &Rc<Row>, osd: &Rc<Osd>) {
        // The borrow ends with the statement, so the error path below is free to
        // disarm the row, which reads the same cell again.
        let result = actions::run(&row.action.borrow());
        match result {
            // The menu closes on a choice, the way the bar's menu does - and for
            // a lock or a shutdown it would be gone a moment later anyway.
            Ok(()) => osd.hide(),
            Err(error) => {
                eprintln!("hypr-osd-session: {error}");
                row.disarm();
            }
        }
    }
}

/// One row of the menu.
struct Row {
    /// Not `Copy`, and not constant: `refresh` replaces it with a fresh answer
    /// from `actions`, so it lives behind a cell.
    action: RefCell<Action>,
    button: gtk::Button,
    label: gtk::Label,
    /// Whether the row is waiting for its second click.
    armed: Cell<bool>,
}

impl Row {
    fn new(action: Action) -> Self {
        let glyph = gtk::Label::new(Some(action.glyph));
        // No font-family: the base stylesheet already sets the bar's font, which
        // is where these codepoints come from.
        glyph.add_css_class("icon");
        glyph.set_size_request(26, -1);

        let label = gtk::Label::new(Some(&action.label));
        label.set_halign(gtk::Align::Start);
        label.set_xalign(0.0);
        label.set_hexpand(true);

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        content.append(&glyph);
        content.append(&label);

        let button = gtk::Button::new();
        button.add_css_class("row");
        if action.danger {
            button.add_css_class("danger");
        }
        button.set_sensitive(action.enabled);
        button.set_child(Some(&content));

        Row {
            action: RefCell::new(action),
            button,
            label,
            armed: Cell::new(false),
        }
    }

    fn arm(&self, text: &str) {
        self.armed.set(true);
        self.label.set_text(text);
        self.button.add_css_class("confirm");
    }

    fn disarm(&self) {
        if self.armed.replace(false) {
            self.label.set_text(&self.action.borrow().label);
            self.button.remove_css_class("confirm");
        }
    }

    /// Ask `actions` again whether this row can do its thing.
    ///
    /// The one answer that changes while the daemon runs is "is there a screen
    /// locker?", and it changes exactly when the user is about to look here. An
    /// armed row keeps the question that is already on screen: re-labelling it
    /// mid-confirmation would rewrite what is being confirmed.
    fn refresh(&self) {
        let id = self.action.borrow().id;
        let Some(fresh) = actions::find(id) else {
            return;
        };
        self.button.set_sensitive(fresh.enabled);
        if !self.armed.get() {
            self.label.set_text(&fresh.label);
        }
        *self.action.borrow_mut() = fresh;
    }
}

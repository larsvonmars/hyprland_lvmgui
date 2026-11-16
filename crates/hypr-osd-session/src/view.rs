//! The session card: a title, one row per action, and a line saying how the card
//! closes.
//!
//! The rows are the desktop's session menu as a card: the five actions, each
//! an OSD: icon plus label, a hover tint, `@crit` when a row is armed or
//! destructive - and the "click again" rule, which is the one thing here that has
//! to be right. A card that appears under the pointer because a hardware key was
//! pressed must not be able to shut the machine down with a single click.
//!
//! # By finger
//!
//! A menu card is opened by a tap (the bar's power pill, or a system gesture) but
//! a tap is not something a *card* can close: a touchscreen has no pointer to
//! move away, and no Escape key. So while the machine is a tablet the card also
//! asks the shell for a tap catcher around itself (see [`SessionView::arm_touch`]):
//! the rest of the screen is dimmed and a tap anywhere there closes the menu
//! through the element's own `hide`, exactly like picking a row does. The
//! rectangle the catcher leaves open is the card's own - [`monitor_hole`] works it
//! out from the measured widget and the placement the shell will give it.
//!
//! The rows themselves are finger-sized by the stylesheet (`.touch button.row`),
//! and the hint line says how to get out instead of describing the pointer - a
//! menu whose only written way out is "click elsewhere" is a dead end on a
//! tablet.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use hypr_osd_core::icons;
use hypr_osd_core::monitors;
use hypr_osd_core::{touch, Hole, Osd, CARD_PAD_X, CARD_PAD_Y, SHADOW_PAD};

use crate::actions::{self, Action};

/// How long the card stays up once a row has been armed. The card's own duration
/// (config `duration_ms`) may be shorter than this, and an armed row must not
/// vanish before it can be answered - so arming re-arms the timer with this.
const CONFIRM_GRACE: Duration = Duration::from_secs(6);

/// The icon column: as wide as every row's icon, and as wide as the largest of
/// them is drawn, so the five labels line up whatever mark is beside them.
const ICON: i32 = 17;
const ICON_COLUMN: i32 = 26;

/// What the hint line says on a machine with a pointer: the card is also a
/// notification, so it does go away by itself, and the destructive rows ask
/// twice.
const HINT_POINTER: &str = "Closes by itself · destructive actions ask twice";

/// And what it says on a tablet, where there is no pointer to move away and no
/// key to press: a tap outside is the way out, and saying so is the only hint the
/// user gets.
const HINT_TOUCH: &str = "Tap outside to close · destructive actions ask twice";

pub struct SessionView {
    /// The card's content, handed to the shell.
    pub root: gtk::Box,
    rows: Vec<Rc<Row>>,
    /// The line under the rows. Kept because its text follows the machine: see
    /// [`HINT_POINTER`] and [`HINT_TOUCH`].
    hint: gtk::Label,
    /// The card's configured width and placement, kept for the tap catcher's
    /// geometry - the shell builds the surface from these, and the hole has to be
    /// the rectangle it ends up with.
    width: i32,
    bottom_margin: f64,
}

impl SessionView {
    pub fn new(width: i32, bottom_margin: f64) -> Self {
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

        let hint = gtk::Label::new(Some(HINT_POINTER));
        hint.add_css_class("hint");
        hint.set_xalign(0.0);
        root.append(&hint);

        SessionView {
            root,
            rows,
            hint,
            width,
            bottom_margin,
        }
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
        // The shell's helper holds the card open while a finger is *down* on it
        // (and, on a laptop, while the pointer rests on it) - a menu must not
        // fade out from under the thing you are aiming at.
        osd.stay_open_while_hovered(&self.root);
    }

    /// Follow the machine: on a tablet the hint says how a finger leaves the
    /// menu, and on a laptop it is the original line.
    pub fn apply_touch(&self, tablet: bool) {
        self.hint
            .set_text(if tablet { HINT_TOUCH } else { HINT_POINTER });
    }

    /// Put the shell's tap catcher around the card while it is up, or take it
    /// away.
    ///
    /// Called when the card is revealed and again when the machine is folded or
    /// unfolded under it. The card is measured *now* rather than remembered from
    /// start-up: `duration_ms = 0` and a config-free width leave the card's size
    /// to its content, and a row that asks twice is wider than the same row at
    /// rest. Every path that takes the card down - picking a row, the auto-hide
    /// timer, this very catcher - runs the shell's `hide`, which takes the catcher
    /// with it, so there is nothing to undo here.
    pub fn arm_touch(&self, osd: &Rc<Osd>) {
        let hide = {
            let osd = osd.clone();
            move || osd.hide()
        };
        let hole = touch::tablet()
            .then(|| monitor_hole(osd, &self.root, self.width, self.bottom_margin))
            .flatten();
        match hole {
            Some(hole) => osd.tap_catcher(Some(hole), hide),
            // Not a tablet (or no monitor to measure): no catcher, and the mouse
            // card behaves exactly as it always did.
            None => osd.tap_catcher(None, hide),
        }
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
                // A confirmation label is longer than the row's own, so the card
                // may have grown a little: the catcher is built around the card as
                // it is now, or a fingertip landing on the new edge would land on
                // the dim instead of on the row it is confirming.
                self.arm_touch(osd);
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

// ---------------------------------------------------------------------------
// The card's rectangle on the screen
// ---------------------------------------------------------------------------

/// The card's rectangle on its monitor, in the logical pixels the catcher's
/// strips are laid out in, or `None` when there is no monitor to measure against.
///
/// The shell owns the surface and its placement, so this repeats the placement's
/// arithmetic rather than asking the shell: for [`hypr_osd_core::Placement::Bottom`]
/// the card sits `bottom_margin` of the screen's height up from the bottom edge,
/// centred horizontally, with the content inset by `CARD_PAD_*`. The one thing
/// that cannot be repeated is the card's *size* - whether the config pins a width
/// or the content decides - so that half is measured from the widget (see
/// [`card_size`]).
fn monitor_hole(
    osd: &Osd,
    root: &impl IsA<gtk::Widget>,
    width: i32,
    bottom_margin: f64,
) -> Option<Hole> {
    let screen = monitor_size(osd)?;
    Some(bottom_hole(screen, card_size(root, width), bottom_margin))
}

/// The logical size of the monitor the card is on.
///
/// GDK's geometry is already in layout pixels - a 2736x1824 panel at 2x is
/// 1368x912 here - which is the same space a layer-shell surface is positioned
/// in, so there is no scale factor to apply.
fn monitor_size(osd: &Osd) -> Option<(i32, i32)> {
    let monitor = osd
        .monitor()
        .as_deref()
        .and_then(monitors::find)
        .or_else(monitors::first)?;
    let geometry = monitor.geometry();
    Some((geometry.width(), geometry.height()))
}

/// The card's size in logical pixels: the content as it will be drawn, plus the
/// padding the shell puts around it, and never narrower than the configured
/// width.
///
/// Measured rather than read from the card's last layout, because the content
/// changes while the card is up - a row that turns into "Click again to shut
/// down" is a wider row - and the catcher's hole has to follow the card that is
/// on screen now. The configured width is a *floor*, exactly as the shell applies
/// it: a zero width (the default) means "as wide as the content needs".
fn card_size(root: &impl IsA<gtk::Widget>, width: i32) -> (i32, i32) {
    let root = root.as_ref();
    // Measured, not read from the allocation: the widget may have been laid out
    // for the label it was showing a moment ago (a row that just turned into its
    // "click again" text), and a stale allocation would hand the catcher a hole
    // that no longer matches the card.
    let (_, natural_width, _, _) = root.measure(gtk::Orientation::Horizontal, -1);
    let (_, natural_height, _, _) = root.measure(gtk::Orientation::Vertical, natural_width);
    (
        width.max(natural_width + 2 * CARD_PAD_X),
        natural_height + 2 * CARD_PAD_Y,
    )
}

/// Where a bottom-anchored card of `card` size sits on a `screen`-sized monitor,
/// with its bottom edge `bottom_margin` of the screen's height up.
///
/// Split out from the measuring so the arithmetic can be tested without a
/// compositor. It mirrors the shell exactly: `set_bottom_margin` keeps
/// `SHADOW_PAD` back from the transparent frame, so the *card* ends up at
/// `screen.height * bottom_margin`, and the layer surface is centred horizontally
/// because it is anchored to the bottom edge only.
fn bottom_hole(screen: (i32, i32), card: (i32, i32), bottom_margin: f64) -> Hole {
    let (screen_width, screen_height) = screen;
    let (card_width, card_height) = card;
    // The same clamp the shell applies: a margin smaller than the shadow pad
    // would put the frame off the bottom edge, so it is pinned to 0 instead.
    let margin = ((f64::from(screen_height) * bottom_margin).round() as i32 - SHADOW_PAD).max(0);
    let bottom = screen_height - margin - SHADOW_PAD;
    Hole {
        x: (screen_width - card_width) / 2,
        y: bottom - card_height,
        width: card_width,
        height: card_height,
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
        let icon = icons::lucide(action.icon, ICON);
        icon.add_css_class("row-icon");
        icon.set_size_request(ICON_COLUMN, -1);

        let label = gtk::Label::new(Some(&action.label));
        label.set_halign(gtk::Align::Start);
        label.set_xalign(0.0);
        label.set_hexpand(true);

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        content.append(&icon);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// This machine: 2736x1824 at scale 2 is 1368x912 logical pixels, and the
    /// default `bottom_margin = 0.12` is 109 px up from the bottom.
    #[test]
    fn a_bottom_card_sits_where_the_shell_puts_it() {
        let hole = bottom_hole((1368, 912), (340, 100), 0.12);
        assert_eq!(
            hole,
            Hole {
                x: (1368 - 340) / 2,
                y: 912 - 109 - 100,
                width: 340,
                height: 100,
            }
        );
        // The hole grown by the shadow pad is exactly the surface the shell drew:
        // card + SHADOW_PAD on every side, with the window bottom at
        // 912 - (109 - 18).
        assert_eq!(hole.y + hole.height + SHADOW_PAD, 912 - (109 - SHADOW_PAD));
    }

    /// A margin smaller than the transparent frame is pinned to the screen edge,
    /// exactly like `set_bottom_margin` does.
    #[test]
    fn a_tiny_margin_is_clamped_to_the_screen_edge() {
        let hole = bottom_hole((1368, 912), (340, 100), 0.0);
        assert_eq!(hole.y + hole.height, 912 - SHADOW_PAD);
        // Taller than it is placed: a card can hang off the top of the screen,
        // and the catcher's strips are clamped there by the shell.
        let tall = bottom_hole((1368, 912), (340, 2000), 0.12);
        assert!(tall.y < 0);
    }
}

//! `hypr-osd-switcher` - element #4: an Alt-Tab window switcher.
//!
//! A grid of every window you have open, most recently used first, with each
//! window's application icon and title. Tab walks it, Enter or releasing Alt
//! switches to the selected window, Escape walks away.
//!
//! This is one of the three elements that take the keyboard (the others are the
//! workspace overview and the launcher). An OSD that eats a
//! keystroke is a bug - the volume card must not stop you typing - but a
//! switcher is *driven* by the keys you are holding: it has to see the Alt
//! release that ends the switch, and the only way to see it is to own the
//! keyboard for as long as the card is up. `osd.lua` binds Alt+Tab to
//! `switcher next`, and everything after that first press arrives on the card
//! itself. (The other one is the workspace overview, for the same kind of
//! reason: a card you opened on purpose may keep its own keys.)
//!
//! Verbs (`hypr-osd-switcher <verb>`):
//!
//! ```text
//!   next                 open the card and step forward (what Alt+Tab runs)
//!   prev                 the same, backwards (Alt+Shift+Tab)
//!   show                 open the card without moving the selection
//!   commit               switch to the selected window and close
//!   cancel               close without switching
//!   status               list the windows in switcher order, no card
//!   (no verb)            start the daemon and wait for the first key press
//! ```
//!
//! # By finger
//!
//! A touchscreen has no Alt to release and no Tab to walk with, so a finger user
//! is handed the card by `show` instead - the system's touch gesture (another
//! crate) calls it, and it has to work from a cold daemon with nothing held: it
//! reads the window list, draws it and takes the keyboard exactly like `next`
//! does, just without moving the selection first. From there:
//!
//! * a tap on a tile is the click it always was, and it switches;
//! * a tap *outside* the card cancels it - while the machine is a tablet the card
//!   arms the shell's tap catcher around itself ([`Switcher::arm_catcher`]), so
//!   the rest of the screen is the "no thanks" target a fingertip needs;
//! * the idle-commit watchdog does not fire while a finger is still down on the
//!   card ([`Switcher::arm_idle`]), because a card that commits while you are
//!   still aiming is a card that switches windows under your hand.
//!
//! The hint line and the tiles themselves follow the machine too: `.touch` grows
//! the tiles (and their type), and the hint says "tap a window" rather than
//! describing keys that are not there.

mod view;

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::monitors;
use hypr_osd_core::touch;
use hypr_osd_core::windows::{self, Window};
use hypr_osd_core::{
    css, hyprctl, run, Config, Content, Hole, Keyboard, Opts, Osd, Placement, CARD_PAD_X,
    CARD_PAD_Y,
};

use view::SwitcherView;

/// D-Bus application id - and therefore the single-instance key.
const APP_ID: &str = "com.schells2.osd.switcher";
/// Layer-shell namespace; the collection shares the `hypr-osd` prefix so one
/// layer rule in `osd.lua` covers every element.
const NAMESPACE: &str = "hypr-osd";
/// Element name: `~/.config/hypr-osd/switcher.conf`.
const ELEMENT: &str = "switcher";

// Defaults; every one is overridable in the config file.
const DEFAULT_COLUMNS: i32 = 5;
const DEFAULT_TILE_WIDTH: i32 = 132;
const DEFAULT_ICON_SIZE: i32 = 72;
/// How long the card may sit there untouched before it commits on its own.
const DEFAULT_IDLE_COMMIT_MS: u64 = 5000;
/// How often a held finger is re-checked once the idle watchdog has come due.
/// Short enough that letting go continues the countdown promptly, long enough
/// that a finger resting on a tile is not a busy loop.
const HELD_RECHECK: Duration = Duration::from_millis(150);

/// What the config file resolved to.
struct Settings {
    /// Tiles per row.
    columns: i32,
    /// Width of one tile (and so how much title fits in it).
    tile_width: i32,
    /// Icon size in pixels.
    icon_size: i32,
    /// Commit if no key arrives for this long (`0` disables the safety net).
    idle_commit: Option<Duration>,
}

impl Settings {
    fn load(config: &Config) -> Self {
        let idle = config.millis("idle_commit_ms", DEFAULT_IDLE_COMMIT_MS);
        Settings {
            columns: config.i32("columns", DEFAULT_COLUMNS).max(1),
            tile_width: config.i32("tile_width", DEFAULT_TILE_WIDTH).max(48),
            icon_size: config.i32("icon_size", DEFAULT_ICON_SIZE).max(16),
            idle_commit: if idle.is_zero() { None } else { Some(idle) },
        }
    }

    /// The card's width: the grid it holds, plus the card's own padding. Read
    /// from the same numbers the grid lays out with, so the two cannot drift.
    fn width(&self) -> i32 {
        let gaps = (self.columns - 1).max(0) * view::GAP;
        self.columns * self.tile_width + gaps + 2 * CARD_PAD_X
    }
}

/// The switcher's state: what is in the list, what is selected, and whether the
/// card is up.
struct Switcher {
    osd: Rc<Osd>,
    view: Rc<SwitcherView>,
    settings: Rc<Settings>,
    /// The windows as they were when the card opened. Alt-tabbing must not make
    /// the list shuffle under your fingers while you are walking it - and the
    /// list is only ever read while the card is up, or on the way into it.
    list: RefCell<Vec<Window>>,
    selected: Cell<usize>,
    /// The idle-commit watchdog (see [`Switcher::arm_idle`]).
    idle: RefCell<Option<glib::SourceId>>,
}

impl Switcher {
    fn new(osd: &Rc<Osd>, view: Rc<SwitcherView>, settings: Rc<Settings>) -> Rc<Self> {
        Rc::new(Switcher {
            osd: osd.clone(),
            view,
            settings,
            list: RefCell::new(Vec::new()),
            selected: Cell::new(0),
            idle: RefCell::new(None),
        })
    }

    /// Open the card on a fresh list, or walk the one already open.
    ///
    /// The first `next` both opens the card and steps onto the second window -
    /// the one you were in before this one - which is what makes a quick
    /// Alt+Tab a two-window swap and holding Alt a walk through the whole list.
    fn step(self: &Rc<Self>, delta: i32) {
        if !self.osd.is_visible() {
            self.open();
        }
        self.move_to(self.selected.get() as i32 + delta);
        self.arm_idle();
    }

    /// Open the card with the list as it is now, selection on the current window.
    ///
    /// This is the touch entry point as well: `show` runs it with nothing held,
    /// and it has no dependency on a key release or a previous `next` - the list
    /// is read, drawn and shown, and the card then waits for a tap. It arms the
    /// tap catcher and the idle watchdog like every other open does.
    fn open(self: &Rc<Self>) {
        let list = windows::list();
        if list.is_empty() {
            eprintln!("hypr-osd-switcher: no windows to switch to");
            return;
        }
        self.selected.set(0);
        // The hint describes whichever input this machine has, and the card can
        // be opened after the machine was folded (the idle watchdog may have
        // been silenced by an open card since).
        self.view.set_hint(touch::tablet());
        self.view.render(&list, &self.settings);
        *self.list.borrow_mut() = list;
        self.osd.set_content(&self.view.root);
        self.osd.show();
        self.view.select(0);
        self.arm_catcher();
        self.arm_idle();
    }

    /// Select `index`, wrapping around either end.
    fn move_to(&self, index: i32) {
        let count = self.list.borrow().len() as i32;
        if count == 0 {
            return;
        }
        let index = index.rem_euclid(count);
        self.selected.set(index as usize);
        self.view.select(index as usize);
    }

    /// Switch to the selected window and take the card down.
    fn commit(self: &Rc<Self>) {
        let address = self
            .list
            .borrow()
            .get(self.selected.get())
            .map(|window| window.address.clone());
        self.close();
        if let Some(address) = address {
            if !hyprctl::focus_window(&address) {
                eprintln!("hypr-osd-switcher: could not focus {address}");
            }
        }
    }

    /// Take the card down without switching.
    fn cancel(self: &Rc<Self>) {
        self.close();
    }

    fn close(self: &Rc<Self>) {
        if let Some(source) = self.idle.borrow_mut().take() {
            source.remove();
        }
        // A card cannot be gone while its tap catcher still dims the screen; the
        // shell's `hide` does this too, as a backstop, but clearing it here keeps
        // the two in step when the close came from the catcher itself.
        self.osd.tap_catcher(None, || {});
        self.osd.hide();
    }

    /// Put the shell's tap catcher around the card while it is up on a tablet, or
    /// take it away.
    ///
    /// Called when the card is opened and again if the machine is folded under
    /// it. On a laptop there is no catcher: the pointer's click on another window
    /// is the card's own click, and a dimmed screen would be a new behaviour for
    /// a mouse user who never asked for one.
    fn arm_catcher(self: &Rc<Self>) {
        let cancel = {
            let me = self.clone();
            move || me.cancel()
        };
        let hole = touch::tablet()
            .then(|| monitor_hole(&self.osd, &self.view.root, self.settings.width()))
            .flatten();
        match hole {
            Some(hole) => self.osd.tap_catcher(Some(hole), cancel),
            None => self.osd.tap_catcher(None, cancel),
        }
    }

    /// Commit if no key arrives for a while.
    ///
    /// The card normally ends with the Alt release, but a release that happens
    /// before the card has the keyboard is a release nobody sees: GTK cannot be
    /// asked whether a modifier is held, and there is no event to wait for. So
    /// the card watches for silence instead - and does what the release would
    /// have done, because by then the user has made up their mind. Every key
    /// press re-arms it, so holding Alt and browsing is not disturbed by it.
    ///
    /// A finger changes the last part: the card can be opened by a gesture with
    /// nothing held, and a tap takes longer than a key. The watchdog therefore
    /// asks the view whether anything is still down on the card and, if it is,
    /// looks again in [`HELD_RECHECK`] instead of committing - a finger that is
    /// still on a tile has not finished choosing.
    fn arm_idle(self: &Rc<Self>) {
        if let Some(source) = self.idle.borrow_mut().take() {
            source.remove();
        }
        let Some(duration) = self.settings.idle_commit else {
            return;
        };
        self.arm_idle_after(duration);
    }

    fn arm_idle_after(self: &Rc<Self>, after: Duration) {
        // The slot is overwritten rather than cleared first: either the previous
        // watchdog fired (a stale id, harmless to drop) or `arm_idle` removed it.
        let me = self.clone();
        let source = glib::timeout_add_local_once(after, move || {
            if !me.osd.is_visible() {
                return;
            }
            if me.view.is_pressed() {
                me.arm_idle_after(HELD_RECHECK);
                return;
            }
            me.commit();
        });
        *self.idle.borrow_mut() = Some(source);
    }
}

fn main() -> glib::ExitCode {
    let config = Config::load(ELEMENT);
    let settings = Rc::new(Settings::load(&config));

    let opts = Opts {
        app_id: APP_ID.to_string(),
        css: css::stylesheet(include_str!("switcher.css")),
        namespace: NAMESPACE.to_string(),
        width: settings.width(),
        // Centred: a switcher is a dialog you are looking at, not a notification
        // that arrives in the corner the bar lives in.
        placement: Placement::Center,
        // The one element that takes the keyboard - see the module comment.
        keyboard: Keyboard::Exclusive,
        // Unused by a centred card, kept at the default.
        bottom_margin: 0.12,
        // Cards follow the focused monitor; only the bar pins itself.
        ..Opts::default()
    };

    // The widgets are built inside the application's start-up (GTK does not
    // exist before that) and the command handler is installed before it, so the
    // two meet here. The switcher is built *once*: the list and the selection
    // have to survive between commands, because `next` three times is one walk
    // through three windows and not three separate opens.
    let state: Rc<OnceCell<Rc<Switcher>>> = Rc::new(OnceCell::new());

    let build = {
        let state = state.clone();
        let settings = settings.clone();
        Box::new(move |osd: &Rc<Osd>| {
            let view = Rc::new(SwitcherView::new());
            // The daemon starts in whatever mode the machine is in, and the shell
            // only calls the tablet handlers on a *change*: the starting answer
            // has to be applied by hand.
            view.set_hint(touch::tablet());
            let switcher = Switcher::new(osd, view.clone(), settings);

            // A click on a tile is a switch, exactly like pressing Enter on it:
            // the card takes the pointer for the same reason it takes the
            // keyboard. A finger's tap *is* that click, so nothing is special-
            // cased here.
            view.on_click({
                let switcher = switcher.clone();
                move |index| {
                    switcher.move_to(index as i32);
                    switcher.commit();
                }
            });

            // Folded or unfolded under the card: the hint and the catcher are the
            // two things that cannot follow the window's `touch` class on their
            // own.
            let watch = osd.clone();
            let watched = switcher.clone();
            osd.on_tablet_change(move |tablet| {
                watched.view.set_hint(tablet);
                if watch.is_visible() {
                    watched.arm_catcher();
                } else {
                    watch.tap_catcher(None, || {});
                }
            });

            install_keys(osd, switcher.clone());
            let root = view.root.clone().upcast::<gtk::Widget>();
            state.set(switcher).ok();
            Content::Single(root)
        })
    };

    let handle = {
        let state = state.clone();
        Rc::new(
            move |_osd: &Rc<Osd>, args: &[String]| -> Result<String, String> {
                let switcher = state
                    .get()
                    .ok_or("the switcher has not finished starting up")?;
                let Some(verb) = args.first().map(String::as_str) else {
                    // No verb: started by Hyprland's autostart, so wait for the
                    // first Alt+Tab.
                    return Ok(String::new());
                };
                match verb {
                    "next" => switcher.step(1),
                    "prev" => switcher.step(-1),
                    "show" => switcher.open(),
                    "commit" => switcher.commit(),
                    "cancel" => switcher.cancel(),
                    "status" => return Ok(status_lines()),
                    other => {
                        return Err(format!(
                            "unknown verb `{other}` (next | prev | show | commit | cancel | status)"
                        ))
                    }
                }
                Ok(String::new())
            },
        )
    };

    run(opts, build, handle)
}

// ---------------------------------------------------------------------------
// The card's rectangle on the screen
// ---------------------------------------------------------------------------

/// The card's rectangle on its monitor, in the logical pixels the catcher's
/// strips are laid out in, or `None` when there is no monitor to measure against.
fn monitor_hole(osd: &Osd, root: &impl IsA<gtk::Widget>, width: i32) -> Option<Hole> {
    let screen = monitor_size(osd)?;
    Some(center_hole(screen, card_size(root, width)))
}

/// The logical size of the monitor the card is on.
///
/// GDK's geometry is already in layout pixels, which is the space the card's own
/// width and margins are measured in, so no scale factor is applied.
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
/// shell's card padding, and never narrower than the width the grid was planned
/// for.
///
/// Measured rather than read from the card's last layout: opening the card again
/// can produce a different number of tile rows, and the catcher must surround the
/// grid that is on screen now. The planned width is a floor, exactly as the shell
/// applies it - the grid's own natural width is the same number.
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

/// Where a centred card of `card` size sits on a `screen`-sized monitor.
///
/// A [`Placement::Center`] surface has no anchor, so layer-shell centres it on
/// the output at its own size; the hole is that rectangle, with the shadow frame
/// the catcher grows it by already accounted for there. Split out from the
/// measuring so the arithmetic can be tested without a compositor.
fn center_hole(screen: (i32, i32), card: (i32, i32)) -> Hole {
    let (screen_width, screen_height) = screen;
    let (card_width, card_height) = card;
    Hole {
        x: (screen_width - card_width) / 2,
        y: (screen_height - card_height) / 2,
        width: card_width,
        height: card_height,
    }
}

/// Wire the keys the switcher lives on.
///
/// Installed through the shell rather than on a widget here, because the shell
/// rebuilds the surface when the card moves to another monitor - a
/// keyboard-driven element that lost its keys after a monitor change would be a
/// bug nobody could explain.
fn install_keys(osd: &Rc<Osd>, switcher: Rc<Switcher>) {
    let columns = switcher.settings.columns;
    osd.on_keys(
        {
            let switcher = switcher.clone();
            move |key, state| {
                if !switcher.osd.is_visible() {
                    return glib::Propagation::Proceed;
                }
                let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
                // `gdk::Key` is a newtype around the keyval rather than an enum,
                // so the arms below match on the value, not on a reference.
                let key = *key;
                let handled = match key {
                    gdk::Key::Tab => {
                        switcher.step(if shift { -1 } else { 1 });
                        true
                    }
                    gdk::Key::ISO_Left_Tab => {
                        switcher.step(-1);
                        true
                    }
                    gdk::Key::Right => {
                        switcher.step(1);
                        true
                    }
                    gdk::Key::Left => {
                        switcher.step(-1);
                        true
                    }
                    // A grid, so "down" is a row and not a tile.
                    gdk::Key::Down => {
                        switcher.step(columns);
                        true
                    }
                    gdk::Key::Up => {
                        switcher.step(-columns);
                        true
                    }
                    gdk::Key::Escape => {
                        switcher.cancel();
                        true
                    }
                    gdk::Key::Return | gdk::Key::KP_Enter => {
                        switcher.commit();
                        true
                    }
                    // 1-9 jump straight to a tile: the fastest way through a
                    // long list, and free once the grid is on screen.
                    other => match digit(&other) {
                        Some(index) if index < switcher.list.borrow().len() => {
                            switcher.move_to(index as i32);
                            switcher.commit();
                            true
                        }
                        _ => false,
                    },
                };
                if handled {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        },
        {
            move |key| {
                // Releasing Alt ends the switch - that is the whole interaction:
                // hold Alt, tap Tab until the right window is highlighted, let go.
                if matches!(*key, gdk::Key::Alt_L | gdk::Key::Alt_R) && switcher.osd.is_visible() {
                    switcher.commit();
                }
            }
        },
    );
}

/// `1`..`9` as a zero-based index.
fn digit(key: &gdk::Key) -> Option<usize> {
    let name = key.name()?;
    let digit = name.chars().next()?.to_digit(10)?;
    usize::try_from(digit).ok()?.checked_sub(1)
}

/// What `status` answers: the list in switcher order, one window per line, the
/// first being the window you are in - which is where a walk starts from. The
/// same list the card draws, for a script or for working out why the card draws
/// what it draws.
fn status_lines() -> String {
    let list = windows::list();
    if list.is_empty() {
        return "no windows".to_string();
    }
    list.iter()
        .enumerate()
        .map(|(index, window)| format!("{} {}", index + 1, window.describe()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This machine: 2736x1824 at scale 2 is 1368x912 logical pixels. The card is
    /// the grid's 724px plus the card padding, and a couple of rows tall; a
    /// centred card puts half of what is left on each side.
    #[test]
    fn a_centred_card_sits_in_the_middle_of_the_screen() {
        let hole = center_hole((1368, 912), (724, 300));
        assert_eq!(
            hole,
            Hole {
                x: (1368 - 724) / 2,
                y: (912 - 300) / 2,
                width: 724,
                height: 300,
            }
        );
        // The hole grown by the shadow pad is the surface the shell drew: the
        // card plus SHADOW_PAD on every side, still centred.
        assert_eq!(hole.x + hole.width / 2, 1368 / 2);
    }

    /// A card larger than the screen still answers with where the shell put it -
    /// a hole that starts off-screen is clamped by the catcher, not by this.
    #[test]
    fn a_card_bigger_than_the_screen_is_still_centred() {
        let hole = center_hole((1368, 912), (2000, 1200));
        assert!(hole.x < 0);
        assert!(hole.y < 0);
        assert_eq!(hole.width, 2000);
        assert_eq!(hole.height, 1200);
        assert_eq!(hole.x + hole.width / 2, 1368 / 2);
    }
}

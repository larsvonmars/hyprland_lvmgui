//! `hypr-osd-switcher` - element #4: an Alt-Tab window switcher.
//!
//! A grid of every window you have open, most recently used first, with each
//! window's application icon and title. Tab walks it, Enter or releasing Alt
//! switches to the selected window, Escape walks away.
//!
//! This is one of the two elements that take the keyboard. An OSD that eats a
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

mod view;

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::windows::{self, Window};
use hypr_osd_core::{css, hyprctl, run, Config, Content, Keyboard, Opts, Osd, Placement};

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
        self.columns * self.tile_width + gaps + 2 * hypr_osd_core::CARD_PAD_X
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
    fn open(self: &Rc<Self>) {
        let list = windows::list();
        if list.is_empty() {
            eprintln!("hypr-osd-switcher: no windows to switch to");
            return;
        }
        self.selected.set(0);
        self.view.render(&list, &self.settings);
        *self.list.borrow_mut() = list;
        self.osd.set_content(&self.view.root);
        self.osd.show();
        self.view.select(0);
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
    fn cancel(&self) {
        self.close();
    }

    fn close(&self) {
        if let Some(source) = self.idle.borrow_mut().take() {
            source.remove();
        }
        self.osd.hide();
    }

    /// Commit if no key arrives for a while.
    ///
    /// The card normally ends with the Alt release, but a release that happens
    /// before the card has the keyboard is a release nobody sees: GTK cannot be
    /// asked whether a modifier is held, and there is no event to wait for. So
    /// the card watches for silence instead - and does what the release would
    /// have done, because by then the user has made up their mind. Every key
    /// press re-arms it, so holding Alt and browsing is not disturbed by it.
    fn arm_idle(self: &Rc<Self>) {
        if let Some(source) = self.idle.borrow_mut().take() {
            source.remove();
        }
        let Some(duration) = self.settings.idle_commit else {
            return;
        };
        let me = Rc::clone(self);
        let source = glib::timeout_add_local_once(duration, move || {
            if me.osd.is_visible() {
                me.commit();
            }
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
            let switcher = Switcher::new(osd, view.clone(), settings);

            // A click on a tile is a switch, exactly like pressing Enter on it:
            // the card takes the pointer for the same reason it takes the
            // keyboard.
            view.on_click({
                let switcher = switcher.clone();
                move |index| {
                    switcher.move_to(index as i32);
                    switcher.commit();
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

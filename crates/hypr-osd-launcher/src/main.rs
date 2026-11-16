//! `hypr-osd-launcher` - the Spotlight-style search card, on `SUPER + SPACE`.
//!
//! One centred card: a search field over a short list of results. What you type
//! is matched against the applications installed on the machine (their names,
//! their generic names, their keywords and their categories) and, as you keep
//! typing, against the files and folders in your own directories. Enter opens
//! the selected one; Escape, or the key again, takes the card away.
//!
//! It is the same gesture as the older Tauri/Next.js launchpad it replaces - the
//! reason it is an element now is the smaller one: a GTK4 + gtk4-layer-shell
//! process draws this card with no webview, no Node runtime and no bundled
//! browser, which is the whole point of this collection.
//!
//! Verbs (`hypr-osd-launcher <verb>`):
//!
//! ```text
//!   toggle               open the card, or close it again (what SUPER+SPACE runs)
//!   show [query]         open it, with an empty query or the one given
//!   tap [connector]      open it from a *finger* (a system gesture rather than a
//!                        key): pinned, with a tap catcher around the card; a
//!                        second tap, or a tap outside it, closes it. `open` is
//!                        accepted as an alias, because a gesture that opens the
//!                        launcher is the same request under either name.
//!   hide | close         take it away
//!   refresh              re-scan the installed applications
//!   search <query>       print the ranked results, without drawing a card
//!   apps                 list every application that was found
//!   status               how many applications, which file roots, open or closed
//!   (no verb)            start the daemon and wait for the first key press
//! ```
//!
//! Three things are worth knowing about how it behaves, because all are
//! deliberately not like the window it replaces:
//!
//! * **It takes the keyboard while it is up.** A layer-shell surface that wants
//!   to be typed into has no other way (`Keyboard::Exclusive`); the alternative -
//!   needing a click before the field accepts a character - is not a launcher.
//!   The price is that a click elsewhere does not dismiss it (the compositor
//!   cannot take the keyboard back), so it ends the way the other keyboard cards
//!   do: Escape, the key again, or - if you walked away - `idle_close_ms` after
//!   a minute of no keys and no pointer movement.
//! * **On a tablet the field gets the on-screen keyboard, and the card a way
//!   out.** The field is focused when the card opens, which is what raises the
//!   daemon keyboard the bar starts when the machine folds (`touch::osk()`, and
//!   `attach` is the whole wiring); with none installed the card says so once on
//!   stderr. There is no Escape key on a touchscreen and no pointer to leave the
//!   card with either, so a card opened by a finger gets a **tap catcher**
//!   (`Osd::tap_catcher`) - a dimmed surface with a hole over the card, where a
//!   tap anywhere else closes it.
//! * **The file search runs on a thread.** The application half is already in
//!   memory and answers the keystroke; the file half is a filesystem walk, so it
//!   is debounced, run off the main loop, and its answer is only accepted if it
//!   still belongs to the newest query. A stale answer cannot overwrite a newer
//!   one (see [`Launcher::drain_file_results`]).

mod files;
mod view;

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::{
    css, hardware, icons, monitors, run, text, touch, Config, Content, Hole, Keyboard, Opts, Osd,
    Placement, CARD_PAD_X, CARD_PAD_Y,
};

use files::FileEntry;
use hypr_osd_core::apps::{self, App};
use hypr_osd_core::icons::names;
use hypr_osd_core::timer::{self, Timer};
use view::{LauncherView, Row, Stand};

/// D-Bus application id - and therefore the single-instance key.
const APP_ID: &str = "com.schells2.osd.launcher";
/// Layer-shell namespace; the collection shares the `hypr-osd` prefix so one
/// layer rule in `osd.lua` covers every element.
const NAMESPACE: &str = "hypr-osd";
/// Element name: `~/.config/hypr-osd/launcher.conf`.
const ELEMENT: &str = "launcher";

// Defaults; every one is overridable in the config file.
const DEFAULT_WIDTH: i32 = 640;
const DEFAULT_MAX_RESULTS: i32 = 7;
const DEFAULT_ICON_SIZE: i32 = 22;
const DEFAULT_IDLE_CLOSE_MS: u64 = 60_000;
const DEFAULT_DEBOUNCE_MS: u64 = 120;
/// How often the main loop looks for an answer from a file-search thread.
const POLL_MS: u64 = 25;

/// How often a tap-opened card re-measures itself for the tap catcher.
///
/// A timer rather than a frame callback: the catcher is asked for from the
/// command that opened the card, which can run before the card's window has a
/// frame clock - and a tick scheduled then is never called at all (measured on
/// the applications panel: the catcher simply never appeared). A timer runs on
/// the main loop whatever the surface is doing, and sixty milliseconds is fast
/// enough that the catcher is around the card by the time the finger has lifted.
const HOLE_POLL: Duration = Duration::from_millis(60);

/// What the config file resolved to.
struct Settings {
    /// The card's width in pixels.
    width: i32,
    /// How many rows the list holds - applications and files sharing them, and
    /// the list always reserving room for all of them, so the card keeps its
    /// size while you type (see `view`'s module comment).
    max_results: usize,
    /// Application (and file type) icon size in pixels.
    icon_size: i32,
    /// Whether files and folders are searched at all.
    files: bool,
    /// Close after this long without a key press or pointer movement (`0` never).
    idle_close: Option<Duration>,
    /// How long a keystroke waits before a file walk is started.
    debounce: Duration,
}

impl Settings {
    fn load(config: &Config) -> Self {
        let idle = config.millis("idle_close_ms", DEFAULT_IDLE_CLOSE_MS);
        Settings {
            width: config.i32("width", DEFAULT_WIDTH).max(360),
            max_results: config.i32("max_results", DEFAULT_MAX_RESULTS).clamp(1, 20) as usize,
            icon_size: config.i32("icon_size", DEFAULT_ICON_SIZE).clamp(12, 48),
            files: config.bool("files", true),
            idle_close: if idle.is_zero() { None } else { Some(idle) },
            debounce: config.millis("debounce_ms", DEFAULT_DEBOUNCE_MS),
        }
    }

    /// How far Page Up/Page Down walk: the rows that are on screen.
    fn page(&self) -> i32 {
        self.max_results as i32
    }
}

/// One row of the result list.
///
/// An index rather than a clone for the applications, because the list is
/// rebuilt on every keystroke and the scan is the one thing that must not be
/// repeated; a file entry is small enough to carry (`FileEntry` is a name and
/// two paths).
enum Item {
    /// Index into [`Launcher::apps`].
    App(usize),
    File(FileEntry),
}

/// The element's state: what was scanned, what matches, what is selected.
struct Launcher {
    osd: Rc<Osd>,
    view: Rc<LauncherView>,
    settings: Rc<Settings>,
    /// Every application on the machine, scanned once at start-up.
    apps: RefCell<Vec<App>>,
    /// The rows currently drawn, parallel to the view's buttons.
    results: RefCell<Vec<Item>>,
    /// The query the results belong to.
    query: RefCell<String>,
    selected: Cell<usize>,
    /// The file matches of the newest *completed* walk.
    file_results: RefCell<Vec<FileEntry>>,
    /// The newest walk's sequence number; an answer to an older one is dropped.
    file_seq: Cell<u64>,
    /// Walks still running, so the poll timer knows when to stop.
    file_pending: Cell<u32>,
    /// The channel the walk threads answer on.
    file_tx: mpsc::Sender<(u64, Vec<FileEntry>)>,
    file_rx: RefCell<mpsc::Receiver<(u64, Vec<FileEntry>)>>,
    /// Whether the poll timer is running. It stops itself, so this is the whole
    /// of its state (see [`Launcher::ensure_poll`]).
    polling: Cell<bool>,
    /// The debounce before a file walk: armed on every keystroke, and overtaken by
    /// the next one (see [`Launcher::changed`]).
    debounce: Rc<Timer>,
    /// The idle-close watchdog (see [`Launcher::arm_idle`]).
    idle: Rc<Timer>,
    /// Icon lookups, so a keystroke does not ask the icon theme for the same
    /// eight pictures again.
    icon_cache: RefCell<HashMap<String, Option<gdk::Paintable>>>,
    /// Whether this card was opened by a *finger* (`tap`, or any open while the
    /// machine is a tablet) - the flag the tap catcher and its hole follow.
    tap_opened: Cell<bool>,
    /// Whether a frame callback is currently keeping the catcher around the card.
    watching_hole: Cell<bool>,
    /// The hole the catcher was last given, so a frame that finds the card where
    /// it already was does not touch the shell.
    last_hole: Cell<Option<Hole>>,
}

impl Launcher {
    fn new(osd: &Rc<Osd>, view: Rc<LauncherView>, settings: Rc<Settings>) -> Rc<Self> {
        let (file_tx, file_rx) = mpsc::channel();
        Rc::new(Launcher {
            osd: osd.clone(),
            view,
            settings,
            apps: RefCell::new(Vec::new()),
            results: RefCell::new(Vec::new()),
            query: RefCell::new(String::new()),
            selected: Cell::new(0),
            file_results: RefCell::new(Vec::new()),
            file_seq: Cell::new(0),
            file_pending: Cell::new(0),
            file_tx,
            file_rx: RefCell::new(file_rx),
            polling: Cell::new(false),
            debounce: Rc::new(Timer::new()),
            idle: Rc::new(Timer::new()),
            icon_cache: RefCell::new(HashMap::new()),
            tap_opened: Cell::new(false),
            watching_hole: Cell::new(false),
            last_hole: Cell::new(None),
        })
    }

    /// Open the card, or close it if it is already up - what `SUPER + SPACE`
    /// runs, because one press should be able to undo itself.
    fn toggle(self: &Rc<Self>) {
        if self.osd.is_visible() {
            self.close();
        } else {
            self.open(None);
        }
    }

    /// Open the card with `query` in the field (empty when there is none), and
    /// put the keyboard in it.
    ///
    /// The query is reset on purpose: the card is not a window you come back to,
    /// it is a question you ask - and a launcher that reopened with the last
    /// search in it would make `SUPER + SPACE`, type, Enter mean something
    /// different the second time. `show <query>` is the exception, and it is for
    /// scripting rather than for the key.
    fn open(self: &Rc<Self>, query: Option<&str>) {
        let query = query.unwrap_or("");
        self.osd.set_content(&self.view.root);
        self.view.set_query(query);
        // `set_query` may not fire a change at all when the field already holds
        // that text, so the paint is asked for explicitly. It also arms the idle
        // watchdog, which is why `open` does not.
        self.changed(query);
        self.osd.show();
        self.view.focus_entry();
        // Again once GTK has laid the card out: a freshly mapped surface can
        // refuse focus until it has been allocated.
        let view = self.view.clone();
        glib::timeout_add_local_once(Duration::ZERO, move || view.focus_entry());
        // A tablet is typed into by the on-screen keyboard; what raises it is
        // the focus above, which the field's `Osk::attach` (installed at build)
        // turns into a text-input request. Saying so once beats a focused field
        // that cannot be typed into.
        if touch::tablet() {
            warn_missing_osk();
            // There is no Escape key on a touchscreen: a card opened while the
            // machine is a tablet leaves the tap catcher as its way out.
            self.arm_catcher();
        }
    }

    /// A finger opened the card - a system gesture rather than a key.
    ///
    /// The same card, opened the same way, with the one difference a fingertip
    /// needs: a **tap catcher** around it, because the gesture that opened it is
    /// not the gesture that can close it (there is no Escape key and no pointer
    /// to leave the card with). A second tap on whatever opened it closes it
    /// again, and so does a tap anywhere outside the card.
    ///
    /// `connector` is which screen the gesture happened on, when it can say:
    /// the card is centred on the focused output otherwise, and pinning it to
    /// the named one keeps it where the finger was rather than where a pointer
    /// last was.
    fn tap(self: &Rc<Self>, connector: Option<&str>) {
        if self.osd.is_visible() {
            self.close();
            return;
        }
        if let Some(connector) = connector {
            self.osd.pin_output(Some(connector));
        }
        self.open(None);
        // `open` arms the catcher for a tablet; a tap is a finger whatever the
        // switch says (a pen on a laptop-mode screen), so this is explicit and
        // idempotent.
        self.arm_catcher();
    }

    /// Leave a way out of the card: the tap catcher around its measured
    /// rectangle.
    fn arm_catcher(self: &Rc<Self>) {
        self.tap_opened.set(true);
        self.watch_hole();
    }

    /// Keep the tap catcher's hole around the card while it is open.
    ///
    /// A frame callback rather than one measurement, because the card has no
    /// size until a frame after it is shown, and the `touch` class can resize it
    /// when the machine folds. Comparing the hole with the last one placed means
    /// the shell is left alone until the card has really moved.
    fn watch_hole(self: &Rc<Self>) {
        if self.watching_hole.replace(true) {
            return;
        }
        self.last_hole.set(None);
        let me = Rc::clone(self);
        glib::timeout_add_local(HOLE_POLL, move || {
            if !me.tap_opened.get() || !me.osd.is_visible() {
                me.watching_hole.set(false);
                me.last_hole.set(None);
                return glib::ControlFlow::Break;
            }
            let Some(hole) = catcher_hole(&me.osd, &me.view.root) else {
                return glib::ControlFlow::Continue;
            };
            if me.last_hole.get() != Some(hole) {
                me.last_hole.set(Some(hole));
                let close = Rc::clone(&me);
                me.osd.tap_catcher(Some(hole), move || close.close());
            }
            glib::ControlFlow::Continue
        });
    }

    /// Stop watching the card's rectangle and take the catcher away.
    fn clear_catcher(self: &Rc<Self>) {
        self.tap_opened.set(false);
        self.last_hole.set(None);
        self.osd.tap_catcher(None, || {});
    }

    fn close(self: &Rc<Self>) {
        // A pending timer is made harmless rather than removed - see `timer` - and
        // a walk still in flight belongs to a card that is gone.
        self.idle.cancel();
        self.debounce.cancel();
        self.next_seq();
        // A card cannot be gone while its tap catcher still dims the screen;
        // `Osd::hide` takes the catcher down as a backstop, and this keeps the
        // flags in step with what is on screen.
        self.clear_catcher();
        self.osd.hide();
    }

    /// Re-scan the installed applications and re-run the current query.
    fn refresh(self: &Rc<Self>) {
        *self.apps.borrow_mut() = apps::scan();
        let query = self.query.borrow().clone();
        self.changed(&query);
    }

    /// The query changed: match applications now, and start a (debounced) walk
    /// for the file half.
    fn changed(self: &Rc<Self>, query: &str) {
        *self.query.borrow_mut() = query.to_string();

        let trimmed = query.trim().to_string();
        if !self.settings.files || trimmed.is_empty() {
            // Nothing to walk: forget the last walk's answer, make sure an
            // in-flight one cannot land on top of it later, and retire any
            // debounce that is still waiting to start a walk.
            self.next_seq();
            self.file_results.borrow_mut().clear();
            self.debounce.cancel();
        } else {
            let me = Rc::clone(self);
            self.debounce.arm(self.settings.debounce, move || {
                me.start_file_search(&trimmed);
            });
        }

        self.recompute();
        self.arm_idle();
    }

    /// Build the result list for the current query and draw it.
    fn recompute(self: &Rc<Self>) {
        let query = self.query.borrow().trim().to_lowercase();
        let results = {
            let apps = self.apps.borrow();
            results_for(
                &apps,
                &self.file_results.borrow(),
                &query,
                self.settings.files,
                self.settings.max_results,
            )
        };

        // The selection is an index into a list that just changed length.
        let selected = self.selected.get().min(results.len().saturating_sub(1));
        self.selected.set(selected);
        *self.results.borrow_mut() = results;
        self.render();
    }

    fn render(self: &Rc<Self>) {
        let rows: Vec<Row> = self
            .results
            .borrow()
            .iter()
            .map(|item| self.row(item))
            .collect();
        self.view.render(&rows, self.selected.get());
    }

    /// The picture and text for one result.
    fn row(&self, item: &Item) -> Row {
        match item {
            Item::App(index) => {
                let apps = self.apps.borrow();
                let Some(app) = apps.get(*index) else {
                    return empty_row();
                };
                Row {
                    icon: self.app_icon(app),
                    glyph: Stand::Letter(text::initial(&app.name)),
                    title: app.name.clone(),
                    subtitle: app
                        .generic
                        .clone()
                        .or_else(|| app.comment.clone())
                        .unwrap_or_default(),
                }
            }
            Item::File(entry) => Row {
                icon: self.file_icon(entry.is_dir),
                glyph: Stand::Icon(if entry.is_dir {
                    names::FOLDER
                } else {
                    names::FILE
                }),
                title: entry.name.clone(),
                subtitle: shorten_home(&entry.parent),
            },
        }
    }

    fn app_icon(&self, app: &App) -> Option<gdk::Paintable> {
        self.icon(app.icon.as_deref()?)
    }

    fn file_icon(&self, is_dir: bool) -> Option<gdk::Paintable> {
        self.icon(if is_dir { "folder" } else { "text-x-generic" })
    }

    /// A themed icon by name, remembered between keystrokes - the same eight or
    /// so applications answer every letter of the query, and `by_name` walks the
    /// icon theme's directories on a miss.
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

    /// Select a result, if there is one.
    fn select(&self, index: usize) {
        if index >= self.results.borrow().len() {
            return;
        }
        self.selected.set(index);
        self.view.select(index);
    }

    /// Walk the results, wrapping around either end.
    fn step(self: &Rc<Self>, delta: i32) {
        let count = self.results.borrow().len() as i32;
        if count == 0 {
            return;
        }
        self.select((self.selected.get() as i32 + delta).rem_euclid(count) as usize);
        self.arm_idle();
    }

    fn activate_selected(self: &Rc<Self>) {
        self.activate(self.selected.get());
    }

    /// Open the result at `index` and take the card down.
    ///
    /// The card goes first: an application can take a moment to appear, and
    /// leaving a search bar over it while it does makes the launcher look stuck.
    fn activate(self: &Rc<Self>, index: usize) {
        enum Opening {
            App(usize),
            File(String),
        }
        let opening = match self.results.borrow().get(index) {
            Some(Item::App(index)) => Opening::App(*index),
            Some(Item::File(entry)) => Opening::File(entry.path.clone()),
            None => return,
        };
        self.close();
        match opening {
            Opening::App(index) => {
                let apps = self.apps.borrow();
                match apps.get(index) {
                    Some(app) => apps::launch(app),
                    None => eprintln!("hypr-osd-launcher: the selected application is gone"),
                }
            }
            Opening::File(path) => open_path(&path),
        }
    }

    /// Close the card if nothing happens for a while.
    ///
    /// Escape and the key itself are how the card normally ends, but neither
    /// works after the screen has been locked - a session lock takes the
    /// keyboard from every other surface, this one included, so a card that was
    /// up when the screen locked hears nothing. After a minute of no keys and no
    /// pointer movement it takes itself away; every interaction re-arms it.
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

    // -- the file half, off the main loop ------------------------------------

    /// Bump the request counter and answer with the new value.
    fn next_seq(&self) -> u64 {
        timer::bump(&self.file_seq)
    }

    /// Start a walk for `query` on its own thread.
    ///
    /// A filesystem walk is the one thing here that can take a moment, and a
    /// keystroke must not wait for it: the thread answers over a channel, and
    /// [`Launcher::drain_file_results`] picks the answer up on the main loop.
    fn start_file_search(self: &Rc<Self>, query: &str) {
        let seq = self.next_seq();
        let sender = self.file_tx.clone();
        let query = query.to_string();
        let limit = self.settings.max_results;
        self.file_pending.set(self.file_pending.get() + 1);
        std::thread::spawn(move || {
            let found = files::walk(&query, limit);
            // A send can only fail if the element is gone, which is also when
            // nobody cares.
            let _ = sender.send((seq, found));
        });
        self.ensure_poll();
    }

    fn ensure_poll(self: &Rc<Self>) {
        if self.polling.get() {
            return;
        }
        self.polling.set(true);
        let me = Rc::clone(self);
        // Repeating, and it stops itself: returning `Break` when the last walk
        // has answered destroys the source, which is why there is no `SourceId`
        // to keep (and none to remove - see `timer`).
        glib::timeout_add_local(Duration::from_millis(POLL_MS), move || {
            me.drain_file_results()
        });
    }

    /// Take every answer that has arrived. The timer stops once no walk is left,
    /// so a closed launcher asks for nothing.
    fn drain_file_results(self: &Rc<Self>) -> glib::ControlFlow {
        loop {
            let received = self.file_rx.borrow().try_recv();
            match received {
                Ok((seq, found)) => {
                    self.file_pending
                        .set(self.file_pending.get().saturating_sub(1));
                    // Only the newest query may paint: an older walk that took
                    // longer must not replace what the user is looking at.
                    if seq == self.file_seq.get() {
                        *self.file_results.borrow_mut() = found;
                        self.recompute();
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.file_pending.set(0);
                    break;
                }
            }
        }
        if self.file_pending.get() == 0 {
            self.polling.set(false);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    }

    // -- what the verbs answer without drawing a card ------------------------

    fn status_lines(&self) -> String {
        let apps = self.apps.borrow().len();
        let roots: Vec<String> = files::search_roots()
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .collect();
        let files = if self.settings.files {
            format!("{} roots ({})", roots.len(), roots.join(", "))
        } else {
            "off".to_string()
        };
        let state = if self.osd.is_visible() {
            "open"
        } else {
            "closed"
        };
        format!(
            "apps {apps}\nfiles {files}\nstate {state}\nwidth {} max_results {} idle_close_ms {}\n\
             measured: {}",
            self.settings.width,
            self.settings.max_results,
            self.settings
                .idle_close
                .map_or(0, |duration| duration.as_millis()),
            self.view.measured(),
        )
    }

    /// The ranked results for `query`, without a card - for a script, and for
    /// working out why the card draws what it draws.
    ///
    /// The file half is walked here and now rather than waited for: a verb is
    /// allowed to take the tens of milliseconds a walk costs, and an answer that
    /// is already complete when it is printed is the whole point of the verb.
    fn search_lines(&self, query: &str) -> String {
        let query = query.trim().to_lowercase();
        let walked = if self.settings.files && !query.is_empty() {
            files::walk(&query, self.settings.max_results)
        } else {
            Vec::new()
        };
        let apps = self.apps.borrow();
        let items = results_for(
            &apps,
            &walked,
            &query,
            self.settings.files,
            self.settings.max_results,
        );
        if items.is_empty() {
            return "no matches".to_string();
        }
        items
            .iter()
            .map(|item| match item {
                Item::App(index) => format!(
                    "app  {} \u{2014} {}",
                    apps[*index].name,
                    apps[*index].desktop_path.display()
                ),
                Item::File(entry) => format!("file {} \u{2014} {}", entry.name, entry.path),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn app_lines(&self) -> String {
        let apps = self.apps.borrow();
        if apps.is_empty() {
            return "no applications".to_string();
        }
        apps.iter()
            .map(|app| {
                format!(
                    "{} [{}] \u{2014} {}",
                    app.name,
                    app.id,
                    app.desktop_path.display()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A small bonus for an application over a file at the same score: this is an
/// application launcher, and when an application and a file are equally good
/// answers the application is the one that was asked for.
const APP_BIAS: u32 = 25;

/// The rows for `query`: applications and files scored on one ladder, best
/// first, capped at `limit`.
///
/// Merged by score rather than laid end to end with the applications first,
/// because the two lists are not the same *kind* of answer: a file whose name
/// starts with what you typed is a better answer than an application that merely
/// mentions it. See [`hypr_osd_core::apps::score`] for the ladder and the bug
/// that made this
/// obvious.
///
/// Pure, so the ranking can be tested without a card: `recompute` calls it with
/// the file matches a walk delivered, `search_lines` with the ones a synchronous
/// walk just returned.
fn results_for(
    apps: &[App],
    files: &[FileEntry],
    query: &str,
    search_files: bool,
    limit: usize,
) -> Vec<Item> {
    // (score, lowercased name to break ties with, the row itself)
    let mut scored: Vec<(u32, String, Item)> = Vec::new();

    if query.is_empty() {
        // Before anything is typed the card is a list of what is installed.
        for (index, app) in apps.iter().enumerate() {
            scored.push((0, app.name.to_lowercase(), Item::App(index)));
        }
    } else {
        for (index, app) in apps.iter().enumerate() {
            let score = apps::score(app, query);
            if score > 0 {
                scored.push((score + APP_BIAS, app.name.to_lowercase(), Item::App(index)));
            }
        }
    }

    if search_files && !query.is_empty() {
        for entry in files {
            scored.push((
                files::rank(entry, query),
                entry.name.to_lowercase(),
                Item::File(entry.clone()),
            ));
        }
    }

    scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, _, item)| item)
        .collect()
}

/// A placeholder row: only reachable if a list and its backing data drifted
/// apart, and better than a panic in a daemon that lives for the session.
fn empty_row() -> Row {
    Row {
        icon: None,
        glyph: Stand::Letter("?".to_string()),
        title: String::new(),
        subtitle: String::new(),
    }
}

/// A path with `$HOME` spelled `~`, which is what a person reads.
fn shorten_home(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

/// Open a file or folder with whatever the desktop uses for it.
fn open_path(path: &str) {
    if hardware::installed("gio") {
        hardware::launch("gio", &["open", path]);
    } else {
        hardware::launch("xdg-open", &[path]);
    }
}

/// The card's rectangle on its monitor - the hole the tap catcher leaves open.
///
/// The card is centred (`Placement::Center`), so its rectangle follows from its
/// own measured size and the monitor's: the transparent shadow frame the shell
/// keeps around it cancels out of the centring, which leaves the visible card
/// exactly in the middle. Measured, not planned, because the `touch` class and
/// the result list's height both decide the card's height at run time.
fn catcher_hole(osd: &Osd, root: &gtk::Box) -> Option<Hole> {
    let (width, height) = (root.width(), root.height());
    if width <= 0 || height <= 0 {
        return None;
    }
    let (monitor_width, monitor_height) = monitor_size(osd)?;
    Some(centre_hole(
        (width + 2 * CARD_PAD_X, height + 2 * CARD_PAD_Y),
        (monitor_width, monitor_height),
    ))
}

/// The arithmetic of [`catcher_hole`], split out so the rectangle can be tested
/// without a display: a card of `card` centred on a monitor of `monitor`.
fn centre_hole(card: (i32, i32), monitor: (i32, i32)) -> Hole {
    Hole {
        x: (monitor.0 - card.0) / 2,
        y: (monitor.1 - card.1) / 2,
        width: card.0,
        height: card.1,
    }
}

/// The logical size of the monitor the card is on.
///
/// Asked of GDK rather than of the compositor: the card is a layer surface on
/// that monitor, and the hole has to be in the same coordinates the catcher
/// surface is laid out in - which is the monitor's own, starting at its corner.
/// GDK's geometry is already in the logical pixels this is all measured in.
fn monitor_size(osd: &Osd) -> Option<(i32, i32)> {
    let monitor = osd
        .monitor()
        .as_deref()
        .and_then(monitors::find)
        .or_else(monitors::first)?;
    let geometry = monitor.geometry();
    Some((geometry.width(), geometry.height()))
}

/// The connector a `tap`/`open` named, if it named one: a gesture that knows
/// which screen it happened on sends it first, and the numbers of a bar-style
/// rectangle may follow (the launcher is centred, so a rectangle means nothing
/// here - it is accepted so the verb's shape matches the other panels').
fn parse_connector(args: &[String]) -> Option<String> {
    let number_start = args.iter().position(|arg| arg.parse::<f64>().is_ok());
    match number_start {
        Some(index) => index
            .checked_sub(1)
            .and_then(|index| args.get(index))
            .cloned(),
        None => args.first().filter(|arg| !arg.is_empty()).cloned(),
    }
}

/// Say once, on stderr, that this tablet is being typed into with no on-screen
/// keyboard installed.
///
/// The card still focuses the field and still takes the keyboard - there is
/// just nothing that can put a character in it, and a user tapping keys on a
/// screen that ignores them deserves better than silence. Once per process: the
/// card opens many times, and the same complaint on every open would be noise.
fn warn_missing_osk() {
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        if let Some(message) = touch::warn_missing(touch::osk()) {
            eprintln!("hypr-osd-launcher: {message}");
        }
    });
}

fn main() -> glib::ExitCode {
    let config = Config::load(ELEMENT);
    let settings = Rc::new(Settings::load(&config));

    let opts = Opts {
        app_id: APP_ID.to_string(),
        css: css::stylesheet(include_str!("launcher.css")),
        namespace: NAMESPACE.to_string(),
        width: settings.width,
        // Centred: a launcher is a dialog you summoned, not a notification that
        // arrives at the bottom of the screen.
        placement: Placement::Center,
        // It has to be typed into - see the module comment.
        keyboard: Keyboard::Exclusive,
        // Unused by a centred card, kept at the shell's default.
        bottom_margin: 0.12,
        // Cards follow the focused monitor; only the bar pins itself.
        ..Opts::default()
    };

    // The widgets are built inside the application's start-up (GTK does not
    // exist before that) and the command handler is installed before it, so the
    // two meet here. Built once: the scan, the selection and the list have to
    // survive between commands, because `toggle` twice is one card opening and
    // closing, not two cards.
    let state: Rc<OnceCell<Rc<Launcher>>> = Rc::new(OnceCell::new());

    let build = {
        let state = state.clone();
        let settings = settings.clone();
        Box::new(move |osd: &Rc<Osd>| {
            let view = Rc::new(LauncherView::new(
                settings.width,
                settings.icon_size,
                settings.max_results,
            ));
            let launcher = Launcher::new(osd, view.clone(), settings.clone());

            // The scan is the one slow-ish thing in here (a few hundred small
            // files), and it happens once, while nobody is waiting: at start-up,
            // so the first `SUPER + SPACE` draws immediately.
            *launcher.apps.borrow_mut() = apps::scan();
            launcher.changed("");

            view.on_changed({
                let launcher = launcher.clone();
                move |query| launcher.changed(query)
            });
            view.on_activate({
                let launcher = launcher.clone();
                move || launcher.activate_selected()
            });
            view.on_click({
                let launcher = launcher.clone();
                move |index| launcher.activate(index)
            });
            // Moving the pointer over the card is looking at it, a tap on it is
            // using it and a scroll is reading it: none of those is being idle
            // (see `view::LauncherView::on_activity`). Without this the card
            // would close itself after `idle_close_ms` while a finger is still
            // moving through the results.
            view.on_activity({
                let launcher = launcher.clone();
                move || launcher.arm_idle()
            });

            // The search field is the one thing here that is typed into: a
            // focused field is what raises the on-screen keyboard on a tablet
            // (the daemon is started and stopped by the bar, see `core::touch`),
            // and the attach is what makes the focus do it.
            if touch::usable() {
                touch::osk().attach(&view.entry);
            }

            install_keys(osd, launcher.clone());
            let root = view.root.clone().upcast::<gtk::Widget>();
            state.set(launcher).ok();
            Content::Single(root)
        })
    };

    let handle = {
        let state = state.clone();
        Rc::new(
            move |_osd: &Rc<Osd>, args: &[String]| -> Result<String, String> {
                let launcher = state
                    .get()
                    .ok_or("the launcher has not finished starting up")?;
                let Some(verb) = args.first().map(String::as_str) else {
                    // No verb: started by Hyprland's autostart, so wait for the
                    // first press of SUPER + SPACE.
                    return Ok(String::new());
                };
                match verb {
                    "toggle" => launcher.toggle(),
                    "show" => launcher.open(args.get(1).map(String::as_str)),
                    // A finger opened it: the card gets a tap catcher as its way
                    // out (see [`Launcher::tap`]). `open` is the same request
                    // under the name the other panels use for a system gesture.
                    "tap" | "open" => launcher.tap(parse_connector(&args[1..]).as_deref()),
                    "hide" | "close" => launcher.close(),
                    "refresh" => launcher.refresh(),
                    "status" => return Ok(launcher.status_lines()),
                    "apps" => return Ok(launcher.app_lines()),
                    "search" => {
                        let query = args[1..].join(" ");
                        return Ok(launcher.search_lines(&query));
                    }
                    other => {
                        return Err(format!(
                            "unknown verb `{other}` (toggle | show [query] | tap [connector] | \
                             hide | refresh | search <query> | apps | status)"
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
/// Installed through the shell rather than on the entry here, because the shell
/// rebuilds the surface when the card moves to another monitor - a keyboard card
/// that lost its keys after a monitor change would be a bug nobody could
/// explain.
///
/// The handler takes only what the *card* has an opinion about, and lets
/// everything else through: text is the entry's job, and swallowing letters here
/// would be the one thing that could make a search bar unsearchable.
fn install_keys(osd: &Rc<Osd>, launcher: Rc<Launcher>) {
    osd.on_keys(
        {
            let launcher = launcher.clone();
            move |key, state| {
                if !launcher.osd.is_visible() {
                    return glib::Propagation::Proceed;
                }
                // With Ctrl held, the arrows and Home/End are the entry's own
                // editing keys (word-wise movement, jump to the ends), not the
                // list's.
                let control = state.contains(gdk::ModifierType::CONTROL_MASK);
                // `gdk::Key` is a newtype around the keyval rather than an enum,
                // so the arms below match on the value, not on a reference.
                let key = *key;
                let page = launcher.settings.page();
                let handled = match key {
                    gdk::Key::Return | gdk::Key::KP_Enter => {
                        launcher.activate_selected();
                        true
                    }
                    gdk::Key::Escape => {
                        launcher.close();
                        true
                    }
                    // Tab has nowhere to go: the field is the only focusable
                    // widget in the card, and letting GTK walk would be a
                    // keystroke that visibly does nothing.
                    gdk::Key::Tab | gdk::Key::ISO_Left_Tab => true,
                    gdk::Key::Down if !control => {
                        launcher.step(1);
                        true
                    }
                    gdk::Key::Up if !control => {
                        launcher.step(-1);
                        true
                    }
                    gdk::Key::Page_Down if !control => {
                        launcher.step(page);
                        true
                    }
                    gdk::Key::Page_Up if !control => {
                        launcher.step(-page);
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
        // No release handling: this card is a place you leave, not a key you
        // hold (unlike the switcher).
        |_| {},
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, keywords: &[&str]) -> App {
        App {
            id: name.to_lowercase(),
            name: name.to_string(),
            keywords: keywords.iter().map(|value| value.to_string()).collect(),
            ..App::default()
        }
    }

    /// The rows as `app:Name` / `file:name`, which is all a ranking test needs to
    /// see.
    fn rows(items: &[Item], apps: &[App]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                Item::App(index) => format!("app:{}", apps[*index].name),
                Item::File(entry) => format!("file:{}", entry.name),
            })
            .collect()
    }

    #[test]
    fn an_empty_query_lists_everything_in_name_order() {
        let apps = vec![app("Zed", &[]), app("alacritty", &[]), app("Mpv", &[])];
        let items = results_for(&apps, &[], "", false, 10);
        assert_eq!(
            rows(&items, &apps),
            vec!["app:alacritty", "app:Mpv", "app:Zed"]
        );
    }

    #[test]
    fn a_query_ranks_the_best_match_first_and_drops_the_rest() {
        let apps = vec![
            app("Firefox", &["Internet"]),
            app("Files", &[]),
            app("Font Manager", &[]),
        ];
        assert_eq!(
            rows(&results_for(&apps, &[], "fire", true, 10), &apps)[0],
            "app:Firefox"
        );
        // A keyword still finds its application.
        assert_eq!(
            rows(&results_for(&apps, &[], "internet", true, 10), &apps),
            vec!["app:Firefox"]
        );
        // Nothing matches at all.
        assert!(results_for(&apps, &[], "zzz", true, 10).is_empty());
    }

    #[test]
    fn a_file_whose_name_matches_outranks_an_application_that_merely_mentions_it() {
        // The regression this ranking exists for: `icf` used to fill the list with
        // applications whose comments happened to contain the letters, and never
        // reach the file.
        let apps = vec![app("Grand Editor", &["icf"]), app("Files", &[])];
        let file = FileEntry {
            name: "icf-notes.pdf".to_string(),
            path: "/home/tester/Documents/icf-notes.pdf".to_string(),
            parent: "/home/tester/Documents".to_string(),
            is_dir: false,
        };
        let items = results_for(&apps, std::slice::from_ref(&file), "icf", true, 10);
        assert_eq!(rows(&items, &apps)[0], "file:icf-notes.pdf");
    }

    #[test]
    fn the_list_is_capped_and_files_are_skipped_when_they_are_switched_off() {
        let apps = vec![app("Alpha", &[]), app("Beta", &[])];
        assert_eq!(results_for(&apps, &[], "a", true, 1).len(), 1);
        let file = FileEntry {
            name: "alpha.txt".to_string(),
            path: "/home/tester/alpha.txt".to_string(),
            parent: "/home/tester".to_string(),
            is_dir: false,
        };
        // With `files = false` the file never appears, even when it ranks first.
        let items = results_for(&apps, std::slice::from_ref(&file), "alpha", false, 10);
        assert_eq!(rows(&items, &apps), vec!["app:Alpha"]);
    }

    #[test]
    fn home_is_shortened_to_a_tilde() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(shorten_home(&format!("{home}/Documents")), "~/Documents");
        assert_eq!(shorten_home("/usr/share"), "/usr/share");
    }

    /// The tap catcher leaves the *card* open, and the card is centred: its hole
    /// is the measured card in the middle of the monitor. Getting this wrong
    /// leaves half the card dimmed by its own catcher or a strip of screen
    /// beside it that swallows taps.
    #[test]
    fn the_catcher_hole_is_the_card_in_the_middle_of_the_monitor() {
        // This machine's screen, 2736x1824 at scale 2, and a 640px card.
        assert_eq!(
            centre_hole((640 + 2 * 16, 300 + 2 * 13), (1368, 912)),
            Hole {
                x: (1368 - 672) / 2,
                y: (912 - 326) / 2,
                width: 672,
                height: 326,
            }
        );
    }

    /// A gesture that knows its screen sends the connector first; the numbers of
    /// a bar-style rectangle may follow and mean nothing here (the card is
    /// centred). Both shapes have to be accepted, because both reach this binary.
    #[test]
    fn a_tap_names_a_connector_with_or_without_a_rectangle() {
        let words = |args: &[&str]| args.iter().map(|word| word.to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse_connector(&words(&["eDP-1"])).as_deref(),
            Some("eDP-1")
        );
        assert_eq!(
            parse_connector(&words(&["eDP-1", "0", "5", "44", "28"])).as_deref(),
            Some("eDP-1")
        );
        // A hand-typed rectangle without a connector says nothing about screens.
        assert_eq!(parse_connector(&words(&["0", "5", "44", "28"])), None);
        assert_eq!(parse_connector(&[]), None);
    }
}

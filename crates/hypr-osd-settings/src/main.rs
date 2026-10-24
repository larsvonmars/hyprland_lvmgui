//! `hypr-osd-settings` - element #8: the settings window.
//!
//! Every other element reads its own `~/.config/hypr-osd/<element>.conf` once, at
//! start-up, and carries the defaults for it in its source. That makes the config
//! files the real surface of this desktop - and leaves them to be edited by hand,
//! which is fine for one or two keys and tedious for a machine.
//!
//! This element is the other way in: one card with a page per element, a control
//! per key, and the file rewritten as the values change. It knows nothing about
//! the elements themselves - the keys, their labels and their defaults all come
//! from [`hypr_osd_core::settings`], the catalogue in the shared crate, so this
//! window and the elements cannot disagree about what a key is called or what it
//! is worth unset.
//!
//! Three things about how it behaves are deliberate:
//!
//! * **It writes at once, and says so.** Every change goes straight to the file
//!   ([`config::edit_value`], which touches one line); the footer says that, and
//!   that an element reads its config at start-up - so a value takes effect when
//!   that element is restarted, which is what the `Restart` chip on each page is
//!   for. Applying settings by itself would mean reaching into another process,
//!   and there is no honest way to do that from here.
//! * **It takes the keyboard while it is up.** The text fields have to be typed
//!   into, and a layer-shell surface has no other way to be typed into (see
//!   `Keyboard::Exclusive`). Escape closes it, and it lets go of the keyboard by
//!   itself after `idle_close_ms` of silence, which is what makes that safe
//!   across a screen lock.
//! * **It is opened by the gear in the system popup's header.** The popup is its
//!   trigger, in the same way the bar's grid button opens the applications panel:
//!   this element has no keybinding of its own.
//!
//! Verbs (`hypr-osd-settings <verb>`):
//!
//! ```text
//!   toggle               open the window, or close it again (what the gear runs)
//!   show [section]       open it, on the first page or the one named
//!   hide | close         take it away
//!   section <name>       open it on that page ("bar", "System popup", or a number)
//!   set <element> <key> <value>
//!                        write one value, exactly as the window would
//!   unset <element> <key>
//!                        drop the key, so the built-in default applies again
//!   restart <element>    stop that element and start it again
//!   command <id>         run one of the shown page's own buttons, as a click
//!                        would (the System tab's `apply` and `restore`)
//!   drop <output> <dx> <dy>
//!                        where a drag of that many canvas pixels would put an
//!                        output, without moving anything (the Display page)
//!   sections             list the pages
//!   keys <element>       what every catalogue key is set to
//!   status               the pages, the files, and whether the window is up
//!   (no verb)            start the daemon and wait for the gear
//! ```

mod arrange;
mod view;

use std::cell::{Cell, OnceCell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::icons::names;
use hypr_osd_core::settings::{self as catalogue, Applies, Kind, MonitorList, Section};
use hypr_osd_core::timer::Timer;
use hypr_osd_core::{
    config, css, desktop, energy, hardware, hyprctl, keymap, lock, monitors, run, state, Config,
    Content, Keyboard, Opts, Osd, Placement, CARD_PAD_X,
};

use view::{Field, Heading, PageAction, SectionRow, SettingsView, TabRow};

/// D-Bus application id - and therefore the single-instance key: the gear's
/// `toggle` reaches the running window instead of starting a second one.
const APP_ID: &str = "com.schells2.osd.settings";
/// Layer-shell namespace, the collection's shared prefix.
const NAMESPACE: &str = "hypr-osd";
/// Element name: `~/.config/hypr-osd/settings.conf`.
const ELEMENT: &str = "settings";

// Defaults; every one is overridable in the config file.
const DEFAULT_WIDTH: i32 = 760;
const DEFAULT_SIDEBAR_WIDTH: i32 = 190;
const DEFAULT_PANE_HEIGHT: i32 = 380;
const DEFAULT_IDLE_CLOSE_MS: u64 = 120_000;
/// Where the element binaries live, for the restart buttons. The installer copies
/// them to `~/.local/bin`, which a session started by a display manager usually
/// does not have on its `PATH` - the same reason the bar keeps `apps_command` as
/// an absolute path.
const DEFAULT_BIN_DIR: &str = "~/.local/bin";

/// How often a restart tries to start an element that came up as a mere
/// forwarder of the process that was still going away (see [`Panel::restart`]).
const RESTART_ATTEMPTS: i32 = 5;

/// What the config file resolved to.
struct Settings {
    width: i32,
    sidebar_width: i32,
    /// The height of the scrolling pane - and therefore the height the card
    /// settles at, whatever page is on it.
    pane_height: i32,
    /// Close after this long without a key press or pointer movement (`0` never).
    idle_close: Option<Duration>,
    bin_dir: PathBuf,
    /// The daemon the System tab's buttons run.
    energy_command: String,
}

impl Settings {
    fn load(config: &Config) -> Self {
        let idle = config.millis("idle_close_ms", DEFAULT_IDLE_CLOSE_MS);
        Settings {
            width: config.i32("width", DEFAULT_WIDTH).max(480),
            sidebar_width: config
                .i32("sidebar_width", DEFAULT_SIDEBAR_WIDTH)
                .clamp(120, 320),
            pane_height: config.i32("pane_height", DEFAULT_PANE_HEIGHT).max(160),
            idle_close: if idle.is_zero() { None } else { Some(idle) },
            bin_dir: expand(&config.string("bin_dir", DEFAULT_BIN_DIR)),
            // Empty means "the energy element on `bin_dir`" - resolved at click
            // time the same way the Restart chip resolves an element's binary.
            energy_command: config.string("energy_command", ""),
        }
    }

    /// The width the scrolling pane gets: the card, less the shell's padding, the
    /// sidebar, the gap between the two panes and this window's own 2px inset
    /// (see `box.settings` in `settings.css`).
    fn pane_width(&self) -> i32 {
        (self.width - 2 * CARD_PAD_X - self.sidebar_width - view::BODY_GAP - 4).max(240)
    }
}

/// Expand a leading `~/` (a config file cannot hold `$HOME`, and a command is not
/// worth a shell for one path).
fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// The window's state: which page is up, and the watchdog that ends it.
struct Panel {
    osd: Rc<Osd>,
    view: Rc<SettingsView>,
    settings: Rc<Settings>,
    /// The tab being shown, as an index into [`catalogue::TABS`].
    tab: Cell<usize>,
    /// The page being shown within that tab.
    section: Cell<usize>,
    /// What the last `apply` did, for `status` - the one thing here that goes
    /// through another program and can therefore be refused.
    applied: RefCell<String>,
    /// The Display page's arrangement editor, kept across re-renders: a drop or
    /// a popover write re-fits it in place ([`Arranger::update`]) rather than
    /// rebuilding the canvas and asking `hyprctl` again. `None` on every other
    /// page.
    arranger: RefCell<Option<Rc<arrange::Arranger>>>,
    /// The per-output editor popover, parented to the box a click chose.
    arrange_popover: RefCell<Option<gtk::Popover>>,
    /// The idle-close watchdog (see [`Panel::arm_idle`]).
    idle: Rc<Timer>,
}

impl Panel {
    fn new(osd: &Rc<Osd>, view: Rc<SettingsView>, settings: Rc<Settings>) -> Rc<Self> {
        Rc::new(Panel {
            osd: osd.clone(),
            view,
            settings,
            tab: Cell::new(0),
            section: Cell::new(0),
            applied: RefCell::new("not applied yet".to_string()),
            arranger: RefCell::new(None),
            arrange_popover: RefCell::new(None),
            idle: Rc::new(Timer::new()),
        })
    }

    /// The pages of the tab being shown.
    fn pages(&self) -> &'static [Section] {
        catalogue::TABS[self.tab.get()].sections
    }

    /// The page being shown.
    fn page(&self) -> &'static Section {
        &self.pages()[self.section.get()]
    }

    /// Open the window, or close it if it is already up - what the gear runs,
    /// because one click should be able to undo itself.
    fn toggle(self: &Rc<Self>) {
        if self.osd.is_visible() {
            self.close();
        } else {
            self.open(None, None);
        }
    }

    /// Open the window on a tab and page (the ones already showing by default),
    /// and start the watchdog.
    ///
    /// The page is re-read from disk every time it is shown, so a file edited by
    /// hand while the window was down is what the window shows - which is the one
    /// thing a settings window must not get wrong.
    fn open(self: &Rc<Self>, tab: Option<usize>, page: Option<usize>) {
        self.osd.set_content(&self.view.root);
        if let Some(tab) = tab {
            self.tab.set(tab.min(catalogue::TABS.len() - 1));
            self.section.set(0);
        }
        self.show_section(page.unwrap_or(self.section.get()));
        self.osd.show();
        // The gear in the popup reads this to stay lit while the window is up
        // (see `state`).
        state::write(&state::settings_panel(), true);
        self.arm_idle();
    }

    fn close(self: &Rc<Self>) {
        self.idle.cancel();
        state::write(&state::settings_panel(), false);
        self.osd.hide();
    }

    /// Switch to a tab and show its first page - what a tab button does.
    fn open_tab(self: &Rc<Self>, index: usize) {
        self.tab.set(index.min(catalogue::TABS.len() - 1));
        self.section.set(0);
        self.show_section(0);
    }

    /// Draw one tab: the tab row, the sidebar, and the rows of the page as the
    /// file has them now.
    fn show_section(self: &Rc<Self>, index: usize) {
        let pages = self.pages();
        let index = index.min(pages.len() - 1);
        self.section.set(index);
        let section = &pages[index];
        let config = Config::load(section.file);

        // The Hyprland pages ask the compositor what each option is worth *now*,
        // so a value set in the user's own `hyprland.lua` is visible instead of
        // looking like the default this file would impose. That is one `hyprctl`
        // per row, on the way into a page: a socket question each, and a page is
        // drawn on a click.
        let live = section.applies == Applies::Compositor;
        // The Energy page opens with what the machine is doing and then the two
        // buttons that ask the daemon to act *now* - both above the policy rows,
        // because a button below a page's fold is a button nobody finds. Neither
        // is a key in the file, so both are made here rather than coming out of
        // the catalogue.
        //
        // The Shortcuts page is the other kind of exception: its rows are not
        // settings at all, so it never reads a file - it is the catalogue's
        // `SHORTCUTS` table drawn as rows (see `shortcut_fields`).
        // The Display page's rows and its editor both describe what the
        // compositor reports; ask once and hand the same answer to both, so a
        // page render is one `hyprctl -j monitors` rather than two.
        let displays = if section.applies == Applies::Displays {
            monitors::list()
        } else {
            Vec::new()
        };
        let mut fields = match section.applies {
            Applies::Energy => energy_readings(),
            Applies::Shortcuts => shortcut_fields(),
            _ => Vec::new(),
        };
        if section.applies == Applies::Energy {
            fields.push(energy_actions());
        }
        match section.applies {
            Applies::Displays => fields.extend(display_fields(&config, &displays)),
            Applies::Shortcuts => {}
            _ => fields.extend(fields_of(&config, section, live)),
        }

        // What the page's chip offers. The settings window's own page gets none:
        // a window cannot stop and start itself from a click inside itself - and
        // the Shortcuts page has nothing to make take effect.
        let action = if section.file == ELEMENT {
            PageAction::None
        } else {
            match section.applies {
                Applies::Element | Applies::Energy => PageAction::Restart,
                Applies::Compositor | Applies::Displays => PageAction::Apply,
                // The lock screen is *generated*: there is no element to restart
                // and no compositor to hand a document to, so the chip rewrites
                // the variables (which is what a change does too - it is there
                // for a `theme.css` that was edited by hand).
                Applies::Lock => PageAction::Regenerate,
                Applies::Shortcuts => PageAction::None,
            }
        };

        // The Display page opens with the outputs as a picture: a monitor's
        // position is a pair of numbers, and dragging one is the only way to
        // choose it that does not require doing arithmetic in your head. On
        // every other page the editor and its popover are put away - a popover
        // hanging off a box that is no longer drawn has to go with it.
        let banner = if section.applies == Applies::Displays {
            self.arrangement(&displays)
        } else {
            self.clear_arrangement();
            None
        };

        self.view.render_tabs(&self.tab_rows());
        self.view.render_sections(&self.section_rows());
        self.view
            .show_section(section, &fields, action, banner.as_ref());
    }

    /// The Display page's arrangement editor, wired to [`Panel::changed`]: a
    /// screen dropped somewhere new writes `monitor:<output>:position`, which is
    /// handed to the compositor exactly like a row's value - and a click on a
    /// box opens the per-output popover on it.
    ///
    /// The editor is *kept*: a re-render of the page re-fits the canvas that
    /// already exists ([`Arranger::update`]) rather than building a new one, so
    /// the picture does not flicker and a popover hanging off a box keeps its
    /// parent. Only the first render of the page builds it.
    ///
    /// The write is deferred by one turn of the main loop on purpose: it redraws
    /// the page, and the widget that is redrawn away is the one whose drag
    /// gesture is still unwinding. Letting the gesture finish first is what keeps
    /// the write from pulling the ground out from under it.
    fn arrangement(self: &Rc<Self>, displays: &[monitors::Display]) -> Option<gtk::Widget> {
        if let Some(arranger) = self.arranger.borrow().as_ref() {
            arranger.update(displays);
            return Some(arranger.root().clone().upcast());
        }
        let me = Rc::clone(self);
        let on_move = move |output: &str, position: &str| {
            let key = format!("monitor:{output}:position");
            let value = position.to_string();
            let me = Rc::clone(&me);
            glib::timeout_add_local_once(Duration::ZERO, move || {
                // `changed` applies the write and then re-fits the editor and
                // redraws the rows itself (see `refresh_displays`).
                me.changed(desktop::FILE, &key, Some(&value));
            });
        };
        let me = Rc::clone(self);
        let on_select = move |output: &str, item: &gtk::Widget| {
            me.open_arrange_popover(output, item);
        };
        // The pane, less the inset the rows keep from its edge.
        let width = (self.settings.pane_width() - 24).max(200);
        let arranger = arrange::Arranger::new(
            width,
            displays,
            arrange::Handlers {
                on_move: Rc::new(on_move),
                on_select: Rc::new(on_select),
            },
        )?;
        let root = arranger.root().clone().upcast::<gtk::Widget>();
        *self.arranger.borrow_mut() = Some(arranger);
        Some(root)
    }

    /// Put the Display page's editor away: another page is up, and the editor
    /// and its popover belong to that page.
    fn clear_arrangement(&self) {
        self.arranger.take();
        if let Some(popover) = self.arrange_popover.take() {
            popover.popdown();
            popover.unparent();
        }
    }

    /// After a Display write: re-read the compositor, re-fit the editor's
    /// picture, redraw the rows, and refill the output popover if one is open -
    /// so a new mode or scale re-sizes the box at once, and the popover shows
    /// what is in force instead of closing. The editor itself is not recreated.
    fn refresh_displays(self: &Rc<Self>) {
        let displays = monitors::list();
        if let Some(arranger) = self.arranger.borrow().as_ref() {
            arranger.update(&displays);
        }
        self.show_section(self.section.get());
        let popover = self.arrange_popover.borrow().clone();
        let Some(popover) = popover.filter(|popover| popover.is_visible()) else {
            return;
        };
        let output = self
            .arranger
            .borrow()
            .as_ref()
            .and_then(|arranger| arranger.selected());
        if let Some(output) = output {
            self.fill_arrange_popover(&popover, &output);
        }
    }

    /// The per-output editor, hung off the box that was clicked: one compact
    /// row per field (dragging owns `position`, so it is not a row here), and
    /// the button that makes this output the primary display.
    fn open_arrange_popover(self: &Rc<Self>, output: &str, item: &gtk::Widget) {
        if let Some(old) = self.arrange_popover.take() {
            old.popdown();
            old.unparent();
        }
        let popover = gtk::Popover::new();
        popover.add_css_class("arrange-pop");
        popover.set_has_arrow(true);
        popover.set_autohide(true);
        popover.set_parent(item);
        self.fill_arrange_popover(&popover, output);
        *self.arrange_popover.borrow_mut() = Some(popover.clone());
        popover.popup();
    }

    /// The popover's rows for one output, built from the same fields the page's
    /// rows are built from. Rebuilt after every write (see `refresh_displays`),
    /// so the open popover shows fresh values instead of closing.
    fn fill_arrange_popover(self: &Rc<Self>, popover: &gtk::Popover, output: &str) {
        let config = Config::load(desktop::FILE);
        let displays = monitors::list();
        let prefix = format!("monitor:{output}:");
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 6);
        rows.add_css_class("arrange-pop-rows");
        for field in display_fields(&config, &displays)
            .iter()
            .filter(|field| field.key.starts_with(&prefix))
        {
            let name = &field.key[prefix.len()..];
            if name == "position" {
                continue;
            }
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("arrange-pop-row");
            // "disabled" is said the other way round here: the switch is
            // "Enabled", because that is the state a person is choosing.
            let label = gtk::Label::new(Some(if name == "disabled" {
                "Enabled"
            } else {
                &field.label
            }));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_tooltip_text(Some(&field.help));
            row.append(&label);
            if name == "disabled" {
                // On when the field is false, and writes the field's own
                // spelling inverted - the same key, the same value, read the
                // other way up.
                let switch = gtk::Switch::new();
                switch.set_active(field.value != "true");
                switch.set_valign(gtk::Align::Center);
                switch.set_tooltip_text(Some(&field.help));
                let me = Rc::clone(self);
                let key = field.key.clone();
                switch.connect_state_set(move |_, active| {
                    me.changed(desktop::FILE, &key, Some(if active { "false" } else { "true" }));
                    glib::Propagation::Proceed
                });
                row.append(&switch);
            } else {
                row.append(&self.view.field_control(desktop::FILE, field));
            }
            rows.append(&row);
        }
        // The page-level key, beside the per-output fields: which output the
        // collection's cards appear on (see `primary_field`).
        let primary = gtk::Button::with_label("Make primary");
        primary.add_css_class("arrange-pop-primary");
        primary.set_tooltip_text(Some(
            "The output this collection's cards appear on - the volume, media and session cards",
        ));
        if config.get(catalogue::PRIMARY_DISPLAY) == Some(output) {
            primary.set_label("Primary display");
            primary.set_sensitive(false);
        } else {
            let me = Rc::clone(self);
            let output = output.to_string();
            primary.connect_clicked(move |_| {
                me.changed(desktop::FILE, catalogue::PRIMARY_DISPLAY, Some(&output));
            });
        }
        rows.append(&primary);
        popover.set_child(Some(&rows));
    }

    /// The tab row, from the catalogue.
    fn tab_rows(&self) -> Vec<TabRow> {
        catalogue::TABS
            .iter()
            .enumerate()
            .map(|(index, tab)| TabRow {
                label: tab.title.to_string(),
                tooltip: tab.blurb.to_string(),
                selected: index == self.tab.get(),
            })
            .collect()
    }

    /// The sidebar, for the tab being shown.
    fn section_rows(&self) -> Vec<SectionRow> {
        self.pages()
            .iter()
            .enumerate()
            .map(|(position, entry)| SectionRow {
                label: entry.title.to_string(),
                icon: entry.icon,
                selected: position == self.section.get(),
            })
            .collect()
    }

    /// A value changed in the window: write it to the file that page belongs to,
    /// and - on a Hyprland page - hand it to the compositor at once.
    ///
    /// `None` drops the key, which is how a value goes back to the built-in
    /// default - and that also means the page has to be drawn again, because the
    /// control has to move back to the default with it.
    ///
    /// A Display write redraws the page either way: a new mode or scale re-sizes
    /// the box in the editor, a drop re-places it, and the popover that is open
    /// on an output has to show the value just written rather than close.
    fn changed(self: &Rc<Self>, file: &str, key: &str, value: Option<&str>) {
        let path = Config::load(file).path;
        if let Err(error) = config::edit_value(&path, key, value) {
            // Writing is the one thing this element does that can fail, and a
            // silent failure would look like a switch that does not stick.
            eprintln!(
                "hypr-osd-settings: cannot write {}: {error}",
                path.display()
            );
            return;
        }

        // A Hyprland setting is live: the document is regenerated from the file
        // and handed over the moment it changes, so the window shows the desktop
        // as it is being adjusted rather than after a login. A Display row is the
        // same - it is applied to the compositor through the same document.
        if matches!(self.page().applies, Applies::Compositor | Applies::Displays) {
            if let Err(error) = self.apply() {
                eprintln!("hypr-osd-settings: {error}");
            }
        }

        // The lock screen has no daemon to restart either: the variables are
        // rewritten as the value is written, and hyprlock reads them at the next
        // lock - which is why that page needs no chip to make a change take
        // effect.
        if self.page().applies == Applies::Lock && self.page().file == file {
            self.regenerate();
        }

        if self.page().applies == Applies::Displays && self.page().file == file {
            self.refresh_displays();
        } else if value.is_none() && self.page().file == file {
            self.show_section(self.section.get());
        }
    }

    /// What the page's chip does - see [`view::PageAction`].
    fn act(&self) {
        let page = self.page();
        match page.applies {
            Applies::Compositor | Applies::Displays => {
                if let Err(error) = self.apply() {
                    eprintln!("hypr-osd-settings: {error}");
                }
            }
            // Regenerating the lock screen's variables is the whole job: it reads
            // the theme and the file and writes what hyprlock will read.
            Applies::Lock => self.regenerate(),
            // The window's own page has no chip at all (see `show_section`).
            Applies::Element | Applies::Energy if page.file != ELEMENT => self.restart(page.file),
            // Nothing to restart and nothing to re-apply: a list of keys.
            Applies::Element | Applies::Energy | Applies::Shortcuts => {}
        }
    }

    /// Rewrite `~/.config/hypr/hyprlock-vars.conf` - the palette and the knobs the
    /// hand-written `hyprlock.conf` is written in terms of (see [`lock`]).
    ///
    /// Writing it *is* making the change take effect, because hyprlock reads its
    /// config at every lock: there is nothing to restart, and no document to hand
    /// to the compositor. What comes of it is kept in `applied`, the same line the
    /// Hyprland tab reports through, so `status` can say whether the last write
    /// worked.
    fn regenerate(&self) {
        match lock::write_vars() {
            Ok(path) => *self.applied.borrow_mut() = format!("wrote {}", path.display()),
            Err(error) => {
                *self.applied.borrow_mut() = format!("failed: {error}");
                eprintln!("hypr-osd-settings: {error}");
            }
        }
    }

    /// One of a page's own buttons ([`catalogue::ENERGY_ACTIONS`]): the System
    /// tab asks the energy daemon to save or restore *now*, rather than waiting
    /// for the next profile poll.
    ///
    /// `hardware::launch`, never `run`: the daemon is already up (it is
    /// autostarted) and this is a verb sent to it, so waiting for an exit would
    /// park this window's main loop - the exact mistake the bar's status pill
    /// made.
    fn command(&self, id: &str) {
        // The element on `bin_dir`, unless a command of its own was configured -
        // the same resolution the Restart chip does, because a session started by
        // a display manager has no `~/.local/bin` on its PATH.
        let program = if self.settings.energy_command.is_empty() {
            self.program("energy")
        } else {
            self.settings.energy_command.clone()
        };
        hardware::launch(&program, &[id]);
    }

    /// Write the Hyprland tab's settings out as Lua and hand them to the
    /// compositor.
    ///
    /// Two steps, and the second is what makes the tab *live*: the document is
    /// generated from the file (so it is always exactly what the file says), and
    /// then `hyprctl eval` runs it in the compositor, which merges the tables into
    /// its options. `dofile` rather than `require`, because the file changes and a
    /// `require` would hand back the first version it ever read.
    ///
    /// `output()` rather than `hardware::run`, because Hyprland's complaint about
    /// a document it will not take (an option this version does not have, say) is
    /// the only thing that makes a failure fixable - and `hyprctl eval` answers a
    /// socket question, so nothing here waits for a program.
    fn apply(&self) -> Result<(), String> {
        let config = Config::load(desktop::FILE);
        // The Hyprland tab's *option* pages only: the Display page's keys are
        // field names, not option paths, and become `hl.monitor` calls below.
        let set: Vec<(&str, Kind, String)> = catalogue::DESKTOP
            .iter()
            .filter(|section| section.applies == Applies::Compositor)
            .flat_map(|section| section.settings.iter())
            .filter_map(|setting| {
                config
                    .get(setting.key)
                    .map(|value| (setting.key, setting.kind, value.to_string()))
            })
            .collect();
        let entries: Vec<(&str, Kind, &str)> = set
            .iter()
            .map(|(key, kind, value)| (*key, *kind, value.as_str()))
            .collect();

        let rules = monitor_rules(&config);
        let monitors: Vec<desktop::MonitorRule> = rules
            .iter()
            .map(|(output, fields)| desktop::MonitorRule {
                output,
                fields: fields
                    .iter()
                    .map(|(key, kind, value)| (key.as_str(), *kind, value.as_str()))
                    .collect(),
            })
            .collect();
        let document = desktop::lua_document(&entries, &monitors);

        let path = self.desktop_lua();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        std::fs::write(&path, document)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;

        let expression = format!("dofile(\"{}\")", path.display());
        match hyprctl::eval(&expression) {
            Ok(said) => {
                *self.applied.borrow_mut() = if said.is_empty() {
                    format!("applied {}", path.display())
                } else {
                    format!("applied {said}")
                };
                Ok(())
            }
            // Hyprland names the option it did not like, which is the whole value
            // of reading its answer rather than its exit status.
            Err(why) => {
                *self.applied.borrow_mut() = format!("failed: {why}");
                Err(format!("Hyprland refused the settings: {why}"))
            }
        }
    }

    /// Where the generated document goes: next to the user's own `hyprland.lua`,
    /// so the whole compositor config is in one directory.
    fn desktop_lua(&self) -> PathBuf {
        config::config_home().join("hypr").join(desktop::LUA)
    }

    /// Stop one element and start it again, so it reads the config just written.
    ///
    /// The awkward part is not the kill, it is doing it from a script that also
    /// names the binary. `pkill -f` matches its pattern against the *whole command
    /// line*, and this shell's own line contains the program it is about to start,
    /// so a pattern that is just the name matches the shell itself: the script dies
    /// at its own first statement (measured: the launcher was killed, nothing was
    /// started, and the trace lines never appeared). Hence the pattern is anchored
    /// at the end of the line (`hypr-osd-launcher$`). An element's own command line
    /// is exactly its path, because they are started without arguments, by the
    /// autostart or by this button; the shell's line has more text after the name
    /// and cannot match. (The README's recipes use the other trick for the same
    /// purpose, a bracket like `hypr-osd-ba[r]`, which cannot work here: this
    /// script has to spell the name out to run it.)
    ///
    /// The second half is the *start*, which is where the rest of the subtlety
    /// is: an element is single-instance by its D-Bus name, so a replacement that
    /// comes up while the process that is going away still holds the name does not
    /// become a daemon at all. It *forwards* its empty command line to the old
    /// instance and exits, leaving nothing running. So the script does not try to
    /// predict when the name is free; it checks the only thing that matters, which
    /// is whether the replacement is still there a moment later, and tries again
    /// if it is not (up to [`RESTART_ATTEMPTS`] times).
    ///
    /// `sh` holds the replacement as its own child and exits without waiting for
    /// it, so the new daemon outlives the script that started it, and the shell
    /// itself is reaped by the shell's own child watch (see `hardware::launch`).
    fn restart(&self, element: &str) {
        if element.is_empty() || element == ELEMENT {
            return;
        }
        let program = self.program(element);
        let pattern = kill_pattern(element);
        let script = format!(
            "pkill -f '{pattern}' >/dev/null 2>&1; \
             attempt=0; \
             while [ \"$attempt\" -lt {attempts} ]; do \
                 attempt=`expr \"$attempt\" + 1`; \
                 sleep 0.25; \
                 '{program}' & \
                 pid=$!; \
                 sleep 0.75; \
                 if kill -0 \"$pid\" 2>/dev/null; then exit 0; fi; \
             done; \
             exit 1",
            attempts = RESTART_ATTEMPTS,
        );
        hardware::launch("sh", &["-c", &script]);
    }

    /// Where an element's binary is: the configured `bin_dir`, or the bare name
    /// (which needs `~/.local/bin` on this process's `PATH`).
    fn program(&self, element: &str) -> String {
        let name = format!("hypr-osd-{element}");
        let candidate = self.settings.bin_dir.join(&name);
        if candidate.is_file() {
            candidate.to_string_lossy().into_owned()
        } else {
            name
        }
    }

    /// Close the window if nothing happens for a while.
    ///
    /// Escape and the close button are how it normally ends, but neither works
    /// after the screen has been locked - a session lock takes the keyboard from
    /// every other surface, this one included, so a window that was up when the
    /// screen locked hears nothing. After `idle_close_ms` of no keys and no
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

    /// Write one value from a verb (`set`/`unset`), apply it if it belongs to
    /// Hyprland, and repaint the page if it is the one showing.
    fn set_value(
        self: &Rc<Self>,
        name: &str,
        key: &str,
        value: Option<&str>,
    ) -> Result<(), String> {
        let page = pages()
            .into_iter()
            .find(|section| section.file == name || group(section) == name)
            .ok_or_else(|| format!("no page named `{name}` (try `sections`)"))?;
        let path = Config::load(page.file).path;
        config::edit_value(&path, key, value)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        // A Hyprland setting is live - it is handed over as soon as it changes -
        // while an element reads its file at start-up. A Display row is a Hyprland
        // setting too: it goes into the same generated document.
        if matches!(page.applies, Applies::Compositor | Applies::Displays) {
            self.apply()?;
        }
        // A lock screen value is live too, by the same mechanism: write the file,
        // rewrite the variables that are generated from it.
        if page.applies == Applies::Lock {
            self.regenerate();
        }
        if self.page().file == page.file {
            self.show_section(self.section.get());
        }
        Ok(())
    }

    // -- what the verbs answer without drawing a window ----------------------

    fn status_lines(&self) -> String {
        let mut out = format!(
            "tab        {} ({} of {})\nshowing    {}\nstate      {}\nsize       {}x{} (sidebar {}, pane {})\nmeasured   {}\ndesktop    {}\nlua        {}\napplied    {}\n",
            catalogue::TABS[self.tab.get()].title,
            self.tab.get() + 1,
            catalogue::TABS.len(),
            self.page().title,
            if self.osd.is_visible() { "open" } else { "closed" },
            self.settings.width,
            self.settings.pane_height,
            self.settings.sidebar_width,
            self.settings.pane_width(),
            self.view.measured(),
            Config::load(desktop::FILE).path.display(),
            self.desktop_lua().display(),
            self.applied.borrow(),
        );
        for tab in catalogue::TABS {
            out.push_str(&format!("\n[{}] {}\n", tab.id, tab.title));
            for section in tab.sections {
                // The Shortcuts page carries no keys and no file, so its line is
                // the count of what it lists instead of a settings count.
                if section.applies == Applies::Shortcuts {
                    out.push_str(&format!(
                        "  {:<14} {} shortcuts  (no config file)\n",
                        group(section),
                        catalogue::SHORTCUTS.len(),
                    ));
                    continue;
                }
                let config = Config::load(section.file);
                // The Display page's row count is one per field per connected
                // output, so it is the compositor's answer, not a constant.
                let described = match section.applies {
                    Applies::Displays => {
                        let count = monitors::list().len();
                        format!(
                            "{} fields \u{d7} {count} monitor{} + primary display",
                            section.settings.len(),
                            if count == 1 { "" } else { "s" },
                        )
                    }
                    _ => format!("{} settings", section.settings.len()),
                };
                out.push_str(&format!(
                    "  {:<14} {}, {} set  {}{}\n",
                    group(section),
                    described,
                    config.entries().len(),
                    config.path.display(),
                    if config.path.is_file() {
                        ""
                    } else {
                        "  (no file yet)"
                    },
                ));
            }
        }
        out.trim_end().to_string()
    }

    /// What every catalogue key of one page (or one file, or one option group) is
    /// set to - how "why does the bar ignore my value" is answered without opening
    /// the window.
    ///
    /// A name several pages share (the Hyprland tab's `desktop` file) prints all
    /// of them, and a Hyprland key prints what the compositor has it at *now* -
    /// which is the only way to see that something else set it to another value.
    fn key_lines(&self, name: &str) -> Result<String, String> {
        let pages: Vec<&'static Section> = pages()
            .into_iter()
            .filter(|section| section.file == name || group(section) == name)
            .collect();
        if pages.is_empty() {
            return Err(format!("no page named `{name}` (try `sections`)"));
        }
        let mut out = String::new();
        for section in pages {
            // The Shortcuts page has no keys to read back - it *is* the list, so
            // this verb prints that list instead (the answer to "which keys does
            // this desktop actually have", without a window).
            if section.applies == Applies::Shortcuts {
                out.push_str(&format!(
                    "{} ({})  no config file\n",
                    section.title,
                    group(section)
                ));
                for shortcut in catalogue::SHORTCUTS {
                    out.push_str(&format!("  {:<32} {}\n", shortcut.keys, shortcut.action));
                }
                continue;
            }
            let config = Config::load(section.file);
            out.push_str(&format!(
                "{} ({})  {}\n",
                section.title,
                group(section),
                config.path.display()
            ));
            // The Display page's keys are per output, and their *values* are what
            // the monitor is doing when the file has no opinion - so they are
            // listed from the same resolution the page draws with.
            if section.applies == Applies::Displays {
                let displays = monitors::list();
                for field in display_fields(&config, &displays) {
                    // A display field is "set" when the *file* has the line - the
                    // value shown is what is in force either way.
                    let set = config.get(&field.key).is_some();
                    out.push_str(&format!(
                        "  {} = {}  ({})\n",
                        field.key,
                        field.value,
                        if set { "set" } else { "default" }
                    ));
                    if let Some(current) = &field.current {
                        out.push_str(&format!("      now {current}\n"));
                    }
                }
                continue;
            }
            for setting in section.settings {
                let set = config.get(setting.key);
                out.push_str(&format!(
                    "  {} = {}  ({})\n",
                    setting.key,
                    set.unwrap_or(setting.default),
                    if set.is_some() { "set" } else { "default" }
                ));
                if section.applies == Applies::Compositor {
                    if let Some(reading) = hyprctl::option(setting.key) {
                        out.push_str(&format!(
                            "      now {}{}\n",
                            reading.value,
                            if reading.set {
                                " - and the compositor's own config sets it"
                            } else {
                                ""
                            }
                        ));
                    }
                }
            }
        }
        Ok(out.trim_end().to_string())
    }

    /// What the dropdowns of one page offer: every name a [`Kind::Choice`] or
    /// [`Kind::Keymap`] row can be set to, with the description the machine gives
    /// it.
    ///
    /// A list of ninety-nine layouts cannot be read off a row that shows one of
    /// them, and a *wrong* one is not something the compositor complains about -
    /// so this is how "is that a name this machine has" is answered without a
    /// window. It is the same resolution the window does, asked of the same
    /// place.
    fn choice_lines(&self, name: &str) -> Result<String, String> {
        let pages: Vec<&'static Section> = pages()
            .into_iter()
            .filter(|section| section.file == name || group(section) == name)
            .collect();
        if pages.is_empty() {
            return Err(format!("no page named `{name}` (try `sections`)"));
        }
        let mut out = String::new();
        for section in pages {
            // A page of keys has no dropdowns at all.
            if section.applies == Applies::Shortcuts {
                continue;
            }
            let config = Config::load(section.file);
            for field in fields_of(&config, section, section.applies == Applies::Compositor)
                .iter()
                .filter(|field| !field.choices.is_empty())
            {
                out.push_str(&format!("\n{} = {}\n", field.key, field.value));
                for entry in &field.choices {
                    // The written value, and the friendlier spelling when the two
                    // differ.
                    let shown = entry.display();
                    out.push_str(&format!(
                        "    {:<20} {}\n",
                        entry.name,
                        if shown == entry.name {
                            ""
                        } else {
                            shown.as_str()
                        }
                    ));
                }
            }
        }
        if out.is_empty() {
            return Ok(format!("`{name}` has no list to pick from"));
        }
        Ok(out.trim_end().to_string())
    }

    /// The tabs and their pages, as the sidebar draws them.
    fn section_lines(&self) -> String {
        let mut out = String::new();
        for tab in catalogue::TABS {
            out.push_str(&format!("[{}] {}  ({})\n", tab.id, tab.title, tab.blurb));
            for (index, section) in tab.sections.iter().enumerate() {
                out.push_str(&format!(
                    "  {:>2}  {:<14} {}\n",
                    index + 1,
                    group(section),
                    section.title
                ));
            }
        }
        out.trim_end().to_string()
    }
}

/// Every page of every tab.
fn pages() -> Vec<&'static Section> {
    catalogue::TABS
        .iter()
        .flat_map(|tab| tab.sections.iter())
        .collect()
}

/// What a page's keys are grouped under: the file itself for an element's page,
/// the first part of the option path for a Hyprland one (`general:gaps_in` →
/// `general`), because that is what the group *is* on the compositor's side, and
/// `display` for the Display page (whose rows are not keys but fields).
fn group(section: &Section) -> &str {
    match section.applies {
        Applies::Element => section.file,
        Applies::Energy => section.file,
        // The lock screen is grouped by its own name too: `lock`, which is both
        // the page's file and what the verbs call it.
        Applies::Lock => section.file,
        // A list of keys has no file of ours; its own name is what the verbs call
        // it (`keys shortcuts`).
        Applies::Shortcuts => section.file,
        Applies::Displays => "display",
        Applies::Compositor => section
            .settings
            .first()
            .and_then(|setting| setting.key.split(':').next())
            .unwrap_or(section.file),
    }
}

/// One output's Display fields, as [`monitor_rules`] builds them: the connector,
/// and one `(field, kind, value)` per field of the rule.
type MonitorFields = (String, Vec<(String, Kind, String)>);

/// The monitor rules the Display page's keys add up to: one entry per output that
/// has at least one `monitor:<output>:<field>` key set in the file.
///
/// A monitor rule is *all-or-nothing* - `hl.monitor` replaces everything known
/// about that output - so a field the file does not set is filled in: from what
/// the monitor is doing right now when it is connected, and from the catalogue's
/// default when it is not. Without that, setting the mode of a monitor would
/// silently reset its scale to Hyprland's default. An output nobody has touched
/// produces nothing at all, which is what keeps this file from overriding a
/// machine's own `hyprland.lua` until the Display page is actually used.
fn monitor_rules(config: &Config) -> Vec<MonitorFields> {
    let live = monitors::list();
    configured_outputs(config)
        .into_iter()
        .map(|output| {
            let monitor = live.iter().find(|display| display.name == output);
            let fields = catalogue::MONITOR_SETTINGS
                .iter()
                .map(|setting| {
                    let key = format!("monitor:{output}:{}", setting.key);
                    let value = config
                        .get(&key)
                        .map(str::to_string)
                        .or_else(|| monitor.and_then(|display| display.value(setting.key)))
                        .unwrap_or_else(|| setting.default.to_string());
                    (setting.key.to_string(), setting.kind, value)
                })
                .collect();
            (output, fields)
        })
        .collect()
}

/// Which outputs the file has an opinion about: every `monitor:<output>:<field>`
/// line whose field is one the Display page knows, in a stable order.
///
/// An output name may contain a `-` but never a `:`, so the field is whatever
/// follows the *last* separator; a line naming a field this window does not know
/// (`monitor:eDP-1:brightness` by hand) does not make the rule - it stays in the
/// file, and the page shows it as text, but it does not claim the output.
fn configured_outputs(config: &Config) -> Vec<String> {
    let mut outputs: Vec<String> = Vec::new();
    for (key, _) in config.entries() {
        let Some(rest) = key.strip_prefix("monitor:") else {
            continue;
        };
        let Some((output, field)) = rest.rsplit_once(':') else {
            continue;
        };
        if catalogue::MONITOR_SETTINGS
            .iter()
            .any(|setting| setting.key == field)
            && !outputs.iter().any(|name| name == output)
        {
            outputs.push(output.to_string());
        }
    }
    outputs.sort();
    outputs
}

/// The System tab's opening rows: what the machine is doing *right now* - the
/// power profile and its driver, the CPU governor and frequency, the battery, and
/// which of the managed effects are currently drawing.
///
/// They are read while the page is drawn, like the Hyprland pages' `now …` lines,
/// and they are [`Kind::Info`] rows: a reading has no key to write and no control
/// to build, which is exactly what "data for the user to optimise with" means.
fn energy_readings() -> Vec<Field> {
    let active = energy::active_profile();
    let live = energy::live_values();
    energy::readings(active.as_deref(), &live)
        .into_iter()
        .map(|reading| Field {
            key: reading.label.to_string(),
            key_label: None,
            heading: None,
            label: reading.label.to_string(),
            help: reading.help.to_string(),
            default: None,
            kind: Kind::Info,
            value: reading.value,
            is_default: false,
            current: None,
            choices: Vec::new(),
            keycap: None,
        })
        .collect()
}

/// The System tab's closing row: the two buttons that ask the energy daemon to
/// act now. One row rather than two, because they are the same kind of thing -
/// "do it immediately" versus "undo it immediately".
fn energy_actions() -> Field {
    Field {
        key: "actions".to_string(),
        key_label: None,
        heading: None,
        label: "Act now".to_string(),
        help: "Save now drops every effect above in the desktop and keeps them off \
               until Restore; Restore puts back the values the machine had before the \
               first time energy was saved. The daemon does both by itself on every \
               profile change."
            .to_string(),
        default: None,
        kind: Kind::Actions(catalogue::ENERGY_ACTIONS),
        value: String::new(),
        is_default: false,
        current: None,
        choices: Vec::new(),
        keycap: None,
    }
}

/// The Display page's first row: which output the collection's notification cards
/// appear on.
///
/// It shares the page and its file with the monitor rules, but it is not one of
/// them: the key is a plain `primary`, the value a connector name or `focused`,
/// and it is read back by the elements themselves (`monitors::primary_connector`)
/// rather than handed to the compositor - Hyprland has no idea what a primary
/// output is. That is why the row carries no heading of its own (it belongs to no
/// output) and why [`monitor_rules`] never sees it (the key is not a field of
/// [`catalogue::MONITOR_SETTINGS`]).
fn primary_field(config: &Config, displays: &[monitors::Display]) -> Field {
    let setting = &catalogue::PRIMARY_DISPLAY_SETTING;
    let set = config.get(setting.key);
    let value = set.unwrap_or(setting.default).to_string();
    // "Follow focus" comes first because it is the state the file is in until
    // somebody picks an output - and the one that keeps a card where the user is
    // looking rather than in a place they chose and forgot about.
    let mut choices = vec![keymap::Entry::labelled(
        catalogue::PRIMARY_FOCUSED,
        "Follow focus",
        "wherever you are when a card appears",
    )];
    choices.extend(displays.iter().map(|display| {
        keymap::Entry::labelled(
            display.name.clone(),
            display.name.clone(),
            display.summary(),
        )
    }));
    Field {
        key: setting.key.to_string(),
        key_label: None,
        heading: None,
        label: setting.label.to_string(),
        help: setting.help.to_string(),
        // The default is a word this collection chose, and a written-out value is
        // the only thing the ↺ is for - so the reset button appears exactly when
        // the file has a line (`is_default` false).
        default: Some(setting.default.to_string()),
        kind: setting.kind,
        value,
        is_default: set.is_none(),
        current: None,
        choices,
        keycap: None,
    }
}

/// The Display page's rows: one per field, per connected output.
///
/// The key is `monitor:<output>:<field>` - exactly what the file holds and what
/// [`monitor_rules`] reads - and the value is what the file says, else what the
/// monitor is doing now, because that is what is in force. `current` is set only
/// once the file has a line of its own (which is also when the ↺ appears), so a
/// row the file does not mention simply states the value it is using.
///
/// The compositor's answer is handed in (`displays`) rather than asked here: a
/// page render has already asked it once for the editor, and asking again would
/// be a second `hyprctl -j monitors` for the same picture.
///
/// The rows of one output share a [`Heading`], which the view draws once: the
/// output name and its live summary over the fields it owns.
///
/// The page ignores every other key of `desktop.conf` (the option pages share the
/// file), but it does not swallow a `monitor:` key it does not understand: a
/// hand-written one that names an absent output or an unknown field is shown as a
/// text row rather than becoming invisible.
fn display_fields(config: &Config, displays: &[monitors::Display]) -> Vec<Field> {
    // The page opens with the collection's own choice - which output the cards
    // appear on - and only then the monitor rules, one group per output.
    let mut fields = vec![primary_field(config, displays)];
    for display in displays {
        // One heading per output, drawn once over that output's rows: the name the
        // keys are qualified by, and what the monitor is doing right now.
        let heading = Heading {
            icon: names::DISPLAY,
            title: display.name.clone(),
            detail: display.summary(),
            focused: display.focused,
        };
        for setting in catalogue::MONITOR_SETTINGS {
            let key = format!("monitor:{}:{}", display.name, setting.key);
            let set = config.get(&key);
            let live = display.value(setting.key).unwrap_or_default();
            let value = set.map(str::to_string).unwrap_or_else(|| live.clone());
            // "now …" is only worth saying when the file and the monitor disagree:
            // a row whose value already *is* the live one would just repeat itself.
            let current = (set.is_some() && live != value).then(|| live.clone());
            let mut choices = monitor_choices(setting.kind, display, displays);
            // A value the list does not know (a mode the monitor stopped
            // reporting, a scale nobody offers) is still what the file says, so
            // it gets an entry of its own rather than the dropdown showing
            // somebody else's value.
            if !choices.is_empty()
                && !value.is_empty()
                && !choices.iter().any(|entry| entry.name == value)
            {
                choices.insert(0, keymap::Entry::plain(value.clone()));
            }
            fields.push(Field {
                key,
                // The heading names the output, so the state line need not.
                key_label: Some(setting.key.to_string()),
                heading: Some(heading.clone()),
                label: setting.label.to_string(),
                help: setting.help.to_string(),
                // A display row is never "the built-in default": what the control
                // shows is what is in force - the file's value, or the monitor's
                // when the file has none - so the state line says that value. The
                // ↺ appears only where a line is actually set, which is also the
                // signal that this row is written down.
                default: set.map(|_| setting.default.to_string()),
                kind: setting.kind,
                value,
                is_default: false,
                current,
                choices,
                keycap: None,
            });
        }
    }

    // Keys for an output that is not connected (a monitor you unplugged, but want
    // configured for next time) or a field this window does not know.
    let known: Vec<String> = fields.iter().map(|field| field.key.clone()).collect();
    for (key, value) in config.entries() {
        if !key.starts_with("monitor:") || known.contains(&key) {
            continue;
        }
        // `monitor:<output>:<field>` - the label keeps the output, which is the
        // part that says which monitor the line is about.
        let label = match key
            .strip_prefix("monitor:")
            .and_then(|rest| rest.rsplit_once(':'))
        {
            Some((output, field)) => format!("{} {}", output, humanize(field)),
            None => humanize(&key),
        };
        fields.push(Field {
            key: key.clone(),
            key_label: None,
            heading: None,
            label,
            help: "A display key this window does not know, or one for an output that is not \
                   plugged in. It is written to the file, but only the fields the Display page \
                   lists reach the compositor."
                .to_string(),
            default: None,
            kind: Kind::Text,
            value,
            is_default: false,
            current: None,
            choices: Vec::new(),
            keycap: None,
        });
    }
    fields
}

/// The Shortcuts page's rows: every key the collection implements, in the two
/// groups [`catalogue::SHORTCUTS`] sorts them into - the ones Hyprland binds, and
/// the ones a card reads itself while it is up.
///
/// A shortcut is not a setting, so its row is not a [`catalogue::Setting`]: the
/// keys go where a setting's name goes (in a keycap, see `Field::keycap`), the
/// action below them, and the sentence about it as the quiet line. Nothing here is
/// read from a file or from the session - see [`catalogue::Applies::Shortcuts`]
/// for why the running compositor cannot answer this and the template can.
fn shortcut_fields() -> Vec<Field> {
    use catalogue::ShortcutScope;
    catalogue::SHORTCUTS
        .iter()
        .map(|shortcut| {
            let (icon, title, detail) = match shortcut.scope {
                ShortcutScope::Hyprland => (
                    names::KEYBOARD,
                    "Bound in Hyprland",
                    "~/.config/hypr/osd.lua - reachable from anywhere",
                ),
                ShortcutScope::Card => (
                    names::GRID,
                    "Inside the cards",
                    "read by the card itself, while it is up",
                ),
            };
            Field {
                // A row here has no config key; the keys are the only name it
                // has, so they are also what a tooltip and the verbs call it.
                key: shortcut.keys.to_string(),
                key_label: None,
                heading: Some(Heading {
                    icon,
                    title: title.to_string(),
                    detail: detail.to_string(),
                    focused: false,
                }),
                label: shortcut.action.to_string(),
                help: shortcut.help.to_string(),
                default: None,
                kind: Kind::Info,
                value: String::new(),
                is_default: false,
                current: None,
                choices: Vec::new(),
                keycap: Some(shortcut.keys.to_string()),
            }
        })
        .collect()
}

/// What a display row's list offers: the monitor's own modes, the scales worth
/// offering, or the other outputs. Anything else has no list, which is what makes
/// the control a field or a spinner.
///
/// The entries carry a friendlier `label` than the value they *write*, because the
/// raw spelling is what Hyprland wants, not what a person reads: `2560x1440@59.95`
/// shows as `2560 × 1440 · 59.95 Hz`, a mirror candidate carries the picture it
/// would copy, and a scale its percentage.
fn monitor_choices(
    kind: Kind,
    display: &monitors::Display,
    all: &[monitors::Display],
) -> Vec<keymap::Entry> {
    match kind {
        Kind::Monitor(MonitorList::Mode) => {
            let mut modes = vec![keymap::Entry::labelled(
                "preferred",
                "Preferred",
                "the monitor's own best mode",
            )];
            modes.extend(display.modes.iter().map(|mode| match mode_parts(mode) {
                Some((width, height, hz)) => keymap::Entry::labelled(
                    mode.clone(),
                    format!("{width} \u{d7} {height}"),
                    format!("{hz:.2} Hz"),
                ),
                None => keymap::Entry::plain(mode.clone()),
            }));
            modes
        }
        Kind::Monitor(MonitorList::Scale) => ["auto", "1", "1.25", "1.5", "1.75", "2", "2.5", "3"]
            .iter()
            .map(|scale| match scale.parse::<f64>() {
                Ok(value) => keymap::Entry::labelled(
                    *scale,
                    *scale,
                    format!("{} %", (value * 100.0).round() as i32),
                ),
                Err(_) => keymap::Entry::labelled("auto", "Auto", "the monitor's own choice"),
            })
            .collect(),
        Kind::Monitor(MonitorList::Mirror) => {
            let mut mirrors = vec![keymap::Entry::labelled("none", "None", "")];
            mirrors.extend(
                all.iter()
                    .filter(|other| other.name != display.name)
                    .map(|other| {
                        keymap::Entry::labelled(
                            other.name.clone(),
                            other.name.clone(),
                            other.summary(),
                        )
                    }),
            );
            mirrors
        }
        _ => Vec::new(),
    }
}

/// Split a mode (`2560x1440@59.95`) into what a friendlier label is made of:
/// width, height and refresh rate. `None` for a spelling this Hyprland does not
/// use - which is then shown as the raw string rather than hidden.
fn mode_parts(mode: &str) -> Option<(i32, i32, f64)> {
    let (size, hz) = mode.split_once('@')?;
    let (width, height) = size.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?, hz.parse().ok()?))
}

/// Resolve one page: the catalogue's keys, as the file has them, plus any key the
/// file sets that the catalogue does not know about.
///
/// The extra keys matter: this window is the only way in for most people, and a
/// key that exists in the file but not in the catalogue must not become
/// invisible - it is shown as text, and the help line says why it is not a
/// control.
///
/// With `live` set (the Hyprland pages), each key also carries what the
/// compositor has it at right now - one `hyprctl` per row, on the way into a page.
fn fields_of(config: &Config, section: &Section, live: bool) -> Vec<Field> {
    // The Display page is not keyed by a config key: its rows are made once per
    // connected output, and they are the monitor's fields rather than the file's
    // catalogue. It shares the file with the option pages, so it owns the
    // `monitor:` keys entirely (see `display_fields`). The page render reaches it
    // directly with the displays it already fetched; this branch is the verbs'
    // way in, and asks the compositor itself.
    if section.applies == Applies::Displays {
        return display_fields(config, &monitors::list());
    }
    // A keyboard row picks from the names the *machine* has, and one of those
    // lists - the variants - is the list of one layout: the layout in force, which
    // is the compositor's answer before the file's and Hyprland's own default
    // last. Only a page that has such a row pays for the two questions.
    let (rules, layout) = if section
        .settings
        .iter()
        .any(|setting| matches!(setting.kind, Kind::Keymap(_)))
    {
        let asked = |key: &str, default: &str| -> String {
            if live {
                if let Some(reading) = hyprctl::option(key) {
                    let value = reading.value.trim().to_string();
                    if !value.is_empty() {
                        return value;
                    }
                }
            }
            config.get(key).unwrap_or(default).trim().to_string()
        };
        // `de,us` is a list of layouts; the variants on offer are the first one's,
        // because that is the layout the keyboard starts on.
        let first = asked("input:kb_layout", "us");
        let first = first.split(',').next().unwrap_or("us").trim().to_string();
        (asked("input:kb_rules", ""), first)
    } else {
        (String::new(), String::new())
    };

    let mut fields: Vec<Field> = section
        .settings
        .iter()
        .map(|setting| {
            let set = config.get(setting.key);
            let value = set.unwrap_or(setting.default).to_string();
            let mut choices = choices_of(setting.kind, &rules, &layout);
            // What the file says has to be selectable, or the row would show a
            // name that is not on offer - and a comma list (`de,us`) or a name
            // this machine's data does not have are both things a file may say.
            // The note says which of those it is: a variant that belongs to
            // *another* layout (`kb_layout = de` with the `ch` of `ch`) is not a
            // value the compositor rejects, it is a keyboard that quietly does not
            // work - the one failure in this tab that has no error message of its
            // own, and this is where it becomes visible.
            if !choices.is_empty()
                && !value.is_empty()
                && !choices.iter().any(|entry| entry.name == value)
            {
                let note = match setting.kind {
                    // The layout is named because that is the actionable part:
                    // `ch` *is* a layout, it is just not a variant of this one.
                    Kind::Keymap(keymap::Keymap::Variant) => format!("not a variant of {layout}"),
                    Kind::Keymap(_) => "unknown name".to_string(),
                    _ => "unknown value".to_string(),
                };
                choices.insert(0, keymap::Entry::described(value.clone(), note));
            }
            Field {
                key: setting.key.to_string(),
                key_label: None,
                heading: None,
                label: setting.label.to_string(),
                help: setting.help.to_string(),
                default: Some(setting.default.to_string()),
                kind: setting.kind,
                value,
                is_default: set.is_none(),
                current: if live {
                    hyprctl::option(setting.key).map(|reading| reading.value)
                } else {
                    None
                },
                choices,
                keycap: None,
            }
        })
        .collect();

    for (key, value) in config.entries() {
        if section.settings.iter().any(|setting| setting.key == key) {
            continue;
        }
        fields.push(Field {
            key: key.clone(),
            key_label: None,
            heading: None,
            label: humanize(&key),
            // A hand-added key means two different things here. An element reads
            // the file itself, so the key is in force the moment it restarts. A
            // Hyprland page does not: the file is only where the window keeps its
            // values, and what reaches the compositor is the document it generates
            // from the options this window manages - so a key outside them does
            // nothing, and the row has to say so instead of looking like a setting
            // that is quietly ignored.
            help: if matches!(section.applies, Applies::Element | Applies::Energy) {
                "Not a setting this window knows; it is here because the file sets it."
            } else {
                "Not an option this window manages; it is written to the file, but the \
                 compositor only ever sees the options the Hyprland tab lists."
            }
            .to_string(),
            default: None,
            kind: Kind::Text,
            value,
            is_default: false,
            current: None,
            // A key the window does not know is text, never a dropdown: there is
            // no list it could be one of.
            choices: Vec::new(),
            keycap: None,
        });
    }
    fields
}

/// The list a dropdown is built from: the catalogue's own for a
/// [`Kind::Choice`], and the machine's XKB names for a [`Kind::Keymap`] - which
/// answers with nothing at all on a machine that has no keymap data, and that
/// empty list is what makes the row a text field again.
fn choices_of(kind: Kind, rules: &str, layout: &str) -> Vec<keymap::Entry> {
    match kind {
        Kind::Choice(options) => options
            .iter()
            .map(|name| keymap::Entry::plain(*name))
            .collect(),
        Kind::Keymap(list) => keymap::entries(list, rules, layout),
        _ => Vec::new(),
    }
}

/// `notification_rows` → `Notification rows`, `tick_ms` → `Tick (ms)`: the
/// label for a key the catalogue does not know, so a hand-added key still reads
/// like a setting rather than like a variable name.
fn humanize(key: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut suffix = "";
    for word in key.split('_') {
        if word == "ms" {
            suffix = " (ms)";
            continue;
        }
        parts.push(word.to_string());
    }
    let mut label = parts.join(" ");
    if let Some(first) = label.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    label.push_str(suffix);
    label
}

/// The `pkill -f` pattern for an element: `hypr-osd-launcher$`.
///
/// Anchored at the end of the command line, because the shell that runs a restart
/// has the binary's path in its own command line - an unanchored pattern would
/// match that shell and the script would kill itself (see [`Panel::restart`]).
/// An element's own command line is exactly its path: they are started without
/// arguments.
fn kill_pattern(element: &str) -> String {
    format!("hypr-osd-{element}$")
}

/// The tab a name on the command line means: its id, or its title.
fn tab_index(name: &str) -> Option<usize> {
    let wanted = name.trim().to_lowercase();
    catalogue::TABS
        .iter()
        .position(|tab| tab.id == wanted || tab.title.to_lowercase() == wanted)
}

/// The page a name on the command line means, as `(tab, page)`: an element's or
/// option group's name, a page title (case and spaces are forgiven), or a 1-based
/// number in the tab that is showing.
fn page_index(name: &str, tab: usize) -> Option<(usize, usize)> {
    let wanted = name.trim().to_lowercase();
    if let Ok(number) = wanted.parse::<usize>() {
        // `then_some` would evaluate `number - 1` eagerly, and page "0" is a
        // number a person can type.
        let count = catalogue::TABS.get(tab)?.sections.len();
        return if (1..=count).contains(&number) {
            Some((tab, number - 1))
        } else {
            None
        };
    }
    for (index, entry) in catalogue::TABS.iter().enumerate() {
        let found = entry.sections.iter().position(|section| {
            section.file == wanted
                || group(section) == wanted
                || section.title.to_lowercase() == wanted
        });
        if let Some(page) = found {
            return Some((index, page));
        }
    }
    None
}

/// Wire the keys the window lives on.
///
/// Installed through the shell rather than on the widgets, because the shell
/// rebuilds the surface when the card moves to another monitor - a keyboard card
/// that lost its keys after a monitor change would be a bug nobody could explain.
///
/// The handler takes Escape and nothing else: every other key belongs to whichever
/// control has the focus (the fields are typed into, the spinners take the
/// arrows), and swallowing them here would make the window unusable.
fn install_keys(osd: &Rc<Osd>, panel: Rc<Panel>) {
    osd.on_keys(
        {
            let panel = panel.clone();
            move |key, _state| {
                if !panel.osd.is_visible() {
                    return glib::Propagation::Proceed;
                }
                // A key is an interaction, and an interaction is not being idle.
                panel.arm_idle();
                if *key == gdk::Key::Escape {
                    panel.close();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        },
        // No release handling: this window is a place you leave, not a key you
        // hold (that is the switcher's shape).
        |_| {},
    );
}

fn main() -> glib::ExitCode {
    let config = Config::load(ELEMENT);
    let settings = Rc::new(Settings::load(&config));

    let opts = Opts {
        app_id: APP_ID.to_string(),
        css: css::stylesheet(include_str!("settings.css")),
        namespace: NAMESPACE.to_string(),
        width: settings.width,
        // A window, in the middle of the screen: it is a place you go to, not a
        // card that hangs off the bar.
        placement: Placement::Center,
        // It has to be typed into - see the module comment.
        keyboard: Keyboard::Exclusive,
        ..Opts::default()
    };

    let state_cell: Rc<OnceCell<Rc<Panel>>> = Rc::new(OnceCell::new());

    let build = {
        let state_cell = state_cell.clone();
        let settings = settings.clone();
        Box::new(move |osd: &Rc<Osd>| {
            // `SettingsView::new` already returns the `Rc` the callbacks need, so
            // it is not wrapped in a second one.
            let view = SettingsView::new(
                settings.sidebar_width,
                settings.pane_width(),
                settings.pane_height,
            );
            let panel = Panel::new(osd, view.clone(), settings.clone());

            // The lock screen's variables are cut from the theme, so opening this
            // window rewrites them: a `theme.css` edited by hand would otherwise
            // reach the lock screen only when someone pressed that page's
            // Re-apply. It goes through the same path a change does, so a failure
            // is reported in `status` rather than only on the terminal, and is not
            // fatal - the file that is already there still works.
            panel.regenerate();

            // A `kill`ed window leaves "open" behind, which would keep the gear
            // lit with nothing behind it - so the flag starts closed, exactly as
            // the island's and the panel's do.
            state::write(&state::settings_panel(), false);
            osd.on_shutdown(|| state::write(&state::settings_panel(), false));

            view.on_tab({
                let panel = panel.clone();
                move |index| panel.open_tab(index)
            });
            view.on_section({
                let panel = panel.clone();
                move |index| panel.show_section(index)
            });
            view.on_change({
                let panel = panel.clone();
                move |file, key, value| panel.changed(file, key, value)
            });
            view.on_action({
                let panel = panel.clone();
                move || panel.act()
            });
            view.on_command({
                let panel = panel.clone();
                move |id| panel.command(id)
            });
            view.on_close({
                let panel = panel.clone();
                move || panel.close()
            });
            // Moving the pointer over the window is looking at it, and looking
            // at it is not being idle.
            view.on_motion({
                let panel = panel.clone();
                move || panel.arm_idle()
            });

            install_keys(osd, panel.clone());

            // The first paint, before the surface is ever shown: the card has to
            // be *sized* before it is placed.
            panel.show_section(0);

            let root = view.root.clone().upcast::<gtk::Widget>();
            state_cell.set(panel).ok();
            Content::Single(root)
        })
    };

    let handle = {
        let state_cell = state_cell.clone();
        Rc::new(
            move |_osd: &Rc<Osd>, args: &[String]| -> Result<String, String> {
                let panel = state_cell
                    .get()
                    .ok_or("the settings window has not finished starting up")?;
                let Some(verb) = args.first().map(String::as_str) else {
                    // No verb: started by the installer, so wait for the gear.
                    return Ok(String::new());
                };
                let named = |args: &[String]| -> Result<(usize, usize), String> {
                    let name = args[1..].join(" ");
                    page_index(&name, panel.tab.get())
                        .ok_or_else(|| format!("no page named `{name}` (try `sections`)"))
                };
                match verb {
                    "toggle" => panel.toggle(),
                    "show" => {
                        if args.len() > 1 {
                            let (tab, page) = named(args)?;
                            panel.open(Some(tab), Some(page));
                        } else {
                            panel.open(None, None);
                        }
                    }
                    "tab" => {
                        let name = args[1..].join(" ");
                        let index = tab_index(&name)
                            .ok_or_else(|| format!("no tab named `{name}` (try `sections`)"))?;
                        panel.open(Some(index), Some(0));
                    }
                    "section" => {
                        let (tab, page) = named(args)?;
                        panel.open(Some(tab), Some(page));
                    }
                    "apply" => panel.apply()?,
                    "hide" | "close" => panel.close(),
                    "set" => {
                        let name = args.get(1).ok_or("`set` needs a page (try `sections`)")?;
                        let key = args.get(2).ok_or("`set` needs a key")?;
                        let value = args.get(3).cloned().unwrap_or_default();
                        panel.set_value(name, key, Some(&value))?;
                    }
                    "unset" => {
                        let name = args.get(1).ok_or("`unset` needs a page (try `sections`)")?;
                        let key = args.get(2).ok_or("`unset` needs a key")?;
                        panel.set_value(name, key, None)?;
                    }
                    "restart" => {
                        let element = args.get(1).ok_or("`restart` needs an element")?;
                        panel.restart(element);
                    }
                    // The Display page's editor, answered without a pointer:
                    // `drop eDP-1 0 300` is "what would dragging that box 300
                    // canvas pixels downwards write". The canvas's width is the
                    // one the page draws at, so the arithmetic is the same.
                    "drop" => {
                        let output = args.get(1).ok_or("`drop` needs an output")?;
                        let dx: f64 = args
                            .get(2)
                            .and_then(|value| value.parse().ok())
                            .ok_or("`drop` needs a horizontal gesture (canvas pixels)")?;
                        let dy: f64 = args
                            .get(3)
                            .and_then(|value| value.parse().ok())
                            .ok_or("`drop` needs a vertical gesture (canvas pixels)")?;
                        let width = (panel.settings.pane_width() - 24).max(200);
                        return arrange::would_drop(&monitors::list(), width, output, dx, dy)
                            .ok_or_else(|| format!("no output named `{output}`"));
                    }
                    "command" => {
                        let id = args
                            .get(1)
                            .ok_or("`command` needs an id (apply | restore)")?;
                        panel.command(id);
                    }
                    "sections" => return Ok(panel.section_lines()),
                    "keys" => {
                        let name = args.get(1).ok_or("`keys` needs a page")?;
                        return panel.key_lines(name);
                    }
                    "choices" => {
                        let name = args.get(1).ok_or("`choices` needs a page")?;
                        return panel.choice_lines(name);
                    }
                    "status" => return Ok(panel.status_lines()),
                    other => {
                        return Err(format!(
                            "unknown verb `{other}` (toggle | show [page] | hide | tab <name> | \
                             section <name> | apply | set <page> <key> <value> | unset <page> \
                             <key> | restart <element> | command <id> | sections | keys <page> | \
                             choices <page> | status)"
                        ))
                    }
                }
                Ok(String::new())
            },
        )
    };

    run(opts, build, handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hand_added_key_gets_a_readable_label() {
        assert_eq!(humanize("notification_rows"), "Notification rows");
        assert_eq!(humanize("tick_ms"), "Tick (ms)");
        assert_eq!(humanize("output"), "Output");
        assert_eq!(humanize("updates_every_ms"), "Updates every (ms)");
    }

    #[test]
    fn the_kill_pattern_cannot_match_its_own_shell() {
        let pattern = kill_pattern("bar");
        assert_eq!(pattern, "hypr-osd-bar$");
        // `pkill -f` matches an anchored regular expression against the whole
        // command line, so "the pattern matches" means "the line *ends* with the
        // name" - which is what the real process's line does...
        let name = "/home/lars/.local/bin/hypr-osd-bar";
        assert!(name.ends_with(&pattern[..pattern.len() - 1]));
        // ...and what the shell's own line, which spells the path out and then
        // goes on to start it, does not.
        let shell = format!("sh -c pkill -f '{pattern}' >/dev/null 2>&1; '{name}' &");
        assert!(!shell.ends_with(&pattern[..pattern.len() - 1]));
    }

    #[test]
    fn pages_are_named_by_group_title_or_number() {
        // An element's page, by its file or its title, in either case...
        assert_eq!(page_index("bar", 0), Some((0, 0)));
        assert_eq!(page_index("Bar", 0), Some((0, 0)));
        assert_eq!(page_index("system popup", 0), Some((0, 2)));
        // ...a number inside the tab that is showing...
        assert_eq!(page_index("1", 0), Some((0, 0)));
        assert_eq!(page_index("11", 0), Some((0, 10)));
        assert_eq!(page_index("12", 0), Some((0, 11)));
        assert_eq!(page_index("13", 0), Some((0, 12)));
        assert_eq!(page_index("14", 0), None);
        assert_eq!(page_index("0", 0), None);
        // The lock screen is a page with a config file but no binary of its own.
        assert_eq!(page_index("lock", 0), Some((0, 6)));
        assert_eq!(page_index("Lock screen", 0), Some((0, 6)));
        // The list of keys is a page like any other, though it has no config file
        // of its own.
        assert_eq!(page_index("shortcuts", 0), Some((0, 12)));
        assert_eq!(page_index("Shortcuts", 0), Some((0, 12)));
        // ...and a Hyprland page by its option group, its title, or the file all
        // of them share. The Display page comes first, and "desktop" names it
        // because it is the first page that shares that file.
        assert_eq!(page_index("display", 0), Some((1, 0)));
        assert_eq!(page_index("Display", 0), Some((1, 0)));
        assert_eq!(page_index("desktop", 0), Some((1, 0)));
        assert_eq!(page_index("general", 0), Some((1, 1)));
        assert_eq!(page_index("Decoration", 0), Some((1, 2)));
        // The numbers come from the tab being shown, so "1" means a different
        // page on each of them.
        assert_eq!(page_index("1", 1), Some((1, 0)));
        assert_eq!(page_index("nonsense", 0), None);
    }

    #[test]
    fn the_tabs_are_named_by_id_or_title() {
        assert_eq!(tab_index("osd"), Some(0));
        assert_eq!(tab_index("Hypr OSD"), Some(0));
        assert_eq!(tab_index("hyprland"), Some(1));
        assert_eq!(tab_index("Hyprland"), Some(1));
        assert_eq!(tab_index("nonsense"), None);
    }

    #[test]
    fn a_hyprland_page_is_grouped_by_its_option_path() {
        // `general:gaps_in` belongs to the group the *option* is in, which is
        // what the sidebar and the verbs call that page - and the Display page is
        // grouped by its own name, because its rows are fields rather than keys.
        let hyprland = catalogue::TABS[1].sections;
        assert_eq!(group(&hyprland[0]), "display");
        assert_eq!(group(&hyprland[1]), "general");
        assert_eq!(group(&hyprland[2]), "decoration");
        // An element's page is grouped by its file, which is its name.
        assert_eq!(group(&catalogue::TABS[0].sections[0]), "bar");
    }

    /// Only a `monitor:<output>:<field>` line whose field the Display page knows
    /// claims an output - a hand-written field or a line with no field at all
    /// stays in the file without taking the monitor over. The order is stable, so
    /// the generated document is too.
    #[test]
    fn only_known_fields_claim_an_output() {
        let path = std::env::temp_dir().join("hypr-osd-settings-configured-outputs.conf");
        std::fs::write(
            &path,
            "monitor:eDP-1:scale = 2\n\
             monitor:eDP-1:brightness = 50\n\
             monitor:DP-1:position = 1368x0\n\
             monitor:bogus\n\
             general:gaps_in = 5\n",
        )
        .unwrap();
        let config = Config::load_from(path);
        assert_eq!(configured_outputs(&config), vec!["DP-1", "eDP-1"]);
    }

    /// A display dropdown offers the monitor's own names: `preferred` first among
    /// the modes, the other outputs (never this one) for a mirror, and nothing at
    /// all for a field that is typed (`position`).
    #[test]
    fn a_display_list_offers_the_monitors_own_names() {
        let display = |name: &str| monitors::Display {
            name: name.to_string(),
            width: 2560,
            height: 1440,
            refresh: 59.95,
            x: 0,
            y: 0,
            scale: 1.0,
            transform: 0,
            mirror: "none".to_string(),
            disabled: false,
            focused: false,
            modes: vec!["2560x1440@59.95".to_string(), "1920x1080@60.00".to_string()],
        };
        let edp = display("eDP-1");
        let dp = display("DP-1");
        let both = vec![edp.clone(), dp.clone()];

        let modes = monitor_choices(Kind::Monitor(MonitorList::Mode), &edp, &both);
        // The value written is the raw mode, but the dropdown says it the way a
        // person reads it.
        assert_eq!(modes[0].name, "preferred");
        assert_eq!(
            modes[0].display(),
            "Preferred \u{b7} the monitor's own best mode"
        );
        let mode = modes
            .iter()
            .find(|entry| entry.name == "1920x1080@60.00")
            .expect("the monitor's own mode is offered");
        assert_eq!(mode.display(), "1920 \u{d7} 1080 \u{b7} 60.00 Hz");

        let mirrors = monitor_choices(Kind::Monitor(MonitorList::Mirror), &edp, &both);
        let names: Vec<&str> = mirrors.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, vec!["none", "DP-1"]);
        // A mirror candidate carries what it would copy.
        assert!(mirrors[1]
            .display()
            .starts_with("DP-1 \u{b7} 2560 \u{d7} 1440"));

        let scales = monitor_choices(Kind::Monitor(MonitorList::Scale), &edp, &both);
        let hundred = scales
            .iter()
            .find(|entry| entry.name == "1")
            .expect("a 1:1 scale is offered");
        assert_eq!(hundred.display(), "1 \u{b7} 100 %");

        // Position is typed, so it has no list.
        assert!(monitor_choices(Kind::Text, &edp, &both).is_empty());
    }

    /// A mode the display page can make friendlier is split into its parts; a
    /// spelling it does not recognise is left as the raw string.
    #[test]
    fn a_mode_splits_into_a_width_a_height_and_a_rate() {
        assert_eq!(mode_parts("2560x1440@59.95"), Some((2560, 1440, 59.95)));
        assert_eq!(mode_parts("3840x2160@60.00"), Some((3840, 2160, 60.0)));
        assert_eq!(mode_parts("preferred"), None);
        assert_eq!(mode_parts("1920x1080"), None);
        assert_eq!(mode_parts("axb@c"), None);
    }
}

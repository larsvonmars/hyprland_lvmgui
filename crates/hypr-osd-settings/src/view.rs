//! The settings window's widgets: a sidebar of elements, and a page of settings.
//!
//! A *view*: it draws the sections it is handed, builds one control per field and
//! reports what was changed. What a change *means* - which file it goes in, and
//! which element has to be restarted for it to matter - is the element's business
//! (`main.rs`). Nothing here knows what a config file is.
//!
//! Two things about the pane are deliberate:
//!
//! * **It is one size, always.** The rows live in a fixed-height scroller, so
//!   picking another element cannot change the card's height - a mapped layer
//!   surface grows with its content but does not shrink with it (measured, the
//!   launcher and the applications panel both hit that wall), and a window that
//!   grew on every section would end up as tall as its longest page.
//! * **Nothing here can widen the card.** A `GtkLabel` reports its *full* text as
//!   its natural width even when it is ellipsised, so the help lines - the
//!   longest text in the window - are capped at the width the pane actually has
//!   (`text::cap_width`, the same helper the tiles use). Without that, one long
//!   help string would decide how wide the whole window comes out.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::icons::{self, names};
use hypr_osd_core::keymap::Entry;
use hypr_osd_core::settings::{Applies, Command, Kind, Section};
use hypr_osd_core::text;

/// How large the sidebar's marks are drawn, and the mark inside a round button.
const SECTION_ICON: i32 = 14;
const BUTTON_ICON: i32 = 12;
/// The mark on a group heading (the Display page's monitor).
const HEADING_ICON: i32 = 14;

/// What separates the two panes of the body. Public because the element works the
/// pane's width out from it, and the two must not be able to disagree.
pub const BODY_GAP: i32 = 10;

/// The keys and the one rule of the window, said once - and said differently on
/// the tabs, because the kinds of page apply in different ways.
pub const KEYS_HINT: &str =
    "esc closes \u{b7} a change is written to the element's config at once \u{b7} \
     restart an element to make it read it again";
pub const HYPRLAND_HINT: &str =
    "esc closes \u{b7} a change is handed to the compositor at once \u{b7} \
     the value it reports is on the grey line";
pub const ENERGY_HINT: &str =
    "esc closes \u{b7} a change is written to the policy file at once \u{b7} \
     restart the energy daemon to make it read it again";
/// The lock screen is the one page whose change is complete the moment the file
/// is written: hyprlock reads its config at every lock, so nothing has to be
/// restarted and nothing handed to the compositor.
pub const LOCK_HINT: &str =
    "esc closes \u{b7} a change rewrites the lock screen's variables at once \u{b7} \
     hyprlock reads them at the next lock";
pub const SHORTCUTS_HINT: &str = "esc closes \u{b7} a list to read, not a page to change \u{b7} \
     the bound keys live in ~/.config/hypr/osd.lua";

/// The room a control keeps for itself, the room a reset button takes, and the
/// room a row spends on its own padding, border and gap. The text column gets
/// what is left of the pane - and it *has* to be capped, because a label reports
/// its full text as its natural width, which would otherwise be what decides how
/// wide the window comes out.
const CONTROL_WIDTH: i32 = 150;
const RESET_WIDTH: i32 = 26;
/// The column a shortcut's keys take. Fixed rather than natural, so the actions
/// beside thirty different keycaps still line up - the same reason a page of
/// settings has one control column.
const KEYCAP_WIDTH: i32 = 148;
/// The column a dropdown takes, and the room a label inside it gets. A dropdown is
/// wider than a field because its labels carry a description (`ch · German
/// (Switzerland)`) - and it is a *fixed* width, so a page of them lines up
/// instead of growing and shrinking with the longest name on it.
const CHOICE_WIDTH: i32 = 250;
const CHOICE_TEXT: i32 = CHOICE_WIDTH - 40;
/// A list longer than this gets GTK's search field, which is how one finds a
/// layout among ninety-nine of them.
const SEARCH_FROM: usize = 12;
// `cap_width` estimates a character's advance from a sample, so the budget keeps
// a margin on top of what the row's own furniture needs (measured after the
// fact: a row spends about 230px on itself, and the pane insets it by 6 more).
const ROW_OVERHEAD: i32 = CONTROL_WIDTH + RESET_WIDTH + 74;
/// The room a sidebar row spends on its icon, its gap, its padding and the box's
/// own, so a long element title cannot widen the list either.
const SECTION_OVERHEAD: i32 = 58;

/// One entry in the sidebar: an element, and whether it is the one being shown.
pub struct SectionRow {
    pub label: String,
    pub icon: &'static str,
    pub selected: bool,
}

/// One entry in the tab row: a whole area of the window.
pub struct TabRow {
    pub label: String,
    pub tooltip: String,
    pub selected: bool,
}

/// What the page's chip does, when it has one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PageAction {
    /// An element's page: stop that element and start it again, so it reads the
    /// config just written.
    Restart,
    /// A Hyprland page: write the generated config and hand it to the compositor
    /// again. (A change is applied at once anyway - this is for after a
    /// hand-edited file.)
    Apply,
    /// The lock screen's page: rewrite the variables `hyprlock.conf` is written in
    /// terms of. (A change rewrites them anyway - this is for after a hand-edited
    /// `theme.css`, which the generated file is cut from.)
    Regenerate,
    /// No chip: the settings window's own page, which cannot act on itself.
    None,
}

/// A heading a run of rows sits under: the Display page's monitor header, and
/// the Shortcuts page's two groups.
///
/// A page whose rows naturally group - one output's fields, say - hands the same
/// heading to each row of the group, and the view draws it once, above them. That
/// is what keeps a page of two monitors from reading as one long list with the
/// output name repeated twelve times.
#[derive(Clone)]
pub struct Heading {
    /// The Lucide drawing beside the group's name.
    pub icon: &'static str,
    /// The group's name (`eDP-1`), also the thing its keys are qualified by.
    pub title: String,
    /// One quiet line about the group as it stands (`2560 × 1440 · 59.95 Hz`).
    pub detail: String,
    /// Draw the accent marker: this is the group the session is on.
    pub focused: bool,
}

/// One setting, resolved against the file that is on disk.
///
/// Everything is owned rather than borrowed from the catalogue: a page also shows
/// the keys the catalogue does *not* know (a hand-edited file can carry anything),
/// and those strings are made up as the page is built.
pub struct Field {
    pub key: String,
    /// What the state line calls this setting, when the full key is too long or
    /// too repetitive to print - the Display page shows `mode`, not
    /// `monitor:eDP-1:mode`, because the heading already names the output.
    pub key_label: Option<String>,
    /// The heading this row belongs under, if the page groups its rows.
    pub heading: Option<Heading>,
    pub label: String,
    pub help: String,
    /// The built-in default, when the catalogue knows this key. `None` is a key
    /// the file carries that the catalogue does not, which has nothing to go back
    /// to - so it gets no reset button.
    pub default: Option<String>,
    pub kind: Kind,
    pub value: String,
    /// Whether the file sets this key at all. An unset key still shows its
    /// default, because that is what the element is using - the state line says
    /// so, which is the difference between "40" and "40 because nobody said
    /// otherwise".
    pub is_default: bool,
    /// What the *system* has this setting at right now, when it can be asked:
    /// the Hyprland pages read each option back from the compositor, so a value
    /// set elsewhere stays visible instead of being quietly overridden.
    pub current: Option<String>,
    /// The names this setting may be set to, when it is one to *pick* rather than
    /// to type: a [`Kind::Choice`]'s own list, or the machine's XKB names for a
    /// [`Kind::Keymap`]. Empty means there is no list - which is what a `Keymap`
    /// row falls back to on a machine without `xkeyboard-config`, and what keeps
    /// it the text field it used to be.
    pub choices: Vec<Entry>,
    /// The keys of a shortcut row, when this row is one: they replace the setting's
    /// name with a keycap, because a shortcut has no key to write and no control to
    /// build - only keys and what they do (see the Shortcuts page).
    pub keycap: Option<String>,
}

type TabHandler = Rc<dyn Fn(usize)>;
type SectionHandler = Rc<dyn Fn(usize)>;
/// (file, key, value) - a `None` value means "go back to the built-in default".
type ChangeHandler = Rc<dyn Fn(&str, &str, Option<&str>)>;
/// The id of one of a page's buttons (see [`Kind::Actions`]).
type CommandHandler = Rc<dyn Fn(&str)>;
type VoidHandler = Rc<dyn Fn()>;

pub struct SettingsView {
    /// The element hands this to the shell as the card's content.
    pub root: gtk::Box,
    /// The row of tab buttons ("Hypr OSD", "Hyprland").
    tabs: gtk::Box,
    sidebar: gtk::Box,
    /// The rows of the shown page. Rebuilt wholesale on every section change: a
    /// page is a handful of widgets, and a control is wired to the key it edits -
    /// rebuilding is cheaper to reason about than rebinding.
    pane: gtk::Box,
    pane_title: gtk::Label,
    pane_blurb: gtk::Label,
    /// The rule under the card - the elements' one, or the compositor's.
    hint: gtk::Label,
    /// The page's own chip: "Restart" on an element's page, "Apply" on a
    /// Hyprland page, and hidden on the window's own (which cannot act on
    /// itself).
    action: gtk::Button,
    /// The width the text column of a row gets: the pane's width less the
    /// control's, so a help line can be capped before it can widen anything.
    text_width: i32,
    /// The same, for a sidebar row's title.
    section_text_width: i32,
    tab_buttons: RefCell<Vec<gtk::Button>>,
    section_buttons: RefCell<Vec<gtk::Button>>,
    on_tab: RefCell<Option<TabHandler>>,
    on_section: RefCell<Option<SectionHandler>>,
    on_change: RefCell<Option<ChangeHandler>>,
    on_action: RefCell<Option<VoidHandler>>,
    on_command: RefCell<Option<CommandHandler>>,
    on_close: RefCell<Option<VoidHandler>>,
    on_motion: RefCell<Option<VoidHandler>>,
}

impl SettingsView {
    pub fn new(sidebar_width: i32, pane_width: i32, pane_height: i32) -> Rc<SettingsView> {
        let view: Rc<SettingsView> = Rc::new_cyclic(|me: &Weak<SettingsView>| {
            let title = gtk::Label::new(Some("SETTINGS"));
            title.add_css_class("popup-title");
            title.set_xalign(0.0);

            let action = glyph_chip(names::ROTATE_CW, "Restart", {
                let me = me.clone();
                move || {
                    if let Some(view) = me.upgrade() {
                        view.emit_action();
                    }
                }
            });
            action.add_css_class("page-action");
            action.set_visible(false);

            let close = icon_button(names::CLOSE, "Close (Esc)");
            {
                let me = me.clone();
                close.connect_clicked(move |_| {
                    if let Some(view) = me.upgrade() {
                        view.emit_close();
                    }
                });
            }

            // The tabs, beside the window's own title: this collection, the
            // desktop it sits on, and the machine under both.
            let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            tabs.add_css_class("tabs");
            tabs.set_valign(gtk::Align::Center);

            let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            header.add_css_class("settings-header");
            header.append(&title);
            header.append(&tabs);
            header.append(&stretch());
            // The page's chip lives here rather than on the page's own title row,
            // where it would eat the blurb's whole width - it acts on whichever
            // page is showing, which is what it says on hover.
            header.append(&action);
            header.append(&close);

            // The page's own header - the element's name and a line about it. It
            // sits *above* the scroller rather than in it: a page title that
            // scrolled away with the rows would be no title at all.
            let pane_title = gtk::Label::new(None);
            pane_title.add_css_class("pane-title");
            pane_title.set_xalign(0.0);
            let pane_blurb = gtk::Label::new(None);
            pane_blurb.add_css_class("pane-blurb");
            pane_blurb.set_xalign(0.0);
            // Capped a touch under the pane: `cap_width` estimates a character's
            // advance, and an estimate that runs 3% long is 16px of card.
            text::cap_width(&pane_blurb, (pane_width - 24).max(120));
            pane_blurb.set_ellipsize(gtk::pango::EllipsizeMode::End);

            let pane_head = gtk::Box::new(gtk::Orientation::Vertical, 1);
            pane_head.append(&pane_title);
            pane_head.append(&pane_blurb);

            let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 2);
            sidebar.add_css_class("sidebar");
            sidebar.set_valign(gtk::Align::Start);
            sidebar.set_size_request(sidebar_width, -1);

            let pane = gtk::Box::new(gtk::Orientation::Vertical, 6);
            pane.add_css_class("pane");
            pane.set_size_request(pane_width - 2, -1);

            let scroller = gtk::ScrolledWindow::new();
            scroller.add_css_class("pane-scroll");
            scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            // The pane's width is the config's, not its content's: without this
            // the scroller reports the widest row as its own natural width and
            // the card grows past the number the user set (measured: 16px, which
            // also silently moved the sidebar out of the plan). The rows are
            // built to fit; anything that does not is clipped rather than
            // quietly widening the window.
            scroller.set_propagate_natural_width(false);
            scroller.set_size_request(pane_width, pane_height);
            scroller.set_child(Some(&pane));

            let right = gtk::Box::new(gtk::Orientation::Vertical, 6);
            right.set_hexpand(true);
            right.append(&pane_head);
            right.append(&scroller);

            let hint = gtk::Label::new(Some(KEYS_HINT));
            hint.add_css_class("hint");
            hint.set_xalign(0.0);
            text::cap_width(&hint, pane_width + sidebar_width - 24);
            hint.set_ellipsize(gtk::pango::EllipsizeMode::End);

            let body = gtk::Box::new(gtk::Orientation::Horizontal, BODY_GAP);
            body.append(&sidebar);
            body.append(&right);

            // The little capsule in the corner, the same grip every drawer in this
            // theme wears.
            let handle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            handle.add_css_class("handle");
            handle.set_halign(gtk::Align::End);
            handle.set_size_request(46, 4);

            let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
            separator.add_css_class("sep");

            let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
            root.add_css_class("settings");
            root.append(&handle);
            root.append(&header);
            root.append(&separator);
            root.append(&body);
            root.append(&hint);

            SettingsView {
                root,
                tabs,
                sidebar,
                pane,
                pane_title,
                pane_blurb,
                hint,
                action,
                text_width: (pane_width - ROW_OVERHEAD).max(120),
                section_text_width: (sidebar_width - SECTION_OVERHEAD).max(60),
                tab_buttons: RefCell::new(Vec::new()),
                section_buttons: RefCell::new(Vec::new()),
                on_tab: RefCell::new(None),
                on_section: RefCell::new(None),
                on_change: RefCell::new(None),
                on_action: RefCell::new(None),
                on_command: RefCell::new(None),
                on_close: RefCell::new(None),
                on_motion: RefCell::new(None),
            }
        });

        // Looking at the window is not being idle (see `main`'s watchdog).
        let motion = gtk::EventControllerMotion::new();
        {
            let view = view.clone();
            motion.connect_motion(move |_, _, _| view.emit_motion());
        }
        view.root.add_controller(motion);

        view
    }

    pub fn on_tab(&self, handler: impl Fn(usize) + 'static) {
        *self.on_tab.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn on_section(&self, handler: impl Fn(usize) + 'static) {
        *self.on_section.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn on_change(&self, handler: impl Fn(&str, &str, Option<&str>) + 'static) {
        *self.on_change.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn on_action(&self, handler: impl Fn() + 'static) {
        *self.on_action.borrow_mut() = Some(Rc::new(handler));
    }

    /// A page's own buttons ([`Kind::Actions`]): the handler is given the id the
    /// catalogue spelled, and the element decides what verb that is.
    pub fn on_command(&self, handler: impl Fn(&str) + 'static) {
        *self.on_command.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn on_close(&self, handler: impl Fn() + 'static) {
        *self.on_close.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn on_motion(&self, handler: impl Fn() + 'static) {
        *self.on_motion.borrow_mut() = Some(Rc::new(handler));
    }

    /// The tab row, rebuilt from the tabs - like the sidebar, so which one is lit
    /// follows the card rather than being something else's to remember.
    pub fn render_tabs(&self, rows: &[TabRow]) {
        while let Some(child) = self.tabs.first_child() {
            self.tabs.remove(&child);
        }
        let mut buttons = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let button = gtk::Button::new();
            button.add_css_class("tab");
            if row.selected {
                button.add_css_class("selected");
            }
            let label = gtk::Label::new(Some(&row.label));
            label.add_css_class("tab-name");
            button.set_child(Some(&label));
            button.set_focus_on_click(false);
            button.set_can_focus(false);
            button.set_tooltip_text(Some(&row.tooltip));
            {
                let handler = self.on_tab.borrow().clone();
                button.connect_clicked(move |_| {
                    if let Some(handler) = &handler {
                        handler(index);
                    }
                });
            }
            self.tabs.append(&button);
            buttons.push(button);
        }
        *self.tab_buttons.borrow_mut() = buttons;
    }

    /// The sidebar, rebuilt from the sections - so the selected row follows the
    /// card rather than being a class something else has to remember to move.
    pub fn render_sections(&self, rows: &[SectionRow]) {
        while let Some(child) = self.sidebar.first_child() {
            self.sidebar.remove(&child);
        }
        let mut buttons = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let button = gtk::Button::new();
            button.add_css_class("section");
            if row.selected {
                button.add_css_class("selected");
            }
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            line.append(&section_icon(row.icon));
            let label = gtk::Label::new(Some(&row.label));
            label.add_css_class("section-name");
            label.set_xalign(0.0);
            label.set_hexpand(true);
            text::cap_width(&label, self.section_text_width);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&label);
            button.set_child(Some(&line));
            button.set_focus_on_click(false);
            button.set_can_focus(false);
            button.set_tooltip_text(Some(&row.label));
            {
                let handler = self.on_section.borrow().clone();
                button.connect_clicked(move |_| {
                    if let Some(handler) = &handler {
                        handler(index);
                    }
                });
            }
            self.sidebar.append(&button);
            buttons.push(button);
        }
        *self.section_buttons.borrow_mut() = buttons;
    }

    /// Draw one element's page: its title, its blurb, and one row per field.
    ///
    /// `action` decides what the page's chip offers - an element's page is
    /// restarted, a Hyprland page re-applied, and the window's own page has no
    /// chip at all, because a window cannot act on itself.
    ///
    /// `banner` is a widget that belongs above the rows and is not a row itself -
    /// the Display page's arrangement editor. It is passed in rather than built
    /// here because it edits the page by gesture and therefore has to know the
    /// monitors the element read.
    pub fn show_section(
        &self,
        section: &Section,
        fields: &[Field],
        action: PageAction,
        banner: Option<&gtk::Widget>,
    ) {
        self.pane_title.set_text(section.title);
        self.pane_blurb.set_text(section.blurb);
        self.hint.set_text(match section.applies {
            Applies::Element => KEYS_HINT,
            Applies::Energy => ENERGY_HINT,
            // Regenerating is the whole of it, and it has already happened by the
            // time the page is drawn.
            Applies::Lock => LOCK_HINT,
            // A page of keys is reference: nothing on it can be changed, so the
            // footer says where the bound half really lives instead.
            Applies::Shortcuts => SHORTCUTS_HINT,
            // A display change is applied to the compositor at once, exactly like
            // an option: the footer says the same thing on both pages.
            Applies::Compositor | Applies::Displays => HYPRLAND_HINT,
        });
        set_chip(&self.action, action);

        while let Some(child) = self.pane.first_child() {
            self.pane.remove(&child);
        }
        // The page's own picture, when it has one, before the first row.
        if let Some(banner) = banner {
            self.pane.append(banner);
        }
        // A page may group its rows (the Display page: one group per output). The
        // heading is drawn once, when it changes - the rows arrive already ordered
        // by group - and the rows under it are inset so the grouping is visible
        // rather than merely implied by the order.
        let mut drawn: Option<String> = None;
        for field in fields {
            let grouped = field.heading.is_some();
            if let Some(heading) = &field.heading {
                if drawn.as_deref() != Some(heading.title.as_str()) {
                    drawn = Some(heading.title.clone());
                    self.pane.append(&group_header(heading));
                }
            }
            let row = self.field_row(section.file, field);
            if grouped {
                row.add_css_class("grouped");
            }
            self.pane.append(&row);
        }
    }

    /// What the pane measured at - the number that catches a widget quietly
    /// widening the window (`status` prints it).
    pub fn measured(&self) -> String {
        format!("{}x{}", self.pane.width(), self.pane.height())
    }

    /// One setting: its name and the line that says what it is worth, and the
    /// control that edits it at the row's right end.
    ///
    /// Two kinds get no control at all: a [`Kind::Info`] reading states what the
    /// machine is doing (the value is the row), and a [`Kind::Actions`] row puts
    /// its buttons where the control would be, because that is the only thing it
    /// has.
    fn field_row(&self, element: &str, field: &Field) -> gtk::Box {
        // A shortcut is not a setting: it has keys in place of a name and nothing
        // to press, so it gets its own row shape rather than a control that is
        // never drawn.
        if field.keycap.is_some() {
            return self.shortcut_row(field);
        }
        let reading = matches!(field.kind, Kind::Info | Kind::Actions(_));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("setting-row");
        if matches!(field.kind, Kind::Info) {
            row.add_css_class("reading-row");
        }

        let name = gtk::Label::new(Some(&field.label));
        name.add_css_class("setting-label");
        name.set_xalign(0.0);
        text::cap_width(&name, self.text_width);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let state = gtk::Label::new(Some(&state_text(field)));
        state.add_css_class("setting-help");
        state.set_xalign(0.0);
        text::cap_width(&state, self.text_width);
        state.set_ellipsize(gtk::pango::EllipsizeMode::End);
        // A reading has no key to name: its tooltip is the sentence about where
        // the value came from, and nothing else.
        state.set_tooltip_text(Some(&if reading {
            field.help.clone()
        } else {
            format!("{}\n\nkey: {}", field.help, field.key)
        }));

        let text_box = gtk::Box::new(gtk::Orientation::Vertical, 1);
        text_box.set_hexpand(true);
        text_box.set_valign(gtk::Align::Center);
        text_box.append(&name);
        text_box.append(&state);

        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        controls.add_css_class("setting-control");
        controls.set_valign(gtk::Align::Center);
        controls.set_hexpand(false);

        if let Kind::Actions(commands) = field.kind {
            for command in commands {
                controls.append(&self.command_button(command));
            }
        } else if !reading {
            // A reset is only offered for a key that is *set*: a key already
            // falling back to its default has nothing to go back to. Resetting
            // asks the element to rewrite the file and draw the page again, so the
            // control and the state line come back telling the same story.
            if !field.is_default && field.default.is_some() {
                let reset = icon_button(names::ROTATE_CW, "Reset to the built-in default");
                reset.add_css_class("reset");
                let handler = self.on_change.borrow().clone();
                let element = element.to_string();
                let key = field.key.clone();
                reset.connect_clicked(move |_| {
                    if let Some(handler) = &handler {
                        handler(&element, &key, None);
                    }
                });
                controls.append(&reset);
            }

            controls.append(&self.control(element, field, &state));
        }

        row.append(&text_box);
        row.append(&controls);
        row
    }

    /// One keyboard shortcut: its keys as a keycap at the row's start, then what
    /// they do and the sentence about it.
    ///
    /// The keycap column is a *fixed* width, so thirty rows of different keys line
    /// their actions up - the same reason a page of settings has one control
    /// column. The keys are right-aligned inside it, so the column's edge is what
    /// the eye follows down the page.
    fn shortcut_row(&self, field: &Field) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("setting-row");
        row.add_css_class("shortcut-row");

        let keys = gtk::Label::new(field.keycap.as_deref());
        keys.add_css_class("keycap");
        keys.set_valign(gtk::Align::Center);
        // Right-aligned inside the fixed column: a label's xalign is where its
        // text sits in an allocation wider than itself, which is exactly this.
        keys.set_xalign(1.0);
        keys.set_size_request(KEYCAP_WIDTH, -1);
        // A capped label, because a keycap is the one string here that is not the
        // pane's to shorten - without this the longest one would widen the row.
        text::cap_width(&keys, KEYCAP_WIDTH - 16);
        keys.set_ellipsize(gtk::pango::EllipsizeMode::End);
        keys.set_tooltip_text(Some(&field.help));

        let name = gtk::Label::new(Some(&field.label));
        name.add_css_class("setting-label");
        name.set_xalign(0.0);
        text::cap_width(&name, self.text_width);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let state = gtk::Label::new(Some(&field.help));
        state.add_css_class("setting-help");
        state.set_xalign(0.0);
        text::cap_width(&state, self.text_width);
        state.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let text_box = gtk::Box::new(gtk::Orientation::Vertical, 1);
        text_box.set_hexpand(true);
        text_box.set_valign(gtk::Align::Center);
        text_box.append(&name);
        text_box.append(&state);

        row.append(&keys);
        row.append(&text_box);
        row
    }

    /// One of a page's own buttons ([`Kind::Actions`]): the same chip the page's
    /// restart/apply button wears, wired to the id the catalogue spelled.
    fn command_button(&self, command: &Command) -> gtk::Button {
        let handler = self.on_command.borrow().clone();
        let id = command.id;
        glyph_chip(command.icon, command.label, move || {
            if let Some(handler) = &handler {
                handler(id);
            }
        })
    }

    /// A field's control on its own, without the row around it: the Display
    /// page's per-output popover shows one of these per field. The state line a
    /// control updates on every change is a throwaway here - the popover is
    /// rebuilt from the file after every write, so no label has to carry it.
    pub fn field_control(&self, element: &str, field: &Field) -> gtk::Widget {
        let state = gtk::Label::new(None);
        self.control(element, field, &state)
    }

    /// The control itself: one per [`Kind`], and nothing else.
    ///
    /// A value outside the range the catalogue gives widens the control's own
    /// range rather than being clamped: a file that says `width = 2000` is a file
    /// the window has to be able to show honestly, and a spinner that silently
    /// pulled it down to its maximum would rewrite it on the next click.
    fn control(&self, element: &str, field: &Field, state: &gtk::Label) -> gtk::Widget {
        let handler = self.on_change.borrow().clone();
        match field.kind {
            Kind::Bool => {
                let switch = gtk::Switch::new();
                switch.set_active(matches!(field.value.as_str(), "true" | "yes" | "on" | "1"));
                switch.set_valign(gtk::Align::Center);
                switch.set_tooltip_text(Some(&field.help));
                {
                    let (element, key) = (element.to_string(), field.key.clone());
                    let state = state.clone();
                    switch.connect_state_set(move |_, active| {
                        let text = if active { "true" } else { "false" };
                        if let Some(handler) = &handler {
                            handler(&element, &key, Some(text));
                        }
                        state.set_text(&state_text_of(&key, text));
                        glib::Propagation::Proceed
                    });
                }
                switch.upcast()
            }
            Kind::Int { min, max, step } => {
                let current: i32 = field.value.parse().unwrap_or(min);
                let spinner = gtk::SpinButton::with_range(
                    f64::from(min.min(current)),
                    f64::from(max.max(current)),
                    f64::from(step.max(1)),
                );
                spinner.set_value(f64::from(current));
                spinner.set_digits(0);
                spinner.set_size_request(90, -1);
                spinner.set_valign(gtk::Align::Center);
                spinner.set_tooltip_text(Some(&field.help));
                {
                    let (element, key) = (element.to_string(), field.key.clone());
                    let state = state.clone();
                    spinner.connect_value_changed(move |spinner| {
                        let text = format!("{}", spinner.value().round() as i32);
                        if let Some(handler) = &handler {
                            handler(&element, &key, Some(&text));
                        }
                        state.set_text(&state_text_of(&key, &text));
                    });
                }
                spinner.upcast()
            }
            Kind::Fraction { min, max } => {
                let current: f64 = field.value.parse().unwrap_or(min);
                let spinner = gtk::SpinButton::with_range(min.min(current), max.max(current), 0.01);
                spinner.set_value(current);
                spinner.set_digits(2);
                spinner.set_size_request(90, -1);
                spinner.set_valign(gtk::Align::Center);
                spinner.set_tooltip_text(Some(&field.help));
                {
                    let (element, key) = (element.to_string(), field.key.clone());
                    let state = state.clone();
                    spinner.connect_value_changed(move |spinner| {
                        let text = format!("{:.2}", spinner.value());
                        if let Some(handler) = &handler {
                            handler(&element, &key, Some(&text));
                        }
                        state.set_text(&state_text_of(&key, &text));
                    });
                }
                spinner.upcast()
            }
            Kind::Text => self.text_control(element, field, state),
            // A list to pick from - unless there is no list, which is what a
            // machine without xkeyboard-config answers with: then the row stays
            // the field it was, rather than an empty dropdown. A display list is
            // the monitor's own (its modes, the scales, the other outputs) and is
            // empty only when `hyprctl` could not be asked.
            Kind::Choice(_) | Kind::Keymap(_) | Kind::Monitor(_) if field.choices.is_empty() => {
                self.text_control(element, field, state)
            }
            Kind::Choice(_) | Kind::Keymap(_) | Kind::Monitor(_) => {
                self.choice_control(element, field, state)
            }
            // Never reached: `field_row` draws a reading as its own value and a
            // row of buttons as buttons, and asks for no control for either.
            Kind::Info | Kind::Actions(_) => {
                gtk::Box::new(gtk::Orientation::Horizontal, 0).upcast()
            }
        }
    }

    /// A line of text: the control for [`Kind::Text`], and the fallback for a
    /// list that could not be read.
    fn text_control(&self, element: &str, field: &Field, state: &gtk::Label) -> gtk::Widget {
        let handler = self.on_change.borrow().clone();
        let entry = gtk::Entry::new();
        entry.add_css_class("setting-text");
        entry.set_text(&field.value);
        entry.set_size_request(CONTROL_WIDTH, -1);
        entry.set_valign(gtk::Align::Center);
        entry.set_tooltip_text(Some(&field.help));
        // Typing into a field must not rewrite the file on every keystroke, so
        // the value is committed when the field is *finished* - Enter, or leaving
        // it.
        let commit = {
            let handler = handler.clone();
            let element = element.to_string();
            let key = field.key.clone();
            let state = state.clone();
            Rc::new(move |text: &str| {
                if let Some(handler) = &handler {
                    handler(&element, &key, Some(text));
                }
                state.set_text(&state_text_of(&key, text));
            })
        };
        {
            let commit = commit.clone();
            entry.connect_activate(move |entry| commit(entry.text().as_str()));
        }
        {
            let commit = commit.clone();
            let entry_for_leave = entry.clone();
            let focus = gtk::EventControllerFocus::new();
            focus.connect_leave(move |_| commit(entry_for_leave.text().as_str()));
            entry.add_controller(focus);
        }
        entry.upcast()
    }

    /// A dropdown: the control for a value that has to be one of a list of names.
    ///
    /// The labels carry the description the machine gives each name (`ch ·
    /// German (Switzerland)`), because a list of bare codes is exactly as hard to
    /// read as the field it replaces - and with the search field GTK puts in the
    /// popup of a long list, "swiss" is how you find `ch`. The labels are capped,
    /// so one long description cannot widen the card, and so the column of
    /// controls stays the same width from row to row.
    fn choice_control(&self, element: &str, field: &Field, state: &gtk::Label) -> gtk::Widget {
        let handler = self.on_change.borrow().clone();

        // The value the file has has to be *in* the list - it is what the row
        // shows - and an empty one has to be sayable, because "this collection has
        // no opinion" is a state the file can be in. `(none)` is that state: it
        // takes the key back out of the file instead of writing an empty value.
        let mut labels: Vec<String> = Vec::new();
        let mut values: Vec<Option<String>> = Vec::new();
        let mut selected = 0;
        if field.value.is_empty() {
            labels.push("(none)".to_string());
            values.push(None);
        } else {
            for entry in &field.choices {
                if entry.name == field.value {
                    selected = values.len();
                }
                labels.push(entry.display());
                values.push(Some(entry.name.clone()));
            }
            if values[selected].as_deref() != Some(field.value.as_str()) {
                // A value the list does not know (`de,us` is a list of them, and a
                // machine may simply not have the rules file in force) is still
                // what the file says: it gets an entry of its own rather than the
                // dropdown quietly showing somebody else's name.
                labels.push(field.value.clone());
                values.push(Some(field.value.clone()));
                selected = values.len() - 1;
            }
        }

        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let model = gtk::StringList::new(&refs);
        // The expression is what the dropdown reads for an item's text - the
        // factory below draws it, and the search entry filters by it.
        let expression = gtk::PropertyExpression::new(
            gtk::StringObject::static_type(),
            None::<&gtk::Expression>,
            "string",
        );
        let dropdown = gtk::DropDown::builder()
            .model(&model)
            .expression(&expression)
            .factory(&choice_factory())
            .build();
        dropdown.add_css_class("setting-choice");
        dropdown.set_valign(gtk::Align::Center);
        dropdown.set_tooltip_text(Some(&field.help));
        dropdown.set_size_request(CHOICE_WIDTH, -1);
        // Ninety-nine layouts - or two hundred and thirty-nine options - are
        // found by typing, not by scrolling.
        if labels.len() > SEARCH_FROM {
            dropdown.set_enable_search(true);
        }
        // Set before the handler is connected: picking the value it already has is
        // not a change, and this is not one either.
        dropdown.set_selected(selected as u32);

        {
            let (element, key) = (element.to_string(), field.key.clone());
            let state = state.clone();
            dropdown.connect_selected_notify(move |dropdown| {
                let value = values.get(dropdown.selected() as usize).cloned().flatten();
                if let Some(handler) = &handler {
                    handler(&element, &key, value.as_deref());
                }
                state.set_text(&state_text_of(&key, value.as_deref().unwrap_or_default()));
            });
        }
        dropdown.upcast()
    }

    fn emit_action(&self) {
        if let Some(handler) = self.on_action.borrow().clone() {
            handler();
        }
    }

    fn emit_close(&self) {
        if let Some(handler) = self.on_close.borrow().clone() {
            handler();
        }
    }

    fn emit_motion(&self) {
        if let Some(handler) = self.on_motion.borrow().clone() {
            handler();
        }
    }
}

/// The state line under a setting's name: the key, and what it is worth *now*.
pub fn state_text_of(key: &str, value: &str) -> String {
    let shown = if value.is_empty() { "(empty)" } else { value };
    format!("{key} = {shown}")
}

/// The state line for a field as it was read: an unset key shows its default and
/// says that is what it is, and a key the system can be asked about shows what it
/// has right now.
///
/// The name it prints is the field's `key_label` when it has one - a grouped row
/// (`mode`) says less than its full key (`monitor:eDP-1:mode`) because the heading
/// above it already says which output the key is about.
fn state_text(field: &Field) -> String {
    // A reading has no key and no default: it *is* the value the machine reports,
    // and the label beside it already says what it is about.
    if matches!(field.kind, Kind::Info) {
        return if field.value.is_empty() {
            "unknown".to_string()
        } else {
            field.value.clone()
        };
    }
    // A row of buttons says what the buttons do in the place a value would be.
    if matches!(field.kind, Kind::Actions(_)) {
        return field.help.clone();
    }
    let key = field.key_label.as_deref().unwrap_or(&field.key);
    let mut line = if field.is_default {
        let default = field.default.clone().unwrap_or_default();
        let shown = if default.is_empty() {
            "(empty)".to_string()
        } else {
            default
        };
        format!("{key} = {shown}  \u{b7}  default")
    } else {
        state_text_of(key, &field.value)
    };
    if let Some(current) = &field.current {
        let shown = if current.is_empty() {
            "(empty)".to_string()
        } else {
            current.clone()
        };
        line.push_str(&format!("  \u{b7}  now {shown}"));
    }
    line
}

/// Point the page chip at what it is for this page - the same drawing for both,
/// because in both cases the chip means "make the file's values take effect",
/// and the word is what says how.
fn set_chip(chip: &gtk::Button, action: PageAction) {
    let (word, tooltip) = match action {
        PageAction::Restart => (
            "Restart",
            "Stop this element and start it again, so it reads the config just written",
        ),
        PageAction::Apply => (
            "Apply",
            "Write the generated config and hand it to Hyprland again - a change is \
             applied as you make it, so this is for re-applying a file edited by hand",
        ),
        PageAction::Regenerate => (
            "Re-apply",
            "Write the lock screen's generated variables again, from theme.css and \
             lock.conf - a change is written as you make it, so this is for after \
             retinting the theme by hand",
        ),
        PageAction::None => {
            chip.set_visible(false);
            return;
        }
    };
    chip.set_visible(true);
    chip.set_tooltip_text(Some(tooltip));
    let word_label = chip
        .child()
        .and_downcast::<gtk::Box>()
        .and_then(|row| row.last_child())
        .and_downcast::<gtk::Label>()
        .expect("a chip's last child is its word");
    word_label.set_text(word);
}

/// What draws the entries of a dropdown - the button part and the popup part are
/// the same drawing.
///
/// A custom factory, rather than `DropDown::from_strings`, for one reason: the
/// label has to be *capped*. A `GtkDropDown`'s natural width is its widest item,
/// and these items carry a description, so the widest one - one of the two
/// hundred options, say - would otherwise decide how wide the row is. Capped and
/// ellipsised, the widest item costs nothing.
fn choice_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let label = gtk::Label::new(None);
        label.add_css_class("choice-name");
        label.set_xalign(0.0);
        text::cap_width(&label, CHOICE_TEXT);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        item.set_child(Some(&label));
    });
    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(label) = item.child().and_downcast::<gtk::Label>() else {
            return;
        };
        let text = item
            .item()
            .and_downcast::<gtk::StringObject>()
            .map(|object| object.string())
            .unwrap_or_default();
        label.set_text(&text);
    });
    factory
}

/// A round button whose whole content is one icon. The tooltip carries the words,
/// because there is nowhere else for them to go.
fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("iconbtn");
    button.set_child(Some(&icons::lucide(icon, BUTTON_ICON)));
    button.set_focus_on_click(false);
    button.set_can_focus(false);
    button.set_tooltip_text(Some(tooltip));
    button
}

/// A chip with a leading icon and a word: the restart button, and the same shape
/// the other panels' own actions wear.
fn glyph_chip(icon: &str, word: &str, action: impl Fn() + 'static) -> gtk::Button {
    let mark = icons::lucide(icon, BUTTON_ICON);
    let label = gtk::Label::new(Some(word));
    // The word carries its own class rather than inheriting the button's size:
    // `base.css` sets a font size on `*`, so inheritance never reaches a label.
    label.add_css_class("btn-word");
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    line.add_css_class("btn-row");
    line.append(&mark);
    line.append(&label);

    let button = gtk::Button::new();
    button.add_css_class("chipbtn");
    button.set_child(Some(&line));
    button.set_focus_on_click(false);
    button.set_can_focus(false);
    button.set_tooltip_text(Some(word));
    button.connect_clicked(move |_| action());
    button
}

/// A section's mark in the sidebar: the size is the pixel size here - a drawing
/// has no font-size - so what is left to CSS is the colour.
fn section_icon(icon: &str) -> gtk::Image {
    let mark = icons::lucide(icon, SECTION_ICON);
    mark.add_css_class("section-icon");
    mark
}

/// The heading over a group of rows: the group's mark, its name, the quiet line
/// about it as it stands, and - when it is the group the session is on - a small
/// marker.
///
/// A heading is not a row: it carries no control and cannot be changed. It is what
/// turns a page of twelve display keys into two groups of six, each one visibly
/// about *that* output.
fn group_header(heading: &Heading) -> gtk::Box {
    let mark = icons::lucide(heading.icon, HEADING_ICON);
    mark.add_css_class("monitor-icon");

    let name = gtk::Label::new(Some(&heading.title));
    name.add_css_class("monitor-name");
    name.set_xalign(0.0);

    let detail = gtk::Label::new(Some(&heading.detail));
    detail.add_css_class("monitor-detail");
    detail.set_xalign(0.0);

    let line = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    line.add_css_class("monitor-head");
    line.append(&mark);
    line.append(&name);
    line.append(&detail);
    if heading.focused {
        // The shared chip, in its accent state: the same mark the cards use for
        // "this one is live".
        let chip = gtk::Label::new(Some("focused"));
        chip.add_css_class("chip");
        chip.add_css_class("accent");
        chip.set_valign(gtk::Align::Center);
        line.append(&chip);
    }
    line.append(&stretch());
    line
}

/// An empty box that takes the slack in a row.
fn stretch() -> gtk::Box {
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    spacer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_line_names_the_key_and_its_value() {
        assert_eq!(state_text_of("height", "40"), "height = 40");
        // An empty value is shown as such rather than as nothing at all: a key
        // that is set to nothing is not a key that is absent.
        assert_eq!(state_text_of("output", ""), "output = (empty)");
    }
}

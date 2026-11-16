//! The bluetooth manager card: the controller, every device it knows, and the
//! one action that makes sense for each of them.
//!
//! The card is a *browser* rather than a notification: a header that says what
//! the radio is doing, a filter, a scan, and two lists - what is paired, and
//! what a scan has merely seen. Every row carries the device's own mark, its
//! battery and its link quality, because those are the three things a person
//! actually looks for when a headset will not connect, and the action button
//! follows from the device's state: connect, disconnect, or pair.
//!
//! ```text
//!  ┌──────────────────────────────────────────────────────────────────┐
//!  │  ▣  Bluetooth                                    ⏻  ◉  ✕        │
//!  │     mercury · On · 1 connected                                  │
//!  │  ┌────────────────────────────┐  [⌕ Scan]  [↻]                 │
//!  │  │ filter devices…            │                                 │
//!  │  └────────────────────────────┘                                 │
//!  │ ──────────────────────────────────────────────────────────────── │
//!  │  PAIRED                                         2               │
//!  │  ▍ ▣ WH-1000XM4                                  ▂▄▆ 82%  [⧉]  │
//!  │     Connected · AA:BB:CC:DD:EE:FF                               │
//!  │    ⌨ K380                                       ▂▄      [⧉] [✕]│
//!  │     Paired · 11:22:33:44:55:66                                  │
//!  │  NEARBY                                         1               │
//!  │    ◎ MX Master                                   ▂▄       [+]   │
//!  │     Nearby · CC:DD:EE:FF:00:11                                  │
//!  │ ──────────────────────────────────────────────────────────────── │
//!  │  ↑ ↓ select · Enter connect · Esc close                         │
//!  └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! The view paints what it is handed and runs the commands its buttons mean. It
//! does not read anything itself: `main` owns the readings and the clock, so
//! there is one place that decides *when* to read and one that decides how to
//! draw. The one piece of state it keeps is what the last reading said - a
//! button like "Disconnect" has to know the device is connected to say what it
//! does - plus the filtered result and which row the keyboard is on.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;

use hypr_osd_core::bluetooth::{Bluetooth, Device, DeviceAction};
use hypr_osd_core::icons::{self, names};

/// The four partial blocks a device's link-quality meter fills from the left.
///
/// These stay *text*, unlike everything else here: they are a meter drawn by
/// glyph composition, and U+2582..U+2588 are ordinary Unicode that every
/// monospace font has - nothing to bundle.
const SIGNAL_BARS: &str = "\u{2582}\u{2584}\u{2586}\u{2588}";

const SECTION_ICON: i32 = 12;
const BUTTON_ICON: i32 = 13;
const DEVICE_ICON: i32 = 18;
const BATTERY_ICON: i32 = 11;
const HERO_ICON: i32 = 22;

/// How long a row's button spins after it was clicked. Long enough to cover
/// `bluetoothctl` talking to a device, short enough that a spinner is never
/// stuck turning for good.
const ACTION_TIMEOUT: Duration = Duration::from_secs(8);

/// What a pairing attempt may hold its row's button for. See the system popup's
/// copy of this constant - it is the same conversation, so the same patience.
const PAIR_PATIENCE: Duration = Duration::from_secs(45);

/// How long the one-line note under the list stays up. Long enough to be read,
/// short enough that it is gone before it turns into furniture.
pub const NOTE_TIME: Duration = Duration::from_secs(8);

fn patience(action: DeviceAction) -> Duration {
    match action {
        DeviceAction::Pair => PAIR_PATIENCE,
        _ => ACTION_TIMEOUT,
    }
}

/// What the last reading said - what the rows and their buttons are drawn from.
#[derive(Default)]
struct Last {
    bluetooth: Bluetooth,
    devices: Vec<Device>,
    scanning: bool,
    query: String,
}

/// A device whose button was just clicked and whose answer has not arrived yet.
struct Pending {
    address: String,
    action: DeviceAction,
    until: Instant,
}

type Tick = Rc<dyn Fn()>;
type Action = Rc<dyn Fn(&str, DeviceAction)>;
type Query = Rc<dyn Fn(&str)>;

pub struct BluetoothView {
    /// The card's content: the shell's card widget is its parent.
    pub root: gtk::Box,
    /// The filter, and the field that raises the on-screen keyboard on a tablet.
    pub entry: gtk::Entry,
    title: gtk::Label,
    subtitle: gtk::Label,
    power: gtk::Button,
    visible: gtk::Button,
    close: gtk::Button,
    scan: gtk::Button,
    scan_idle: gtk::Widget,
    refresh: gtk::Button,
    scroller: gtk::ScrolledWindow,
    /// Where the section headings, the device rows and the empty states are
    /// built. Rebuilt whole on every read - devices come and go.
    list: gtk::Box,
    /// The rows currently drawn, by address. Selection is a class change on
    /// these, never a rebuild: a rebuild on press would destroy the button under
    /// the pointer before its release could turn into a click.
    rows: RefCell<Vec<(String, gtk::Box)>>,
    hint: gtk::Label,
    note: gtk::Label,
    footer: gtk::Label,
    last: RefCell<Last>,
    pending: RefCell<Option<Pending>>,
    /// The address the keyboard is on. An address rather than an index, because
    /// the list is rebuilt underneath it (a device connects, a scan ends) and an
    /// index would silently point at the next row.
    selected: RefCell<Option<String>>,
    on_scan: RefCell<Option<Tick>>,
    on_refresh: RefCell<Option<Tick>>,
    on_power: RefCell<Option<Tick>>,
    on_discoverable: RefCell<Option<Tick>>,
    on_close: RefCell<Option<Tick>>,
    on_action: RefCell<Option<Action>>,
    on_query: RefCell<Option<Query>>,
    /// This view, weakly: a row is built by `&self`, but its buttons have to
    /// reach back into the `Rc` that owns the state. Patched in by `wire`.
    me: RefCell<Weak<BluetoothView>>,
    list_height: i32,
}

impl BluetoothView {
    pub fn new(list_height: i32) -> Rc<BluetoothView> {
        // ---- the hero: what the radio is doing ------------------------------
        let mark = icons::lucide(names::BLUETOOTH, HERO_ICON);
        mark.add_css_class("hero-icon");
        // A `GtkBox` hands a child its natural width at the box's start, so the
        // drawing takes the slack to sit in the middle of its square - and the
        // square refuses that slack for itself, or it grows around the drawing.
        mark.set_hexpand(true);
        let hero_mark = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        hero_mark.add_css_class("hero-mark");
        hero_mark.set_valign(gtk::Align::Center);
        hero_mark.set_hexpand(false);
        hero_mark.append(&mark);

        let title = gtk::Label::new(Some("Bluetooth"));
        title.add_css_class("hero-title");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_max_width_chars(28);
        let subtitle = gtk::Label::new(Some("No controller"));
        subtitle.add_css_class("hero-sub");
        subtitle.set_xalign(0.0);
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
        subtitle.set_max_width_chars(44);
        let hero_text = gtk::Box::new(gtk::Orientation::Vertical, 1);
        hero_text.set_valign(gtk::Align::Center);
        hero_text.set_hexpand(true);
        hero_text.append(&title);
        hero_text.append(&subtitle);

        let power = icon_button(names::POWER, "Turn bluetooth on");
        let visible = icon_button(names::EYE_OFF, "Hidden");
        let close = icon_button(names::CLOSE, "Close (Esc)");
        close.add_css_class("close");

        let hero = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        hero.add_css_class("hero");
        hero.append(&hero_mark);
        hero.append(&hero_text);
        hero.append(&power);
        hero.append(&visible);
        hero.append(&close);

        // ---- the filter and the two tools ----------------------------------
        let search_icon = icons::lucide(names::SEARCH, 13);
        search_icon.add_css_class("search-icon");
        let entry = gtk::Entry::new();
        entry.add_css_class("filter");
        entry.set_placeholder_text(Some("Filter devices…"));
        entry.set_hexpand(true);
        entry.set_width_chars(12);
        let search = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        search.add_css_class("search");
        search.set_hexpand(true);
        search.set_valign(gtk::Align::Center);
        search.append(&search_icon);
        search.append(&entry);

        let scan = gtk::Button::new();
        scan.add_css_class("chipbtn");
        scan.set_tooltip_text(Some("Look for devices nearby for a few seconds"));
        let scan_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        scan_row.add_css_class("btn-row");
        scan_row.append(&icons::lucide(names::SEARCH, BUTTON_ICON));
        let scan_word = gtk::Label::new(Some("Scan"));
        scan_word.add_css_class("btn-word");
        scan_row.append(&scan_word);
        scan.set_child(Some(&scan_row));
        let scan_idle = scan.child().expect("the scan chip has its idle content");

        let refresh = icon_button(names::ROTATE_CW, "Read the devices again");

        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        tools.add_css_class("tools");
        tools.append(&search);
        tools.append(&scan);
        tools.append(&refresh);

        // ---- the lists -----------------------------------------------------
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        list.add_css_class("dev-lists");
        let hint = gtk::Label::new(None);
        hint.add_css_class("dev-hint");
        hint.set_xalign(0.5);
        hint.set_visible(false);
        hint.set_wrap(true);
        hint.set_max_width_chars(48);
        let note = gtk::Label::new(None);
        note.add_css_class("dev-note");
        note.set_xalign(0.0);
        note.set_wrap(true);
        note.set_visible(false);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.add_css_class("list-body");
        body.append(&list);
        body.append(&hint);
        body.append(&note);

        let scroller = gtk::ScrolledWindow::new();
        scroller.add_css_class("dev-scroll");
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_size_request(-1, list_height.max(120));
        scroller.set_child(Some(&body));
        // A scroller propagates its child's *natural* width and would widen the
        // card around a long device name (the settings window's pane learned
        // this the expensive way); the configured width is the authority here.
        scroller.set_propagate_natural_width(false);
        scroller.set_hexpand(true);

        let footer = gtk::Label::new(Some("↑ ↓ select · Enter connect · Esc close"));
        footer.add_css_class("footer-hint");
        footer.set_xalign(0.0);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("bluetooth");
        root.append(&hero);
        root.append(&separator());
        root.append(&tools);
        root.append(&scroller);
        root.append(&separator());
        root.append(&footer);

        let view = Rc::new(BluetoothView {
            root,
            entry,
            title,
            subtitle,
            power,
            visible,
            close,
            scan,
            scan_idle,
            refresh,
            scroller,
            list,
            rows: RefCell::new(Vec::new()),
            hint,
            note,
            footer,
            last: RefCell::new(Last::default()),
            pending: RefCell::new(None),
            selected: RefCell::new(None),
            on_scan: RefCell::new(None),
            on_refresh: RefCell::new(None),
            on_power: RefCell::new(None),
            on_discoverable: RefCell::new(None),
            on_close: RefCell::new(None),
            on_action: RefCell::new(None),
            on_query: RefCell::new(None),
            me: RefCell::new(Weak::new()),
            list_height,
        });
        // The buttons are wired once, after the `Rc` exists: the list is rebuilt
        // wholesale, but the header's controls live as long as the card.
        view.wire();
        view
    }

    /// Connect every control that is not rebuilt with the list.
    fn wire(self: &Rc<BluetoothView>) {
        *self.me.borrow_mut() = Rc::downgrade(self);
        let view = Rc::clone(self);
        self.close
            .connect_clicked(move |_| view.fire(&view.on_close));

        let view = Rc::clone(self);
        self.power
            .connect_clicked(move |_| view.fire(&view.on_power));

        let view = Rc::clone(self);
        self.visible
            .connect_clicked(move |_| view.fire(&view.on_discoverable));

        let view = Rc::clone(self);
        self.scan.connect_clicked(move |_| view.fire(&view.on_scan));

        let view = Rc::clone(self);
        self.refresh
            .connect_clicked(move |_| view.fire(&view.on_refresh));

        let view = Rc::clone(self);
        self.entry.connect_changed(move |entry| {
            let query = entry.text().to_string();
            view.last.borrow_mut().query = query.clone();
            if let Some(on_query) = view.on_query.borrow().as_ref() {
                on_query(&query);
            }
            view.rebuild();
        });
    }

    pub fn on_scan(&self, callback: Tick) {
        *self.on_scan.borrow_mut() = Some(callback);
    }

    pub fn on_refresh(&self, callback: Tick) {
        *self.on_refresh.borrow_mut() = Some(callback);
    }

    pub fn on_power(&self, callback: Tick) {
        *self.on_power.borrow_mut() = Some(callback);
    }

    pub fn on_discoverable(&self, callback: Tick) {
        *self.on_discoverable.borrow_mut() = Some(callback);
    }

    pub fn on_close(&self, callback: Tick) {
        *self.on_close.borrow_mut() = Some(callback);
    }

    pub fn on_action(&self, callback: Action) {
        *self.on_action.borrow_mut() = Some(callback);
    }

    pub fn on_query(&self, callback: Query) {
        *self.on_query.borrow_mut() = Some(callback);
    }

    fn fire(&self, callback: &RefCell<Option<Tick>>) {
        if let Some(callback) = callback.borrow().as_ref() {
            callback();
        }
    }

    fn strong(&self) -> Option<Rc<BluetoothView>> {
        self.me.borrow().upgrade()
    }

    /// Put the keyboard in the filter (what raises the on-screen keyboard on a
    /// tablet), and paint it selected so the field reads as ready.
    pub fn focus_entry(&self) {
        self.entry.grab_focus();
    }

    pub fn set_query(&self, query: &str) {
        if self.entry.text().as_str() == query {
            return;
        }
        // `set_text` fires `changed`, which runs the caller's callback; that is
        // the single path, so there is no second way for the filter to change.
        self.entry.set_text(query);
    }

    /// Paint a whole read: the controller, the devices, and whether a scan is
    /// running.
    pub fn render(&self, bluetooth: Bluetooth, devices: Vec<Device>, scanning: bool) {
        {
            let mut last = self.last.borrow_mut();
            last.bluetooth = bluetooth;
            last.devices = devices;
            last.scanning = scanning;
        }
        self.rebuild();
    }

    /// The scan started or ended. The list is not re-read here - the element
    /// re-reads it when the scan ends - but the button and the header follow the
    /// flag immediately.
    pub fn render_scanning(&self, scanning: bool) {
        self.last.borrow_mut().scanning = scanning;
        self.render_header();
        self.render_scan_button();
        self.rebuild();
    }

    /// Say something that just happened, for a few seconds. The one line a row
    /// cannot say for itself - a pairing that failed - comes through here.
    pub fn note(&self, text: &str) {
        self.note.set_text(text);
        self.note.set_visible(true);
        let label = self.note.clone();
        glib::timeout_add_local_once(NOTE_TIME, move || label.set_visible(false));
    }

    /// Hold a row's button while its answer is on the way.
    pub fn set_pending(&self, address: &str, action: DeviceAction) {
        *self.pending.borrow_mut() = Some(Pending {
            address: address.to_string(),
            action,
            until: Instant::now() + patience(action),
        });
        self.rebuild();
    }

    pub fn clear_pending(&self, address: &str) {
        {
            let mut pending = self.pending.borrow_mut();
            if pending
                .as_ref()
                .is_some_and(|waiting| waiting.address == address)
            {
                *pending = None;
            }
        }
        self.rebuild();
    }

    /// Walk the visible rows - what Up and Down mean.
    pub fn step(&self, delta: i32) {
        let visible = self.visible();
        if visible.is_empty() {
            return;
        }
        let current = {
            let selected = self.selected.borrow();
            selected
                .as_ref()
                .and_then(|address| visible.iter().position(|device| device.address == *address))
        };
        let next = match current {
            Some(index) => (index as i32)
                .saturating_add(delta)
                .clamp(0, visible.len() as i32 - 1) as usize,
            None if delta < 0 => visible.len() - 1,
            None => 0,
        };
        *self.selected.borrow_mut() = Some(visible[next].address.clone());
        self.apply_selected();
    }

    /// Run the action of the row the keyboard is on (or the first visible one
    /// when nothing is selected) - what Enter means.
    pub fn activate_selected(&self) {
        let selected = self.selected.borrow().clone();
        let device = {
            let last = self.last.borrow();
            let visible = Self::filter(&last.devices, &last.query);
            selected
                .and_then(|address| visible.into_iter().find(|device| device.address == address))
                .or_else(|| Self::filter(&last.devices, &last.query).into_iter().next())
        };
        let Some(device) = device else {
            return;
        };
        *self.selected.borrow_mut() = Some(device.address.clone());
        if let Some(on_action) = self.on_action.borrow().as_ref() {
            on_action(&device.address, device.action());
        }
    }

    /// The address the keyboard is on, for `status`.
    pub fn selected(&self) -> Option<String> {
        self.selected.borrow().clone()
    }

    /// The rows that pass the filter, in the order they are drawn.
    fn visible(&self) -> Vec<Device> {
        let last = self.last.borrow();
        Self::filter(&last.devices, &last.query)
    }

    fn filter(devices: &[Device], query: &str) -> Vec<Device> {
        let query = query.trim().to_lowercase();
        devices
            .iter()
            .filter(|device| {
                query.is_empty()
                    || device.label().to_lowercase().contains(&query)
                    || device.address.to_lowercase().contains(&query)
            })
            .cloned()
            .collect()
    }

    /// Repaint the header, the scan button and the lists from the last read.
    fn rebuild(&self) {
        self.prune_pending();
        self.render_header();
        self.render_scan_button();
        self.rebuild_rows();
    }

    fn render_header(&self) {
        let last = self.last.borrow();
        let bluetooth = &last.bluetooth;
        let accessible = bluetooth.present && bluetooth.powered;

        self.title.set_text("Bluetooth");

        let summary = if bluetooth.present {
            let visibility = bluetooth.visibility();
            let visibility = if bluetooth.powered {
                format!(" · {visibility}")
            } else {
                String::new()
            };
            format!("{} · {}{}", bluetooth.alias, bluetooth.state(), visibility)
        } else {
            "No bluetooth controller".to_string()
        };
        self.subtitle.set_text(&summary);
        self.subtitle.set_tooltip_text(Some(if bluetooth.present {
            "The controller, the radio and how many devices are connected"
        } else {
            "No BlueZ controller answered - is bluetoothd running?"
        }));

        self.power
            .set_tooltip_text(Some(&format!("Bluetooth: {}", bluetooth.action())));
        set_class(&self.power, "on", bluetooth.powered);
        self.power.set_sensitive(bluetooth.present);

        set_icon(
            &self.visible,
            if bluetooth.discoverable {
                names::EYE
            } else {
                names::EYE_OFF
            },
        );
        self.visible
            .set_tooltip_text(Some(if bluetooth.discoverable {
                "Visible - stop answering pairing requests"
            } else {
                "Hidden - answer pairing requests so new devices can find this one"
            }));
        set_class(&self.visible, "on", bluetooth.discoverable);
        self.visible.set_sensitive(accessible);

        self.scan.set_sensitive(accessible && !last.scanning);
        self.refresh.set_sensitive(bluetooth.present);
        self.entry.set_sensitive(bluetooth.present);
        drop(last);

        let count = self
            .last
            .borrow()
            .devices
            .iter()
            .filter(|d| d.connected)
            .count();
        self.footer.set_text(&if count > 0 {
            format!("{count} connected · ↑ ↓ select · Enter connect · Esc close")
        } else {
            "↑ ↓ select · Enter connect · Esc close".to_string()
        });
    }

    fn render_scan_button(&self) {
        let scanning = self.last.borrow().scanning;
        if scanning {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.add_css_class("btn-row");
            let spinner = gtk::Spinner::new();
            spinner.add_css_class("btn-spinner");
            spinner.start();
            let word = gtk::Label::new(Some("Scanning"));
            word.add_css_class("btn-word");
            row.append(&spinner);
            row.append(&word);
            self.scan.set_child(Some(&row));
        } else {
            self.scan.set_child(Some(&self.scan_idle));
        }
    }

    fn rebuild_rows(&self) {
        self.rows.borrow_mut().clear();
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let last = self.last.borrow();
        let bluetooth = &last.bluetooth;
        let visible = Self::filter(&last.devices, &last.query);
        let has_query = !last.query.trim().is_empty();

        let empty = match (bluetooth.present, bluetooth.powered) {
            (false, _) => Some((
                "No bluetooth controller",
                "BlueZ did not answer. Is bluetoothd running?",
            )),
            (_, false) => Some((
                "Bluetooth is off",
                "Switch the radio on to see paired and nearby devices.",
            )),
            (true, true) if visible.is_empty() && has_query => Some((
                "Nothing matches the filter",
                "Clear the field to see every device again.",
            )),
            (true, true) if visible.is_empty() => {
                Some(("No devices yet", "Press Scan to look for devices nearby."))
            }
            _ => None,
        };
        if let Some((headline, body)) = empty {
            self.hint.set_text(&format!("{headline}\n{body}"));
            self.hint.set_visible(true);
            set_class(&self.hint, "empty", bluetooth.present && bluetooth.powered);
            self.list.set_visible(false);
            self.scroller.set_visible(true);
            return;
        }
        self.hint.set_visible(false);
        self.list.set_visible(true);

        let paired: Vec<Device> = visible
            .iter()
            .filter(|device| device.paired)
            .cloned()
            .collect();
        let nearby: Vec<Device> = visible
            .iter()
            .filter(|device| !device.paired)
            .cloned()
            .collect();

        if !paired.is_empty() || !has_query {
            self.section("Paired", paired.len(), names::LINK);
            if paired.is_empty() {
                self.section_hint("Nothing paired yet - pair a device from Nearby.");
            } else {
                for device in &paired {
                    let row = self.device_row(device);
                    self.rows
                        .borrow_mut()
                        .push((device.address.clone(), row.clone()));
                    self.list.append(&row);
                }
            }
        }
        if !nearby.is_empty() || !has_query {
            self.section("Nearby", nearby.len(), names::SEARCH);
            if nearby.is_empty() {
                self.section_hint(if bluetooth.powered {
                    "Scan to look for devices nearby."
                } else {
                    "Switch bluetooth on to look for devices."
                });
            } else {
                for device in &nearby {
                    let row = self.device_row(device);
                    self.rows
                        .borrow_mut()
                        .push((device.address.clone(), row.clone()));
                    self.list.append(&row);
                }
            }
        }

        // The selected row may have been filtered away; the first visible one
        // takes over, so Enter always means something.
        let selected = self.selected.borrow().clone();
        if selected.is_none()
            || !visible
                .iter()
                .any(|device| Some(&device.address) == selected.as_ref())
        {
            *self.selected.borrow_mut() = visible.first().map(|device| device.address.clone());
        }
        self.apply_selected();
    }

    /// Move the selection class between the rows that are already drawn. Never a
    /// rebuild: this runs from a press on a row, and replacing the row would take
    /// the button under the pointer away before its release could click it.
    fn apply_selected(&self) {
        let selected = self.selected.borrow().clone();
        for (address, row) in self.rows.borrow().iter() {
            set_class(
                row,
                "selected",
                selected.as_deref() == Some(address.as_str()),
            );
        }
    }

    fn section(&self, title: &str, count: usize, icon: &str) {
        let mark = icons::lucide(icon, SECTION_ICON);
        mark.add_css_class("section-icon");
        let name = gtk::Label::new(Some(title));
        name.add_css_class("section");
        name.set_xalign(0.0);
        let number = gtk::Label::new(Some(&count.to_string()));
        number.add_css_class("chain-count");
        number.set_xalign(1.0);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.add_css_class("section-row");
        row.append(&mark);
        row.append(&name);
        row.append(&spacer());
        row.append(&number);
        self.list.append(&row);
    }

    fn section_hint(&self, text: &str) {
        let label = gtk::Label::new(Some(text));
        label.add_css_class("section-hint");
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.set_max_width_chars(54);
        self.list.append(&label);
    }

    /// One device: what it is, how its link is doing, its battery, and the
    /// action that follows from its state - plus a way to forget it.
    fn device_row(&self, device: &Device) -> gtk::Box {
        let selected = self
            .selected
            .borrow()
            .as_ref()
            .is_some_and(|address| *address == device.address);

        // The mark is what makes a connected row read at a glance: a thin accent
        // bar at the row's edge. It is always in the row - only its colour comes
        // and goes - so connected and idle rows keep their badges aligned.
        let mark = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        mark.add_css_class("dev-mark");
        mark.set_valign(gtk::Align::Fill);
        set_class(&mark, "live", device.connected);

        let icon = icons::lucide(device.icon.icon(), DEVICE_ICON);
        icon.add_css_class("dev-icon");
        icon.set_hexpand(true);
        let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        badge.add_css_class("dev-badge");
        badge.set_valign(gtk::Align::Center);
        badge.set_hexpand(false);
        badge.append(&icon);
        set_class(&badge, "live", device.connected);

        let name = gtk::Label::new(Some(device.label()));
        name.add_css_class("dev-name");
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.set_max_width_chars(26);
        let subtitle = gtk::Label::new(Some(&device.subtitle()));
        subtitle.add_css_class("dev-sub");
        subtitle.set_xalign(0.0);
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
        subtitle.set_max_width_chars(34);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        text.append(&name);
        text.append(&subtitle);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 9);
        row.add_css_class("device-row");
        set_class(&row, "selected", selected);
        set_class(&row, "live", device.connected);
        row.append(&mark);
        row.append(&badge);
        row.append(&text);

        // The link quality as bars: filled bars in the dim colour, the rest
        // fainter. The raw dBm moves to the tooltip.
        let signal = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        signal.add_css_class("dev-sig");
        signal.set_valign(gtk::Align::Center);
        if let Some(level) = device.signal() {
            let filled: String = SIGNAL_BARS.chars().take(level as usize).collect();
            let empty: String = SIGNAL_BARS.chars().skip(level as usize).collect();
            let on = gtk::Label::new(Some(&filled));
            on.add_css_class("sig-on");
            let off = gtk::Label::new(Some(&empty));
            off.add_css_class("sig-off");
            signal.append(&on);
            signal.append(&off);
        }
        row.append(&signal);

        // The battery as a pill of its own, so it lines up across the rows and a
        // low one can colour itself without colouring the whole row.
        if let Some(battery) = device.battery {
            let pill = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            pill.add_css_class("dev-batt");
            pill.set_valign(gtk::Align::Center);
            set_class(&pill, "low", battery < 20);
            pill.set_tooltip_text(Some("Battery, as the device reports it"));
            let icon = icons::lucide(device.battery_icon(), BATTERY_ICON);
            icon.add_css_class("batt-icon");
            let value = gtk::Label::new(Some(&format!("{battery}%")));
            value.add_css_class("batt-value");
            pill.append(&icon);
            pill.append(&value);
            row.append(&pill);
        }

        // The buttons sit in a fixed slot at the row's end, so the meters and
        // the batteries line up from row to row whatever each row carries.
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        actions.add_css_class("actions");
        actions.set_hexpand(false);
        actions.set_valign(gtk::Align::Center);
        actions.append(&spacer());

        let action = device.action();
        let waiting = self.pending_for(&device.address);
        let button = gtk::Button::new();
        button.add_css_class("iconbtn");
        button.set_focus_on_click(false);
        if waiting {
            let spinner = gtk::Spinner::new();
            spinner.add_css_class("btn-spinner");
            spinner.start();
            button.set_child(Some(&spinner));
            button.set_sensitive(false);
            button.set_tooltip_text(Some("Talking to the device…"));
        } else {
            button.set_child(Some(&icons::lucide(action.icon(), BUTTON_ICON)));
            button.set_tooltip_text(Some(action.tooltip()));
            set_class(&button, "on", device.connected);
            set_class(&button, "danger", action == DeviceAction::Remove);
            let view = self.strong();
            let address = device.address.clone();
            button.connect_clicked(move |_| {
                if let Some(view) = view.as_ref() {
                    view.run_action(&address, action);
                }
            });
        }
        actions.append(&button);

        // A paired device that is not connected carries the forget button beside
        // its Connect: it is the one action here that cannot be undone from this
        // card, so it is secondary and never the first thing under the pointer.
        if device.paired && !device.connected && !waiting {
            let remove = icon_button(DeviceAction::Remove.icon(), DeviceAction::Remove.tooltip());
            remove.add_css_class("danger");
            let view = self.strong();
            let address = device.address.clone();
            remove.connect_clicked(move |_| {
                if let Some(view) = view.as_ref() {
                    view.run_action(&address, DeviceAction::Remove);
                }
            });
            actions.append(&remove);
        }
        row.append(&actions);

        // The address and the signal are already in the row; a tooltip that
        // repeats them only covers the row it is about (a tooltip appears under
        // the pointer, which is often the card's own first row).
        //
        // A click anywhere on the row selects it, so Enter has an obvious
        // subject. It does not connect anything: the buttons are that job, and
        // replacing the row here would take the button under the pointer away
        // before its release could click it.
        let click = gtk::GestureClick::new();
        click.set_button(1);
        let view = self.strong();
        let address = device.address.clone();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(view) = view.as_ref() {
                *view.selected.borrow_mut() = Some(address.clone());
                view.apply_selected();
            }
        });
        row.add_controller(click);

        row
    }

    fn run_action(self: &Rc<BluetoothView>, address: &str, action: DeviceAction) {
        if let Some(on_action) = self.on_action.borrow().as_ref() {
            on_action(address, action);
        }
    }

    fn pending_for(&self, address: &str) -> bool {
        self.pending
            .borrow()
            .as_ref()
            .is_some_and(|pending| pending.address == address)
    }

    /// Drop the spinner once the device has done what was asked of it, or once
    /// it has been waiting long enough that the answer is clearly not coming.
    fn prune_pending(&self) {
        let mut pending = self.pending.borrow_mut();
        let Some(waiting) = pending.as_ref() else {
            return;
        };
        let device = self
            .last
            .borrow()
            .devices
            .iter()
            .find(|device| device.address == waiting.address)
            .cloned();
        let settled = match (device, waiting.action) {
            (None, _) => true, // the device is not listed any more
            (Some(device), DeviceAction::Connect) => device.connected,
            (Some(device), DeviceAction::Disconnect) => !device.connected,
            (Some(device), DeviceAction::Pair) => device.paired && device.connected,
            // A device the card asked to forget is settled only once the
            // controller's next read stops listing it.
            (Some(_), DeviceAction::Remove) => false,
        };
        if settled || Instant::now() >= waiting.until {
            *pending = None;
        }
    }

    /// How tall the list came out - what `status` prints, because a row that
    /// widened the card is the one layout fault that is invisible in the CSS.
    pub fn measured(&self) -> String {
        format!(
            "card {:?}x{:?}, list {:?}x{:?}, planned list height {}",
            self.root.width(),
            self.root.height(),
            self.scroller.width(),
            self.scroller.height(),
            self.list_height
        )
    }
}

// ---------------------------------------------------------------------------
// Small shared pieces
// ---------------------------------------------------------------------------

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("iconbtn");
    button.set_child(Some(&icons::lucide(icon, BUTTON_ICON)));
    button.set_tooltip_text(Some(tooltip));
    button.set_focus_on_click(false);
    button
}

fn set_icon(button: &gtk::Button, icon: &str) {
    button.set_child(Some(&icons::lucide(icon, BUTTON_ICON)));
}

fn spacer() -> gtk::Box {
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    spacer
}

fn separator() -> gtk::Separator {
    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    separator.add_css_class("sep");
    separator
}

fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, address: &str, paired: bool, connected: bool) -> Device {
        Device {
            address: address.to_string(),
            name: name.to_string(),
            icon: Default::default(),
            connected,
            paired,
            trusted: paired,
            rssi: None,
            battery: None,
        }
    }

    #[test]
    fn the_filter_matches_a_name_or_an_address() {
        let devices = vec![
            device("WH-1000XM4", "AA:BB:CC:DD:EE:FF", true, true),
            device("K380 Keyboard", "11:22:33:44:55:66", true, false),
            device("", "CC:DD:EE:FF:00:11", false, false),
        ];
        assert_eq!(BluetoothView::filter(&devices, "").len(), 3);
        assert_eq!(BluetoothView::filter(&devices, "sony").len(), 0);
        assert_eq!(BluetoothView::filter(&devices, "k380").len(), 1);
        assert_eq!(BluetoothView::filter(&devices, "cc:dd").len(), 2);
        // An unnamed device is still findable by the address it shows.
        assert_eq!(BluetoothView::filter(&devices, "dd:ee:ff:00").len(), 1);
        assert_eq!(BluetoothView::filter(&devices, "dd:ee:ff").len(), 2);
    }

    #[test]
    fn a_pairing_attempt_is_more_patient_than_a_connect() {
        assert!(patience(DeviceAction::Pair) > patience(DeviceAction::Connect));
    }
}

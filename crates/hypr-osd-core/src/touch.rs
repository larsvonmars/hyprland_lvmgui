//! Touchscreens, the tablet they turn a laptop into, and the keyboard that is
//! not there.
//!
//! Every element in this collection was drawn for a pointer, and a pointer is
//! something a fingertip is not: it hovers, it has a right button, it lands on a
//! 12px target without being asked to be careful, and a keyboard is assumed to be
//! a real one. This module is the shared answer to the three questions that come
//! out of that - *is there a finger*, *is the machine a tablet right now*, and
//! *is there a keyboard to type on* - so every element asks the same way instead
//! of growing its own half-answer.
//!
//! # The finger
//!
//! [`present`] says whether the machine has a touchscreen at all (asked once:
//! Hyprland's own device list is the authority, and `HYPR_OSD_TOUCH=1|0` overrides
//! it for testing a session on a machine that does not have one). [`is_touch`]
//! is how a *gesture* finds out what touched it - a finger and a pen have no
//! cursor, so the panel that unfolds from the bar on hover has to open pinned
//! with a way out instead (see the `tap` verb and `Osd::tap_catcher`).
//!
//! [`on_secondary`] is the other half: the context menu that a right click
//! opens has a long press behind it on a touchscreen, and that is the *same*
//! action rather than a second one, so a widget binds it once.
//!
//! # The tablet
//!
//! [`tablet`] is true while the machine is being used as a tablet: the Surface
//! Type Cover's tablet-mode switch says so when it is folded back, and a machine
//! whose cover is *detached* says so by having no keyboard left to type on. The
//! switch reaches us as `switch:on:<name>` / `switch:off:<name>` on Hyprland's
//! event socket (see [`watch_tablet`]); the answer is shared between the
//! elements through `$XDG_RUNTIME_DIR` (see [`crate::state`]), because the bar
//! and the panel it unfolds are different processes and must agree.
//!
//! A machine with no touchscreen is never a tablet. A machine in tablet mode
//! gets `touch` added to its windows' CSS classes, which is what the shared
//! stylesheet and each element's own rules hang their larger targets on.
//!
//! # The keyboard
//!
//! [`Osk`] is the on-screen keyboard for the moments a card asks to be typed
//! into and there is no hardware keyboard to do it. Three shapes of it exist and
//! behave differently:
//!
//! * **squeekboard** is a daemon that shows itself when a client asks for text
//!   input, so there is nothing to start - only something to *have started*
//!   (`osd.lua` does that, and `install.sh` installs it).
//! * **wvkbd** is started and stopped, so the element that wants it owns the
//!   process through [`Osk::attach`].
//! * **a command of the user's own** (`osk = wvkbd-mobintl -H 300` in
//!   `touch.conf`) is treated the same way as wvkbd: start it on focus, stop it
//!   when the focus leaves.
//!
//! All three are optional. A desktop without one works exactly as it did before,
//! and then the cards that are typed into simply take the keyboard they have.

use std::cell::RefCell;
use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;

use crate::config::Config;
use crate::hardware;
use crate::hypripc::{self, Events};
use crate::state;

/// Element name: `~/.config/hypr-osd/touch.conf`.
pub const ELEMENT: &str = "touch";

/// The smallest thing a fingertip can hit reliably, in logical pixels (44 is
/// the platform guidance both Apple and GNOME settled on). Elements use it when
/// they have to set a size in code rather than in CSS.
pub const TARGET: i32 = 44;

/// How long a finger has to rest for a long press, in milliseconds. The
/// stylesheet-side equivalent of this is the `GestureLongPress` default (500ms);
/// a value here is a *factor* on that default, so 1.0 is 500ms.
const DEFAULT_LONG_PRESS_MS: u64 = 500;

/// The height wvkbd is asked for, in logical pixels, when the config does not
/// say. Tall enough for a full row of keys plus the suggestion strip, and short
/// enough that a 912px-tall tablet still shows the card above it.
const DEFAULT_OSK_HEIGHT: i32 = 260;

/// The config file, read once. `touch.conf` is shared by every element, unlike
/// the per-element files: the questions it answers are about the *machine*, not
/// about one card.
pub struct Settings {
    /// Whether the touch handling is wanted at all. A machine with a touchscreen
    /// can still be used with a pointer only; `enabled = false` in the config is
    /// how a user says so.
    pub enabled: bool,
    /// What to run for the on-screen keyboard: `auto` (find one), `none` (never),
    /// or a command line (`wvkbd-mobintl -H 300`).
    pub osk: String,
    /// How tall wvkbd should be, in logical pixels.
    pub osk_height: i32,
    /// How long a long press lasts, in milliseconds.
    pub long_press: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: true,
            osk: "auto".to_string(),
            osk_height: DEFAULT_OSK_HEIGHT,
            long_press: DEFAULT_LONG_PRESS_MS,
        }
    }
}

/// The config, loaded on first use and shared. A missing file is not an error -
/// every value has a default in this module.
pub fn settings() -> &'static Settings {
    static SETTINGS: OnceLock<Settings> = OnceLock::new();
    SETTINGS.get_or_init(|| {
        let config = Config::load(ELEMENT);
        Settings {
            enabled: config.bool("enabled", true),
            osk: config.string("osk", "auto"),
            osk_height: config.i32("osk_height", DEFAULT_OSK_HEIGHT).max(80),
            long_press: config
                .i32("long_press_ms", DEFAULT_LONG_PRESS_MS as i32)
                .clamp(200, 2000) as u64,
        }
    })
}

/// Whether *any* of the touch handling should happen: a touchscreen is present
/// (or forced on) and the config has not switched it off.
pub fn usable() -> bool {
    settings().enabled && present()
}

/// Whether this machine has a touchscreen.
///
/// The answer comes from Hyprland's own device list, which is the only place a
/// Wayland client can see the input devices it is not allowed to open itself.
/// It is asked once: a touchscreen is not something that appears in the middle
/// of a session (a USB one would, but the cost of being wrong there is that the
/// session keeps its pointer-shaped behaviour, not that it breaks).
///
/// `HYPR_OSD_TOUCH=1` (or `0`) overrides the answer, which is how the touch
/// layout can be tried on a machine that has none - and how the test suite can
/// exercise it without a compositor.
pub fn present() -> bool {
    if let Some(forced) = env_flag("HYPR_OSD_TOUCH") {
        return forced;
    }
    static PRESENT: OnceLock<bool> = OnceLock::new();
    *PRESENT.get_or_init(|| {
        hypripc::json("j/devices")
            .and_then(|devices| devices.get("touch").cloned())
            .and_then(|touch| touch.as_array().map(|list| list.is_empty()))
            .map(|empty| !empty)
            .unwrap_or(false)
    })
}

/// `1`/`0`-ish environment variables, for the testing overrides.
fn env_flag(name: &str) -> Option<bool> {
    match std::env::var(name)
        .ok()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "1" | "yes" | "on" | "true" => Some(true),
        "0" | "no" | "off" | "false" => Some(false),
        _ => None,
    }
}

/// Whether a *typing* keyboard is attached.
///
/// The Type Cover is the keyboard this machine has, and while it is attached the
/// tablet is being used as a laptop - so no on-screen keyboard and no
/// touch-sized layout are wanted. Its absence is also the fallback for the
/// tablet-mode switch (see [`tablet`]).
///
/// Hyprland lists every keyboard, including the fixed-function devices every
/// machine has (the power button, the lid, the virtual `video-bus`). None of
/// those can type a password, so they are filtered out; anything left is a real
/// keyboard - the Type Cover, a USB one, a Bluetooth one.
///
/// A [`Duration::ZERO`] cache is deliberately not used: the cover comes and goes
/// during a session, and a cached "still attached" that outlives the detach would
/// leave a tablet without an OSK. The list is one local socket round-trip
/// (`hyprctl -j devices`), so it is asked when it matters, not on every frame;
/// the TTL below is the compromise.
pub fn has_keyboard() -> bool {
    if let Some(forced) = env_flag("HYPR_OSD_KEYBOARD") {
        return forced;
    }
    const TTL: Duration = Duration::from_millis(1000);
    static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    if let Ok(cache) = CACHE.lock() {
        if let Some((at, attached)) = *cache {
            if at.elapsed() < TTL {
                return attached;
            }
        }
    }
    let attached = hypripc::json("j/devices")
        .and_then(|devices| devices.get("keyboards").cloned())
        .and_then(|keyboards| keyboards.as_array().cloned())
        .is_some_and(|keyboards| {
            keyboards.iter().any(|keyboard| {
                keyboard
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(is_typing_keyboard)
            })
        });
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((Instant::now(), attached));
    }
    attached
}

/// Whether a keyboard's name is one that can type, rather than one of the
/// fixed-function devices every machine exposes as a keyboard.
fn is_typing_keyboard(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    // Fixed-function devices: the power/sleep buttons, the lid, the kernel's
    // virtual bus, the media-key face of the Type Cover, and the wireless
    // radio switch. None of them can type a password.
    const FIXED: &[&str] = &[
        "gpio-keys",
        "video-bus",
        "power-button",
        "sleep-button",
        "lid",
        "consumer-control",
        "wireless-radio",
        "wmi",
    ];
    !FIXED
        .iter()
        .any(|fixed| name.starts_with(fixed) || name.contains(fixed))
}

/// Whether the machine is being used as a tablet right now.
///
/// Three answers, in order of authority:
///
/// 1. what the **compositor's switch binds** last wrote down - they run a verb
///    that writes [`crate::state::tablet_mode`] (see `osd.lua`), and nothing in
///    this process can see a switch any other way: Hyprland 0.56 fires switch
///    *binds* but publishes no switch *event*, and `hyprctl -j devices` carries
///    a switch's name but not its state;
/// 2. the same flag as this process last read it, kept in [`switch_state`] so a
///    running element does not have to re-read the file on every question;
/// 3. the keyboard: no typing keyboard attached means a tablet, which is the
///    right answer for a session that started with the cover already off, and
///    the right answer for a machine whose vendor switch reports nothing.
///
/// A machine without a touchscreen is never a tablet, however it is folded.
pub fn tablet() -> bool {
    if !usable() {
        return false;
    }
    if let Ok(state) = switch_state().lock() {
        if let Some(state) = *state {
            return state;
        }
    }
    if let Some(state) = state::read_opt(&state::tablet_mode()) {
        return state;
    }
    !has_keyboard()
}

/// What the tablet-mode switch last told this process. `None` until it speaks.
fn switch_state() -> &'static Mutex<Option<bool>> {
    static SWITCH: Mutex<Option<bool>> = Mutex::new(None);
    &SWITCH
}

/// Record what the switch said, in this process and in the shared flag. Any
/// element may be the one that saw it (the bar's switch bind is what normally
/// runs, but a hand-written bind or a `touch` verb may write the file too).
pub fn set_tablet(tablet: bool) {
    if let Ok(mut state) = switch_state().lock() {
        *state = Some(tablet);
    }
    state::write(&state::tablet_mode(), tablet);
}

/// A live subscription to the machine turning into (and out of) a tablet.
///
/// Keep it alive for as long as the element wants to hear about changes; the
/// poll timer stops when it is dropped.
pub struct TabletWatch {
    /// The compositor's event stream, for the day Hyprland starts publishing
    /// switches on it (today it does not; the parser is harmless either way and
    /// the socket is the same one the elements already hold for other reasons).
    _events: Events,
    /// The shared flag is where the answer really comes from: the switch binds
    /// write it, and this reads it. One small file read per tick, once a second,
    /// only on a machine that has a touchscreen at all.
    poll: Option<glib::SourceId>,
}

impl Drop for TabletWatch {
    fn drop(&mut self) {
        if let Some(poll) = self.poll.take() {
            poll.remove();
        }
    }
}

/// Watch for tablet-mode changes and call `on_change` when the answer changes
/// (including the first time it is known).
///
/// Two sources, one answer. The **shared flag** is authoritative: the two switch
/// binds `osd.lua` installs (`switch:on:` / `switch:off:` on the Type Cover's
/// switch) run a verb that writes it, because that is the only door a switch has
/// into an ordinary program. This watches the file and reports a change as soon
/// as it appears.
///
/// The **event socket** is watched too, for `switch:*` lines and for the two
/// device events: a compositor build that does publish switches (or a machine
/// whose switch the binds do not name) gets picked up without waiting for the
/// next poll, and the keyboard coming and going is the fallback signal on a
/// machine with no switch at all. Neither source is required.
///
/// `on_change` runs only when the answer actually changed - and the current
/// answer is handed over once at the start, so an element that is starting up in
/// tablet mode does not have to ask.
pub fn watch_tablet(on_change: impl Fn(bool) + 'static) -> TabletWatch {
    let last = std::rc::Rc::new(std::cell::Cell::new(tablet()));
    let on_change = std::rc::Rc::new(on_change);

    // The socket half: a compositor that publishes `switch:on:<name>` (with the
    // lid filtered out - closing the lid is not folding the keyboard) and the
    // device list changing under us.
    let handler: std::rc::Rc<dyn Fn(bool)> = on_change.clone();
    let watched = last.clone();
    let _events = Events::start(move |event| {
        let event = event.split(">>").next().unwrap_or(event);
        let now = match tablet_after(event) {
            Some(SwitchEvent::Switch(on)) => {
                set_tablet(on);
                on
            }
            // The keyboard changed, and no switch has spoken: re-derive.
            Some(SwitchEvent::Devices) => {
                if switch_state().lock().is_ok_and(|state| state.is_some()) {
                    return;
                }
                !has_keyboard()
            }
            None => return,
        };
        announce(&watched, &handler, now);
    });

    // The file half: the switch binds' answer. `glib::timeout_add_local` on the
    // main loop, because `on_change` builds widgets.
    let handler: std::rc::Rc<dyn Fn(bool)> = on_change;
    let watched = last;
    let poll = glib::timeout_add_local(Duration::from_millis(1000), move || {
        if let Some(now) = state::read_opt(&state::tablet_mode()) {
            if switch_state().lock().is_ok_and(|state| state.is_none()) {
                set_tablet(now);
            }
            announce(&watched, &handler, now);
        }
        glib::ControlFlow::Continue
    });

    TabletWatch {
        _events,
        poll: Some(poll),
    }
}

/// Tell the element about `now` if it is news, and remember it either way.
fn announce(
    watched: &std::rc::Rc<std::cell::Cell<bool>>,
    handler: &std::rc::Rc<dyn Fn(bool)>,
    now: bool,
) {
    if watched.get() == now {
        return;
    }
    watched.set(now);
    handler(now);
}

/// What one compositor event says about tablet mode, if anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SwitchEvent {
    /// The tablet-mode switch went on or off.
    Switch(bool),
    /// A device was added or removed, so the keyboard answer may have changed.
    Devices,
}

/// Read one event line. The parsing is split out so that the names this
/// compositor really sends can be tested without a compositor.
fn tablet_after(event: &str) -> Option<SwitchEvent> {
    let event = event.trim();
    if let Some(name) = event.strip_prefix("switch:on:") {
        return is_tablet_switch(name).then_some(SwitchEvent::Switch(true));
    }
    if let Some(name) = event.strip_prefix("switch:off:") {
        return is_tablet_switch(name).then_some(SwitchEvent::Switch(false));
    }
    if event.starts_with("deviceadded") || event.starts_with("deviceremoved") {
        return Some(SwitchEvent::Devices);
    }
    None
}

/// Whether a switch's name is the one that means "folded into a tablet": the
/// Surface Type Cover's own ("Microsoft Surface Type Cover Tablet Mode
/// Switch"), and anything else a vendor calls a tablet mode switch. The lid is
/// deliberately not one.
fn is_tablet_switch(name: &str) -> bool {
    name.to_ascii_lowercase().contains("tablet")
}

/// Whether the gesture that just fired came from a finger or a pen rather than
/// a mouse - which is the difference between "open the panel and watch the
/// pointer" and "open the panel and wait for a tap outside".
///
/// A pen counts as a finger for this: it has a cursor while it is in proximity,
/// but the moment it taps it is being *used as a pointer on a tablet*, and a
/// panel that closed itself under the stylus would be a bug. A trackpad does
/// not: its touches are pointer motion, with the hover the touchscreen lacks.
pub fn is_touch(gesture: &impl IsA<gtk::Gesture>) -> bool {
    if let Some(device) = gesture.device() {
        if matches!(
            device.source(),
            gdk::InputSource::Touchscreen | gdk::InputSource::Pen
        ) {
            return true;
        }
    }
    // GDK can also hand a finger over through the seat's *pointer* device, with
    // the event flagged as an emulation. The device name is no help there, so
    // the event itself is asked - which is the only place that flag lives.
    let sequence = gesture.last_updated_sequence();
    gesture
        .last_event(sequence.as_ref())
        .or_else(|| gesture.last_event(None))
        .is_some_and(|event| event.is_pointer_emulated())
}

thread_local! {
    /// The widgets whose primary sequence has just been *consumed by a long
    /// press*, and when it happened.
    ///
    /// A widget can have both a click and a long press bound (the clock opens the
    /// island on a tap and toggles do-not-disturb on a long press, a tray icon
    /// activates on a click and opens its menu on a long press). The two gestures
    /// are independent controllers, so without this the long press would fire its
    /// action *and* the tap its own on the way up - a media pill that skips a
    /// track and pauses playback at once. The press marks its widget here; the
    /// click's release consumes the mark and does nothing. The mark expires on
    /// its own, so a long press whose sequence is cancelled (the finger dragged
    /// away) cannot swallow a later tap.
    ///
    /// A thread-local because it is only ever touched from the main thread,
    /// which is where GTK runs.
    static LONG_PRESSED: RefCell<HashMap<usize, Instant>> = RefCell::new(HashMap::new());
}

/// How long a long press keeps suppressing the click that follows it. Only the
/// gap between the long press firing and the finger lifting has to fit in it.
const LONG_PRESS_MARK: Duration = Duration::from_millis(1500);

/// A widget's identity in [`LONG_PRESSED`]: the GObject pointer, which is stable
/// for the widget's lifetime.
fn widget_key(widget: &impl IsA<gtk::Widget>) -> usize {
    widget.as_ptr() as usize
}

/// Remember that a long press just fired on `widget`, so the click that follows
/// it can be swallowed.
fn mark_long_press(widget: &impl IsA<gtk::Widget>) {
    let key = widget_key(widget);
    LONG_PRESSED.with(|marks| {
        marks.borrow_mut().insert(key, Instant::now());
    });
}

/// Whether a long press just fired on `widget` - and forget it, since one
/// long press may only swallow the one click it belongs to.
fn take_long_press(widget: &impl IsA<gtk::Widget>) -> bool {
    let key = widget_key(widget);
    LONG_PRESSED.with(|marks| {
        let mut marks = marks.borrow_mut();
        let recent = marks
            .get(&key)
            .is_some_and(|at| at.elapsed() < LONG_PRESS_MARK);
        marks.remove(&key);
        recent
    })
}

/// Bind a click and tell the handler whether a finger did it.
///
/// This is the replacement for `connect_clicked` on a widget whose touch
/// behaviour differs from its pointer behaviour - the bar's pills, where a
/// *tap* must open a panel pinned (with a tap catcher as the way out) and a
/// *click* keeps the hover flow that has always been there.
///
/// A `GtkButton`'s own `clicked` signal is not used beside it: the button's
/// internal gesture and this one would both fire for one click. The pressed
/// state (the `:active` styling) still comes from the button itself.
///
/// The action runs on **release**, not on press, because a widget can also have
/// a long press ([`on_secondary`]): deciding on release is what lets the long
/// press take the sequence first (see [`LONG_PRESSED`]) instead of the widget
/// doing both things at once. A release after the finger has been dragged off
/// the widget never happens - GTK cancels the click - so dragging across the bar
/// still does not fire every pill it passes.
pub fn on_click(widget: &impl IsA<gtk::Widget>, action: impl Fn(bool) + 'static) {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    let target = widget.clone();
    click.connect_released(move |gesture, _, _, _| {
        if take_long_press(&target) {
            return;
        }
        action(is_touch(gesture));
    });
    widget.add_controller(click);
}

/// Bind the *secondary* action of a widget - the thing a right click does - so
/// that a touchscreen can reach it too.
///
/// Two gestures, one action:
///
/// * a right click, for a mouse (and a pen's barrel button, where the compositor
///   maps it);
/// * a long press, for a finger. `touch-only` is the point: holding the left
///   mouse button is *not* a right click on a desktop, and a menu that appeared
///   after half a second of holding would be a new behaviour for the pointer.
///   A long press that turns into a scroll is cancelled by GTK, so a finger
///   dragging a list does not open menus as it goes.
///
/// The delay is `long_press_ms` from `touch.conf` (500ms by default, the
/// platform's own).
pub fn on_secondary(widget: &impl IsA<gtk::Widget>, action: impl Fn() + 'static) {
    let action = std::rc::Rc::new(action);

    let right = gtk::GestureClick::new();
    right.set_button(gdk::BUTTON_SECONDARY);
    let call = action.clone();
    right.connect_pressed(move |_, _, _, _| call());
    widget.add_controller(right);

    let long = gtk::GestureLongPress::new();
    long.set_touch_only(true);
    // `delay-factor` is a multiplier on GTK's own 500ms default.
    long.set_delay_factor(settings().long_press as f64 / 500.0);
    let call = action;
    let target = widget.clone();
    long.connect_pressed(move |_, _, _| {
        // Tell a click bound beside this one that the sequence is a long press,
        // so it does not also run its own action when the finger lifts.
        mark_long_press(&target);
        call();
    });
    widget.add_controller(long);
}

// ---------------------------------------------------------------------------
// The on-screen keyboard
// ---------------------------------------------------------------------------

/// The on-screen keyboard, and how it is driven.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Osk {
    /// squeekboard: a daemon that shows and hides itself around a client's text
    /// input; we only nudge it (it is also what `osd.lua` starts at login).
    Squeekboard,
    /// wvkbd, or a command of the user's: started on focus and stopped when the
    /// focus leaves.
    Spawned { program: String, args: Vec<String> },
    /// No on-screen keyboard is installed, or the config said `none`: the cards
    /// still take the keyboard, there is just nothing to type on.
    Missing,
}

impl Osk {
    /// Whether there is an OSK to drive at all.
    pub fn available(&self) -> bool {
        !matches!(self, Osk::Missing)
    }

    /// The command that provides it, for `status` output and for the installer
    /// to name when it offers to install one.
    pub fn program(&self) -> Option<&str> {
        match self {
            Osk::Squeekboard => Some("squeekboard"),
            Osk::Spawned { program, .. } => Some(program),
            Osk::Missing => None,
        }
    }

    /// Ask for the keyboard - but only when a tablet is being typed on, which is
    /// the question [`tablet`] answers. A card that asks while the Type Cover is
    /// attached gets the hardware keyboard it was always going to get.
    pub fn show(&self) {
        if !tablet() || !self.available() {
            return;
        }
        match self {
            Osk::Squeekboard => {
                // The daemon shows itself on a text-input request; this makes
                // sure it is not hidden from a previous focus. The call is
                // harmless when it is already up.
                dbus_visible(true);
            }
            Osk::Spawned { program, args } => spawn(program, args),
            Osk::Missing => {}
        }
    }

    /// Let the keyboard go. Called when the focus leaves the field, or the card
    /// closes.
    pub fn hide(&self) {
        match self {
            Osk::Squeekboard => dbus_visible(false),
            Osk::Spawned { .. } => stop(),
            Osk::Missing => {}
        }
    }

    /// Start the keyboard for a machine that has *become* a tablet, and stop it
    /// again when the cover comes back.
    ///
    /// This is not the same thing as [`Osk::show`]. A *daemon* on-screen
    /// keyboard - squeekboard - shows itself whenever any client focuses a text
    /// field, with no way for us to tell it "not while the Type Cover is
    /// attached": if it were running on a laptop, every visit to the launcher's
    /// search field would be covered by a keyboard nobody needs. So it is not
    /// started at login at all; it is started when the machine is folded into a
    /// tablet and stopped when it is not, and while it is running the cards get
    /// their keyboard without asking (which is what makes [`Osk::attach`]
    /// enough on the daemon kind).
    ///
    /// The element that runs this is the bar, in its tablet handler: it is the
    /// one element that is always up. The other kinds of on-screen keyboard are
    /// started on demand by [`Osk::show`], so this is a no-op for them.
    pub fn start_daemon(&self) {
        if !usable() {
            return;
        }
        match self {
            Osk::Squeekboard => {
                if !hardware::installed("squeekboard") || running("squeekboard") {
                    return;
                }
                hardware::launch("squeekboard", &[]);
            }
            Osk::Spawned { program, args } => spawn(program, args),
            Osk::Missing => {}
        }
    }

    /// Stop the keyboard a tablet no longer needs: the cover is back, so the
    /// daemon that would cover the screen goes away.
    pub fn stop_daemon(&self) {
        match self {
            Osk::Squeekboard => {
                hardware::run("pkill", &["-x", "squeekboard"]);
            }
            Osk::Spawned { .. } => stop(),
            Osk::Missing => {}
        }
    }

    /// Raise the keyboard when `widget` has the focus and drop it when it does
    /// not.
    ///
    /// Wired to the *focus* rather than to a tap on the field, because that is
    /// where the answer is true for every way a card gets to be typed into: a
    /// tap focuses the field, a card that opens with the field focused asks for
    /// it at once, and Escape or a tap elsewhere takes the focus away again.
    pub fn attach(&self, widget: &impl IsA<gtk::Widget>) {
        let osk = self.clone();
        let focus = gtk::EventControllerFocus::new();
        focus.connect_enter(move |_| osk.show());
        let osk = self.clone();
        focus.connect_leave(move |_| osk.hide());
        widget.add_controller(focus);
    }
}

/// The on-screen keyboard this machine has, decided once (see the module
/// comment for the order) and shared by every element that asks.
pub fn osk() -> &'static Osk {
    static OSK: OnceLock<Osk> = OnceLock::new();
    OSK.get_or_init(detect_osk)
}

/// What `touch.conf`'s `osk` and the machine's installed programs say.
fn detect_osk() -> Osk {
    let wanted = settings().osk.trim();
    match wanted.to_ascii_lowercase().as_str() {
        // "no keyboard, please": a card typed into with a pen and a real
        // keyboard that is not there is still better than one that covers the
        // screen.
        "none" | "off" | "false" | "no" => Osk::Missing,
        // The default: take the first one installed, squeekboard first because
        // it is the only one that shows itself (nothing can forget to ask it).
        "auto" | "on" | "true" | "yes" | "" => {
            let installed = ["squeekboard", "wvkbd-mobintl", "wvkbd"]
                .into_iter()
                .find(|program| hardware::installed(program));
            match installed {
                Some("squeekboard") => Osk::Squeekboard,
                Some(program) => Osk::Spawned {
                    program: program.to_string(),
                    args: vec!["-H".to_string(), settings().osk_height.to_string()],
                },
                None => Osk::Missing,
            }
        }
        // A command line of the user's own: `wvkbd-mobintl -H 300`.
        _ => match wanted.split_whitespace().next() {
            Some(program) => Osk::Spawned {
                program: program.to_string(),
                args: wanted
                    .split_whitespace()
                    .skip(1)
                    .map(str::to_string)
                    .collect(),
            },
            None => Osk::Missing,
        },
    }
}

/// The child process an [`Osk::Spawned`] keyboard is, while it is up.
///
/// One per process, because one element asks for one keyboard: two cards open
/// at once would otherwise start two.
fn child() -> &'static Mutex<Option<Child>> {
    static CHILD: Mutex<Option<Child>> = Mutex::new(None);
    &CHILD
}

/// Start the spawned keyboard, or leave the one that is already up alone.
fn spawn(program: &str, args: &[String]) {
    let mut slot = match child().lock() {
        Ok(slot) => slot,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(running) = slot.as_mut() {
        // A keyboard that exited by itself (a crash, a display change) leaves a
        // reaped handle; the next question restarts it.
        if running.try_wait().is_ok_and(|status| status.is_none()) {
            return;
        }
    }
    let spawned = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        Ok(child) => *slot = Some(child),
        Err(error) => eprintln!("hypr-osd: cannot start on-screen keyboard {program}: {error}"),
    }
}

/// Stop the spawned keyboard.
fn stop() {
    let mut slot = match child().lock() {
        Ok(slot) => slot,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(mut running) = slot.take() {
        let _ = running.kill();
        let _ = running.wait();
    }
}

/// Stop the on-screen keyboard if this process started one. Run on the way out:
/// a keyboard whose parent died would stay on screen forever, and a wvkbd that
/// was started by a card is exactly that kind of child.
pub fn shutdown() {
    stop();
}

/// Whether a program is already running (one instance per session is the rule
/// for a daemon keyboard), without failing when `pgrep` is missing.
fn running(program: &str) -> bool {
    std::process::Command::new("pgrep")
        .args(["-x", program])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Ask squeekboard to show or hide itself, over D-Bus.
///
/// The call is what squeekboard publishes for a compositor that wants a manual
/// hand, and it is ignored (quietly) when the daemon is not running - which is
/// the normal state on a machine that never installed one.
fn dbus_visible(visible: bool) {
    let _ = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "sm.puri.OSK0",
            "--object-path",
            "/sm/puri/OSK0",
            "--method",
            "sm.puri.OSK0.SetVisible",
            if visible { "true" } else { "false" },
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Tell the user, once, that a card wants to be typed into and there is nothing
/// to type on. Called by the elements that offer a field instead of failing
/// silently - the same way the volume element says `wpctl` is missing.
pub fn warn_missing(osk: &Osk) -> Option<String> {
    if osk.available() || !tablet() {
        return None;
    }
    Some(
        "no on-screen keyboard is installed (squeekboard, wvkbd-mobintl, wvkbd) - \
         install one to type on a tablet"
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tablet_switch_is_read_out_of_an_event_line() {
        // Exactly what this compositor emits for a Surface Type Cover.
        assert_eq!(
            tablet_after("switch:on:Microsoft Surface Type Cover Tablet Mode Switch"),
            Some(SwitchEvent::Switch(true))
        );
        assert_eq!(
            tablet_after("switch:off:Microsoft Surface Type Cover Tablet Mode Switch"),
            Some(SwitchEvent::Switch(false))
        );
        // The lid is a switch too, and closing it is not folding the keyboard.
        assert_eq!(tablet_after("switch:on: Lid Switch"), None);
        assert_eq!(tablet_after("switch:off:Lid Switch"), None);
        // A switch event carries the line's trailing `>>`; parsing is tolerant.
        assert_eq!(
            tablet_after("switch:on:Microsoft Surface Type Cover Tablet Mode Switch>>"),
            Some(SwitchEvent::Switch(true))
        );
        // Device changes can change the keyboard answer...
        assert_eq!(
            tablet_after("deviceadded>>ipts-045e:099f-touchscreen"),
            Some(SwitchEvent::Devices)
        );
        assert_eq!(
            tablet_after("deviceremoved>>microsoft-surface-type-cover-keyboard"),
            Some(SwitchEvent::Devices)
        );
        // ...but nothing else does.
        assert_eq!(tablet_after("workspace>>2"), None);
        assert_eq!(tablet_after("activewindow>>kitty,~"), None);
        assert_eq!(tablet_after(""), None);
    }

    #[test]
    fn only_a_keyboard_that_can_type_counts_as_one() {
        // The Surface's own keyboard, and the ones a dock or a bag adds.
        assert!(is_typing_keyboard("microsoft-surface-type-cover-keyboard"));
        assert!(is_typing_keyboard("usb-hid-keyboard"));
        assert!(is_typing_keyboard("bluetooth-keyboard-1"));
        // The fixed-function devices every machine reports as keyboards.
        assert!(!is_typing_keyboard("gpio-keys"));
        assert!(!is_typing_keyboard("gpio-keys-1"));
        assert!(!is_typing_keyboard("video-bus"));
        assert!(!is_typing_keyboard("power-button"));
        assert!(!is_typing_keyboard("sleep-button"));
        assert!(!is_typing_keyboard("lid-switch"));
        assert!(!is_typing_keyboard(
            "microsoft-surface-type-cover-consumer-control"
        ));
    }

    #[test]
    fn a_config_line_can_name_the_keyboard_command() {
        // `auto` is the default and finds the first installed program; a command
        // line is taken apart the way a shell would, without the shell.
        let osk = Osk::Spawned {
            program: "wvkbd-mobintl".to_string(),
            args: vec!["-H".to_string(), "300".to_string()],
        };
        assert!(osk.available());
        assert_eq!(osk.program(), Some("wvkbd-mobintl"));
        assert!(!Osk::Missing.available());
        assert_eq!(Osk::Squeekboard.program(), Some("squeekboard"));
    }

    #[test]
    fn the_environment_can_force_the_answers_for_testing() {
        // The override names are part of the contract the test suite and the
        // README rely on; a typo here would look like a broken session.
        assert_eq!(env_flag("HYPR_OSD_TOUCH_DOES_NOT_EXIST"), None);
        std::env::set_var("HYPR_OSD_TOUCH_TEST", "1");
        assert_eq!(env_flag("HYPR_OSD_TOUCH_TEST"), Some(true));
        std::env::set_var("HYPR_OSD_TOUCH_TEST", "off");
        assert_eq!(env_flag("HYPR_OSD_TOUCH_TEST"), Some(false));
        std::env::remove_var("HYPR_OSD_TOUCH_TEST");
    }
}

//! The layer-shell card, and the application shell around it.
//!
//! An element owns one card - or, if it is the bar, one card *per screen*. A card
//! lives in a borderless layer-shell surface on the *overlay* layer, so it draws
//! above fullscreen windows, takes no keyboard focus (the window you were typing
//! in keeps it) and can take pointer input - the volume slider needs that, a
//! plain indicator does not care. Everything outside the card is transparent,
//! which is what makes `osd.lua`'s `ignore_alpha` layer rule able to let clicks
//! through there.
//!
//! The element supplies its content (one widget, or a factory that makes one per
//! output - see [`Content`]) and a command handler; the shell takes care of where
//! the card goes, when it disappears, and how a second invocation of the binary
//! reaches the running instance.
//!
//! A **finger** is not a pointer, and the shell answers for that too: while the
//! machine is a tablet (see [`crate::touch`]) every window wears the `touch` CSS
//! class, so the stylesheet can grow its targets; and a card that was opened by a
//! tap rather than by hovering gets a **tap catcher** ([`Osd::tap_catcher`]) -
//! a dimmed, full-screen layer surface with a hole over the card, which turns a
//! tap anywhere else into "close this". That is the only way a popup can be
//! dismissed on a screen that has neither an Escape key nor a pointer to move
//! away.

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::css;
use crate::monitors;
use crate::touch;

/// Space kept around the card inside the surface, so its box-shadow and glow
/// are not clipped by the surface edge. `Opts::bottom_margin` measures the
/// *visual* gap, so this is subtracted again when the margin is applied.
pub const SHADOW_PAD: i32 = 18;

/// Padding between the card's edge and the element's content. The card recipe
/// in `base.css` deliberately has none (neither has theme.py's `.card`), which
/// is what keeps `Opts::width` the card's real, on-screen width.
pub const CARD_PAD_X: i32 = 16;
pub const CARD_PAD_Y: i32 = 13;

/// Where the card sits on its screen.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Anchored to the bottom edge, `Opts::bottom_margin` of the screen's height
    /// up from it. The OSD cards: they arrive where the bar is, for a moment,
    /// and never cover what you are reading.
    Bottom,
    /// Centred on the screen, for a card you are *looking at* rather than a
    /// notification passing by - the switcher's grid of windows.
    Center,
    /// Stretched over the whole screen, for a card that *is* the screen for as
    /// long as it is up - the workspace overview, which dims everything behind
    /// it and shows the desktop as tiles.
    ///
    /// `Opts::width` does not apply (there is nothing to size: the surface is
    /// the screen) and neither does the shadow margin the other cards keep - a
    /// card with no edge of its own has nothing to keep its shadow away from.
    Fill,
    /// The top bar: anchored to the top edge and stretched across the screen, at
    /// a fixed height. `margin_top` floats it below the top edge and `margin_x`
    /// insets it from the sides, which is what makes it read as a bar rather
    /// than a strip glued to the screen; `exclusive` reserves the height plus
    /// that top margin with the compositor, so windows and other surfaces are
    /// laid out *below* the bar instead of under it.
    ///
    /// Two things differ from every other placement, both deliberate: the
    /// surface goes on the *top* layer (not overlay), so a fullscreen window
    /// covers the bar like it covers everything else, and the content widget is
    /// not inset by `CARD_PAD_*` - a bar is full width and its pills carry the
    /// rhythm themselves.
    ///
    /// And it is *up for good*: a bar is not a notification that arrives and
    /// leaves, so the shell shows it as soon as it has one, and again on every
    /// rebuild (a monitor change) - a bar that disappeared when a screen was
    /// plugged in would be a bug. [`Osd::hide`] still takes it away, which is
    /// what a `hide`/`toggle` verb is for.
    ///
    /// It is also the one placement that is on **every** output rather than one:
    /// the shell builds a surface per monitor (using the element's
    /// [`Content::PerOutput`] factory) and keeps them in step with GDK's monitor
    /// list, so plugging a screen in grows a bar onto it and unplugging one takes
    /// that bar away. `Opts::pin_output` narrows it back down to a single
    /// connector when the config asks for that.
    Bar {
        height: i32,
        margin_top: i32,
        margin_x: i32,
        exclusive: bool,
    },
    /// A content-sized card hanging under the top edge, centred horizontally:
    /// the island popup that unfolds from the bar. `margin_top` is measured from
    /// the top edge of the screen (a layer-shell margin is measured from the
    /// output's own edge), so it is the bar's height plus whatever gap the
    /// popup wants - see the island element's config.
    TopCard { margin_top: i32 },
    /// The same kind of card, but aligned to the *right* edge of the screen:
    /// the system popup, which unfolds under the bar's right-hand end. Its right
    /// edge lines up with the bar's, so the two read as one object - which is
    /// also why `margin_right` is the bar's own `margin_x` rather than a number
    /// of the popup's own.
    TopRight { margin_top: i32, margin_right: i32 },
    /// The mirror of [`Placement::TopRight`]: a card hanging under the bar's
    /// left-hand end, with its left edge lined up with the bar's. The
    /// applications panel unfolds from the button at the bar's left end, which is
    /// the same gesture as the island and the system popup - a panel that comes
    /// out of the pill you pressed - and `margin_left` is the bar's `margin_x`
    /// for the same reason `margin_right` is on the other side.
    TopLeft { margin_top: i32, margin_left: i32 },
}

/// What the card does with the keyboard.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Keyboard {
    /// Never takes it. The window you were typing in keeps typing, which is the
    /// point of an OSD and the default for the whole collection.
    None,
    /// Grabs it while the card is up. Only for a card that *is driven* by keys:
    /// the switcher has to see the Alt release that ends a switch, the overview
    /// has to hear Escape and the arrows, and the launcher has to be typed into -
    /// none of which can be allowed to reach the window underneath, or the card
    /// would be fighting the application for the same keystroke. An OSD that eats
    /// a keystroke is a bug; this is the exception, and it is for cards you opened
    /// on purpose.
    Exclusive,
}

/// Everything an element has to tell the shell about itself.
#[derive(Clone)]
pub struct Opts {
    /// D-Bus application id, e.g. `com.schells2.osd.volume`. It also makes the
    /// binary single-instance: a second invocation forwards its argv to the
    /// running instance rather than opening a second card.
    pub app_id: String,
    /// The complete stylesheet (`css::stylesheet`).
    pub css: String,
    /// Layer-shell namespace. `hypr-osd` for every element in the collection,
    /// so one layer rule in `osd.lua` covers all of them.
    pub namespace: String,
    /// The card's width in pixels. Its height follows the content.
    pub width: i32,
    /// Distance from the bottom edge of the screen as a fraction of the
    /// monitor's height (0.12 = 12 % up from the bottom). Only used by
    /// [`Placement::Bottom`].
    pub bottom_margin: f64,
    /// Where the card sits (see [`Placement`]).
    pub placement: Placement,
    /// Whether the card takes the keyboard (see [`Keyboard`]).
    pub keyboard: Keyboard,
    /// Keep the surface on one output instead of following the focused one.
    ///
    /// A card is *about* what you are doing, so it belongs on the screen you are
    /// doing it on - hence the default. A bar is not: it is furniture, and one
    /// that jumped to whichever monitor you last clicked would be broken. So a
    /// bar is on **every** output (one surface each, see [`Content::PerOutput`]),
    /// and this field is how it is told to be on just one - `output = eDP-1` in
    /// the config.
    pub pin_output: Option<String>,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            app_id: String::new(),
            css: String::new(),
            namespace: "hypr-osd".to_string(),
            width: 340,
            bottom_margin: 0.12,
            placement: Placement::Bottom,
            keyboard: Keyboard::None,
            pin_output: None,
        }
    }
}

/// A callback an element wants to run once, on the way out (see
/// [`Osd::on_shutdown`]).
type ShutdownCallback = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;

/// Handlers for keys that reach the card (see [`Osd::on_keys`]).
///
/// Only the press handler answers with a propagation verdict: GTK's
/// `key-released` is a void signal, so a release cannot be swallowed - which is
/// fine, because by then the key is already on its way out.
type KeyPressHandler = Rc<dyn Fn(&gdk::Key, gdk::ModifierType) -> glib::Propagation>;
type KeyReleaseHandler = Rc<dyn Fn(&gdk::Key)>;

/// How a bar builds one output's content: the shell calls it once per output,
/// with the connector it is building for (see [`Content::PerOutput`]).
type OutputContent = Rc<dyn Fn(&str) -> gtk::Widget>;

/// What an element wants to run when the machine becomes (or stops being) a
/// tablet (see [`Osd::on_tablet_change`]).
type TabletHandler = Rc<dyn Fn(bool)>;

struct Surface {
    window: gtk::Window,
    card: gtk::Box,
    /// Connector of the monitor this surface sits on, so a command can tell
    /// whether it has to be rebuilt for a different output.
    monitor: Option<String>,
}

/// The card's rectangle on its own monitor, in logical pixels - the shape of the
/// hole the tap catcher leaves open.
///
/// The element knows this rectangle, not the shell: it is the card it built, at
/// the size it asked for, at the placement its config names. It is measured from
/// the monitor's top-left corner (so the bar's `margin_x`/`margin_top` and the
/// panel's own gap are already in it), because that is the coordinate space a
/// layer surface on that monitor is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hole {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Hole {
    /// The same rectangle grown by `pad` on every side - what a fingertip needs:
    /// a card with a shadow should not be dimmed by its own shadow.
    fn grown(self, pad: i32) -> Hole {
        Hole {
            x: self.x - pad,
            y: self.y - pad,
            width: self.width + pad * 2,
            height: self.height + pad * 2,
        }
    }
}

/// The dimmed, full-screen surface that catches taps outside a card.
///
/// Four strips around the hole rather than one surface with a transparent window
/// in it, for two reasons: a GTK widget tree can say "here is a box" but not
/// "here is a hole", and `ignore_alpha` only makes a *transparent* pixel
/// click-through - so the strips have to be the parts that swallow the tap, not
/// the hole that must not. Each strip carries its own click gesture, which is
/// also what makes it work no matter which side of the card the compositor
/// stacks this surface on.
struct Catcher {
    window: gtk::Window,
    /// The four strips, re-laid out every time the card moves.
    strips: Vec<gtk::Box>,
}

pub struct Osd {
    opts: Opts,
    app: gtk::Application,
    /// Keeps the application alive between key presses: the card is hidden most
    /// of the time, and GApplication quits when nothing holds it.
    _hold: gio::ApplicationHoldGuard,
    me: Weak<Osd>,
    /// The card's surface: at most one, and the shell moves it to whichever
    /// output the card belongs on. Empty for a bar, which has no single surface.
    surface: RefCell<Option<Surface>>,
    /// A bar's surfaces: one per output, because a bar is furniture and every
    /// screen gets its own. Empty for every other placement.
    bars: RefCell<Vec<Surface>>,
    /// How a bar builds one output's content (see [`Content::PerOutput`]).
    bar_factory: RefCell<Option<OutputContent>>,
    /// Whether GDK's monitor list has been hooked for changes, so screens
    /// plugged in later get a bar (see [`Osd::watch_outputs`]).
    watching_outputs: Cell<bool>,
    /// The element's content widget, i.e. the card's child.
    content: RefCell<Option<gtk::Widget>>,
    hide: RefCell<Option<glib::SourceId>>,
    stay_open: RefCell<Option<Box<dyn Fn() -> bool>>>,
    /// Key handlers, installed on every surface the shell builds (see
    /// [`Osd::on_keys`]).
    keys: RefCell<Option<(KeyPressHandler, KeyReleaseHandler)>>,
    /// The connector the surface is kept on, when the element knows better than
    /// "whatever is focused" - starts as [`Opts::pin_output`] and can be changed
    /// afterwards with [`Osd::pin_output`].
    pin: RefCell<Option<String>>,
    /// Where the card sits, and how wide it is. Both start as [`Opts`] says and
    /// can change at runtime ([`Osd::set_placement`], [`Osd::set_width`]) - a
    /// tablet grows the bar, and a bar whose height was frozen at start-up would
    /// be a target too small to hit for the rest of the session.
    placement: RefCell<Placement>,
    width: Cell<i32>,
    /// The dimmed tap catcher around a card opened by a finger (see
    /// [`Osd::tap_catcher`]).
    catcher: RefCell<Option<Catcher>>,
    /// What a tap outside the card runs - the element's own "close it".
    tap_outside: RefCell<Option<Rc<dyn Fn()>>>,
    /// The hole the catcher was last laid out for, so an element that re-fits it
    /// on every tick (the card can grow while it is up) does not rebuild the
    /// bands when nothing has moved. `None` whenever the catcher is hidden.
    catcher_hole: RefCell<Option<Hole>>,
    /// The element's handlers for a machine becoming (or stopping being) a
    /// tablet (see [`Osd::on_tablet_change`]).
    tablet_handlers: RefCell<Vec<TabletHandler>>,
    /// The subscription that keeps the tablet answer live, while the element is
    /// running.
    tablet_watch: RefCell<Option<crate::touch::TabletWatch>>,
    /// Whether the live watch has been started (see [`Osd::begin_touch`]).
    watching_tablet: Cell<bool>,
}

impl Osd {
    fn new(app: &gtk::Application, opts: Opts) -> Rc<Self> {
        let hold = app.hold();
        let pin = RefCell::new(opts.pin_output.clone());
        let placement = opts.placement;
        let width = opts.width;
        Rc::new_cyclic(|me| Osd {
            opts,
            app: app.clone(),
            _hold: hold,
            me: me.clone(),
            surface: RefCell::new(None),
            bars: RefCell::new(Vec::new()),
            bar_factory: RefCell::new(None),
            watching_outputs: Cell::new(false),
            content: RefCell::new(None),
            hide: RefCell::new(None),
            stay_open: RefCell::new(None),
            keys: RefCell::new(None),
            pin,
            placement: RefCell::new(placement),
            width: Cell::new(width),
            catcher: RefCell::new(None),
            tap_outside: RefCell::new(None),
            catcher_hole: RefCell::new(None),
            tablet_handlers: RefCell::new(Vec::new()),
            tablet_watch: RefCell::new(None),
            watching_tablet: Cell::new(false),
        })
    }

    /// The connector this element's surface is on, as far as it knows: the one
    /// the config pinned it to, the focused output it started on, or `None`
    /// before there is a surface at all.
    ///
    /// A bar has a surface per output and so has no single answer - its own
    /// views each know their connector (they are built with it, see
    /// [`Content::PerOutput`]), and the first of them answers here so that a
    /// caller with no better information still gets something true.
    pub fn monitor(&self) -> Option<String> {
        if let Some(surface) = self.surface.borrow().as_ref() {
            return surface.monitor.clone();
        }
        self.bars
            .borrow()
            .first()
            .and_then(|surface| surface.monitor.clone())
    }

    /// The connectors this element currently has a surface on: every output for
    /// a bar, the one screen a card is on otherwise.
    ///
    /// What an element needs to tell which of its own per-output widgets are
    /// still on screen - a bar's views are keyed by connector, and a monitor
    /// that was unplugged is not in this list any more.
    pub fn outputs(&self) -> Vec<String> {
        if let Some(surface) = self.surface.borrow().as_ref() {
            return surface.monitor.iter().cloned().collect();
        }
        self.bars
            .borrow()
            .iter()
            .filter_map(|surface| surface.monitor.clone())
            .collect()
    }

    /// Keep the surface on this connector from now on, rebuilding it if it is
    /// somewhere else.
    ///
    /// [`Opts::pin_output`] is decided when the element starts, which is right
    /// for a bar and wrong for the popup that unfolds from one: *where the bar
    /// is* is something only the bar knows, and it says so when it asks for the
    /// panel (the `open` verb both popups take). A panel that landed on the
    /// other monitor would hang in mid-air, because every coordinate it has is
    /// derived from the bar's.
    ///
    /// For a bar this means something different, and deliberately: the bar is on
    /// every output, and pinning it is how the config says "only on this one".
    pub fn pin_output(&self, connector: Option<&str>) {
        let wanted = connector.map(str::to_owned);
        if *self.pin.borrow() == wanted && self.monitor() == wanted {
            return;
        }
        *self.pin.borrow_mut() = wanted;
        if matches!(self.placement(), Placement::Bar { .. }) {
            self.sync_outputs();
        } else {
            self.ensure_surface();
        }
    }

    /// Where the card sits right now - [`Opts::placement`] unless an element
    /// changed it at runtime.
    pub fn placement(&self) -> Placement {
        *self.placement.borrow()
    }

    /// The card's width right now - [`Opts::width`] unless an element changed it.
    pub fn width(&self) -> i32 {
        self.width.get()
    }

    /// Move (or resize) the card, rebuilding the surfaces so the change is on
    /// screen.
    ///
    /// This exists for a change no stylesheet can express: the bar's height, a
    /// card's width and where a card hangs are numbers the shell gives
    /// layer-shell when it makes the surface, not CSS lengths. A machine folding
    /// into a tablet grows the bar - and a bar whose height had been frozen at
    /// start-up would be a row of 30-pixel targets for the rest of the session -
    /// so the element asks for the new numbers and the shell makes the surfaces
    /// again.
    ///
    /// The element's widgets survive it: a card re-parents its same content
    /// widget, and a bar asks its factory for each output's content again, which
    /// is the same path a monitor change takes.
    pub fn set_placement(&self, placement: Placement) {
        *self.placement.borrow_mut() = placement;
        self.rebuild();
    }

    /// Resize the card without moving it (see [`Osd::set_placement`]).
    pub fn set_width(&self, width: i32) {
        self.width.set(width);
        self.rebuild();
    }

    /// Run `callback` when the application shuts down.
    ///
    /// An element that started a child process (a follower, see [`crate::follow`])
    /// must stop it here: children are not killed by their parent dying, and a
    /// daemon that is stopped and started again would otherwise leave one behind
    /// every time. `run` turns SIGTERM/SIGINT into a clean shutdown for exactly
    /// this reason.
    pub fn on_shutdown(&self, callback: impl FnOnce() + 'static) {
        let pending: ShutdownCallback = Rc::new(RefCell::new(Some(Box::new(callback))));
        self.app.connect_shutdown(move |_| {
            if let Some(callback) = pending.borrow_mut().take() {
                callback();
            }
        });
    }

    /// Handle key presses and releases that reach the surface.
    ///
    /// Installed on *every* surface the shell builds, not just the first one:
    /// moving the card to another monitor rebuilds it, and a keyboard-driven
    /// element that quietly lost its keys after a monitor change would be a bug
    /// nobody could explain.
    ///
    /// The controller runs in the capture phase, so a handler sees a key before
    /// GTK's own focus handling does - which is what lets an element decide that
    /// Tab means "next tile" rather than "next widget". Answering
    /// `glib::Propagation::Stop` swallows the press; `Proceed` lets GTK have it.
    /// Releases are informational: GTK gives no verdict for them.
    pub fn on_keys<F, G>(&self, on_press: F, on_release: G)
    where
        F: Fn(&gdk::Key, gdk::ModifierType) -> glib::Propagation + 'static,
        G: Fn(&gdk::Key) + 'static,
    {
        let press: KeyPressHandler = Rc::new(on_press);
        let release: KeyReleaseHandler = Rc::new(on_release);
        *self.keys.borrow_mut() = Some((press.clone(), release.clone()));
        if let Some(surface) = self.surface.borrow().as_ref() {
            attach_keys(&surface.window, &press, &release);
        }
    }

    // -----------------------------------------------------------------
    // Touch: the tablet, and the tap that dismisses a panel
    // -----------------------------------------------------------------

    /// Watch the machine being folded into (and out of) a tablet, and run
    /// `handler` when the answer changes.
    ///
    /// The shell already does the two things every element wants on its own: it
    /// adds or removes the `touch` CSS class on every surface it owns, and it
    /// keeps the shared tablet flag up to date (see [`crate::touch::tablet`]).
    /// This is for the elements that have a size or a layout of their own to
    /// change with it - the bar, whose height is a number rather than a CSS
    /// length, is the one that needs it.
    ///
    /// The live watch starts with [`Osd::begin_touch`], which `run` calls; an
    /// element never has to.
    pub fn on_tablet_change(&self, handler: impl Fn(bool) + 'static) {
        self.tablet_handlers.borrow_mut().push(Rc::new(handler));
    }

    /// Start reacting to the machine becoming a tablet: the `touch` class, the
    /// shared flag, and [`Osd::on_tablet_change`]'s handlers.
    ///
    /// Called once by `run`, after the element has built its widgets. On a
    /// machine with no touchscreen this does nothing at all - not even open the
    /// compositor's event socket.
    pub fn begin_touch(&self) {
        if self.watching_tablet.replace(true) || !touch::usable() {
            return;
        }
        let me = self.me.clone();
        let watch = touch::watch_tablet(move |tablet| {
            if let Some(osd) = me.upgrade() {
                osd.set_tablet(tablet);
            }
        });
        *self.tablet_watch.borrow_mut() = Some(watch);
        // Hand the element the answer it is starting with, so a bar that comes
        // up on an already-folded machine starts its keyboard instead of waiting
        // for the next fold.
        self.set_tablet(touch::tablet());
    }

    /// Put the `touch` class where it belongs after the tablet answer changed.
    fn set_tablet(&self, tablet: bool) {
        {
            let single = self.surface.borrow();
            let bars = self.bars.borrow();
            for surface in single.iter().chain(bars.iter()) {
                apply_touch_class(&surface.window, tablet);
            }
            if let Some(catcher) = self.catcher.borrow().as_ref() {
                apply_touch_class(&catcher.window, tablet);
            }
        }
        // Handlers run with no borrow of the surfaces held: a handler may
        // rebuild them (the bar does, because its height is not a CSS length).
        let handlers = self.tablet_handlers.borrow().clone();
        for handler in handlers {
            handler(tablet);
        }
    }

    /// Rebuild every surface this element owns, from the same widgets.
    ///
    /// For a change that cannot be expressed in CSS: a placement's own numbers
    /// (the bar's height, a card's width) are set when the surface is made, so a
    /// session that changes how big the shell should be has to make the surface
    /// again. A card's widgets come back with it - the shell re-parents the same
    /// widget rather than asking the element to build a new one - and a bar asks
    /// its factory for each output's content again, exactly as a monitor change
    /// does.
    pub fn rebuild(&self) {
        if matches!(self.placement(), Placement::Bar { .. }) {
            let mut bars = self.bars.borrow_mut();
            for bar in bars.drain(..) {
                bar.window.destroy();
            }
            drop(bars);
            self.sync_outputs();
            return;
        }
        let content = self.detach_content();
        if let Some(surface) = self.surface.borrow_mut().take() {
            surface.window.destroy();
        }
        self.ensure_surface();
        if let Some(content) = content {
            self.attach_content(&content);
        }
    }

    /// Catch taps outside `hole` and run `on_outside` when one lands there; `None`
    /// takes the catcher away.
    ///
    /// This is the way out for a card that was opened by a *finger*. A hover popup
    /// closes when the pointer leaves it, which is a thing no touchscreen can do:
    /// the finger lifts, the pointer is wherever it last touched, and the panel
    /// would either stay up forever or close the moment it opened. So the element
    /// asks for the catcher while the panel is up, and the catcher - a dimmed,
    /// full-screen layer surface on the overlay layer with a hole over the card -
    /// turns a tap anywhere else into the element's own "close it".
    ///
    /// `hole` is the card's rectangle on its monitor, in logical pixels, measured
    /// from the monitor's top-left corner. The shadow the shell keeps around a
    /// card is added here, so pass the card *as drawn*.
    ///
    /// The catcher never takes the keyboard: a card that wants to be typed into
    /// keeps it, and the on-screen keyboard stays available above the dim.
    pub fn tap_catcher(&self, hole: Option<Hole>, on_outside: impl Fn() + 'static) {
        *self.tap_outside.borrow_mut() = Some(Rc::new(on_outside));
        if hole.is_none() {
            *self.catcher_hole.borrow_mut() = None;
            if let Some(catcher) = self.catcher.borrow().as_ref() {
                catcher.window.set_visible(false);
            }
            return;
        }
        self.ensure_catcher();
        let hole = hole.expect("checked above");
        // The card can be re-fitted on every tick; only the ticks that moved it
        // are worth laying the bands out again.
        let visible = self
            .catcher
            .borrow()
            .as_ref()
            .is_some_and(|catcher| catcher.window.is_visible());
        if visible && *self.catcher_hole.borrow() == Some(hole) {
            return;
        }
        let geometry = self.monitor_size();
        {
            let catcher = self.catcher.borrow();
            let Some(catcher) = catcher.as_ref() else {
                return;
            };
            place_strips(catcher, hole, geometry);
            catcher.window.set_visible(true);
        }
        *self.catcher_hole.borrow_mut() = Some(hole);
    }

    /// Build the catcher surface, once.
    fn ensure_catcher(&self) {
        if self.catcher.borrow().is_some() {
            return;
        }
        let window = gtk::Window::new();
        window.add_css_class("osd");
        apply_touch_class(&window, touch::tablet());
        window.init_layer_shell();
        window.set_namespace(Some(self.opts.namespace.as_str()));
        window.set_layer(Layer::Overlay);
        // It catches a tap, never a keystroke: the card under it may be waiting
        // for typing (a launcher's filter) or for nothing at all.
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_exclusive_zone(-1);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
        let monitor = self
            .surface
            .borrow()
            .as_ref()
            .and_then(|surface| surface.window.monitor())
            .or_else(monitors::first);
        if let Some(monitor) = &monitor {
            window.set_monitor(Some(monitor));
        }
        let fixed = gtk::Fixed::new();
        window.set_child(Some(&fixed));

        let mut strips = Vec::with_capacity(4);
        for _ in 0..4 {
            let strip = gtk::Box::new(gtk::Orientation::Vertical, 0);
            strip.add_css_class("tap-catcher");
            // The four strips are laid out around the hole; a hidden one is a
            // side the card reaches the screen edge on.
            strip.set_visible(false);
            fixed.put(&strip, 0.0, 0.0);
            let me = self.me.clone();
            let click = gtk::GestureClick::new();
            click.connect_pressed(move |_, _, _, _| {
                if let Some(osd) = me.upgrade() {
                    let callback = osd.tap_outside.borrow().clone();
                    if let Some(callback) = callback {
                        callback();
                    }
                }
            });
            strip.add_controller(click);
            strips.push(strip);
        }
        *self.catcher.borrow_mut() = Some(Catcher { window, strips });
    }

    /// The logical size of the monitor the card is on - the space the catcher's
    /// strips are laid out in.
    fn monitor_size(&self) -> Option<(i32, i32)> {
        let monitor = self
            .surface
            .borrow()
            .as_ref()
            .and_then(|surface| surface.window.monitor())
            .or_else(monitors::first)?;
        let geometry = monitor.geometry();
        Some((geometry.width(), geometry.height()))
    }

    /// Put the element's content into the card, on the focused monitor.
    ///
    /// Called by `run` for a [`Content::Single`] element, right after the
    /// `build` closure has made its widgets - GTK does not exist before then.
    /// Everything that later moves the surface (a config change,
    /// [`Osd::pin_output`]) goes through [`Osd::ensure_surface`], which
    /// re-attaches these same widgets to the new card rather than asking the
    /// element to build them again.
    pub fn set_content(&self, widget: &impl IsA<gtk::Widget>) {
        let content = widget.clone().upcast::<gtk::Widget>();
        self.ensure_surface();
        self.attach_content(&content);
    }

    /// Put the element's content into the current card and remember it.
    fn attach_content(&self, content: &gtk::Widget) {
        let card = self
            .surface
            .borrow()
            .as_ref()
            .map(|surface| surface.card.clone());
        let Some(card) = card else {
            return;
        };
        put_content(&card, content, &self.placement());
        *self.content.borrow_mut() = Some(content.clone());
    }

    /// Put the element's content on **every** output, one surface each.
    ///
    /// The bar's half of [`Content`], called by `run` for a
    /// [`Content::PerOutput`] element, and again from [`Osd::sync_outputs`]
    /// whenever the set of screens changes. `factory` is the element's own: the
    /// shell asks it for one output's widgets at a time, and never calls it
    /// while the element could be looking back in here (see [`Osd::sync_outputs`]).
    fn set_output_content(&self, factory: OutputContent) {
        *self.bar_factory.borrow_mut() = Some(factory);
        self.watch_outputs();
        self.sync_outputs();
    }

    /// Re-sync when screens come and go.
    ///
    /// GDK's monitor list is the authority on where a layer-shell surface *can*
    /// go: it is the same `wl_output` set the compositor advertises, so a screen
    /// being plugged, unplugged or re-configured shows up here as a change to
    /// the list. (Hyprland reports the same thing a moment later as
    /// `monitoradded`/`monitorremoved`; an element that watches those events
    /// calls [`Osd::sync_outputs`] as the safety net.)
    fn watch_outputs(&self) {
        // Once: the surfaces are reconciled on every change, not re-hooked.
        if self.watching_outputs.replace(true) {
            return;
        }
        let Some(display) = gdk::Display::default() else {
            return;
        };
        let me = self.me.clone();
        display.monitors().connect_items_changed(move |_, _, _, _| {
            if let Some(osd) = me.upgrade() {
                osd.sync_outputs();
            }
        });
    }

    /// Make sure there is exactly one bar on every output it belongs on.
    ///
    /// *Reconciled* rather than rebuilt: a screen that is still there keeps its
    /// surface and the widgets in it, so a monitor being plugged in does not
    /// cost the other bars their hover state, their tray or their measurements.
    /// That is also what makes calling this on a tick cheap - the usual answer is
    /// "nothing changed".
    ///
    /// Does nothing for any placement but [`Placement::Bar`] (a card has one
    /// surface and [`Osd::ensure_surface`] owns it), and before the element has
    /// handed its content over.
    pub fn sync_outputs(&self) {
        if !matches!(self.placement(), Placement::Bar { .. }) {
            return;
        }
        let Some(factory) = self.bar_factory.borrow().clone() else {
            // The element's widgets do not exist yet; `run` calls this again the
            // moment they do.
            return;
        };

        // Every output, or only the connector the config names: a bar on one
        // screen only is what `output = eDP-1` asks for.
        let pinned = self.pin.borrow().clone();
        let available: Vec<String> = monitors::all()
            .into_iter()
            .filter_map(|monitor| Some(monitor.connector()?.to_string()))
            .collect();
        let wanted = wanted_outputs(&available, pinned.as_deref());

        // 1. Let go of the screens that are gone. Leaving a surface behind would
        //    keep a strip reserved on an output that no longer exists.
        {
            let mut bars = self.bars.borrow_mut();
            bars.retain(|bar| {
                let kept = bar
                    .monitor
                    .as_deref()
                    .is_some_and(|connector| wanted.iter().any(|keep| keep == connector));
                if !kept {
                    bar.window.destroy();
                }
                kept
            });
        }

        // 2. A screen that stayed keeps its bar, but the `GdkMonitor` under it
        //    can be a *new object* (a mode or scale change makes GDK build one),
        //    so the surface is pointed at the current one either way.
        {
            let bars = self.bars.borrow();
            for connector in &wanted {
                if let Some(bar) = bars
                    .iter()
                    .find(|bar| bar.monitor.as_deref() == Some(connector.as_str()))
                {
                    if let Some(monitor) = monitors::find(connector) {
                        bar.window.set_monitor(Some(&monitor));
                    }
                }
            }
        }

        // 3. The screens that are new get their bar. The element's factory runs
        //    *outside* the borrow of the collection it is feeding: it belongs to
        //    the element, which is free to do anything with it except call back
        //    in here.
        let additions: Vec<String> = {
            let bars = self.bars.borrow();
            wanted
                .into_iter()
                .filter(|connector| {
                    !bars
                        .iter()
                        .any(|bar| bar.monitor.as_deref() == Some(connector.as_str()))
                })
                .collect()
        };
        let built = additions
            .into_iter()
            .map(|connector| {
                let monitor = monitors::find(&connector).or_else(monitors::first);
                let surface = self.make_surface(monitor, Some(connector.clone()));
                let content = factory(&connector);
                (surface, content)
            })
            .collect::<Vec<_>>();
        let mut bars = self.bars.borrow_mut();
        for (surface, content) in built {
            put_content(&surface.card, &content, &self.placement());
            bars.push(surface);
        }
    }

    /// Show the card and hide it again after `duration`, unless
    /// [`Osd::set_stay_open_when`] says otherwise by then.
    pub fn reveal(&self, duration: Duration) {
        self.retarget();
        self.set_every_surface_visible(true);
        self.arm_hide(duration);
    }

    /// Hide immediately (and forget any pending timer).
    pub fn hide(&self) {
        if let Some(source) = self.hide.borrow_mut().take() {
            source.remove();
        }
        // A card cannot be gone while its tap catcher still dims the screen, so
        // closing the card is also what takes the catcher away. Elements do not
        // have to remember to do both.
        *self.catcher_hole.borrow_mut() = None;
        if let Some(catcher) = self.catcher.borrow().as_ref() {
            catcher.window.set_visible(false);
        }
        self.set_every_surface_visible(false);
    }

    /// Show the card and leave it up until something hides it.
    ///
    /// For a card that is a *menu* rather than a notification - the session card
    /// stays until an action is picked or its key toggles it away - or for an
    /// element whose config asks for that (`duration_ms = 0`).
    pub fn show(&self) {
        if let Some(source) = self.hide.borrow_mut().take() {
            source.remove();
        }
        self.retarget();
        self.set_every_surface_visible(true);
    }

    /// Whether the card is on screen right now.
    ///
    /// The shell owns the auto-hide timer, so only it knows: a keybinding that
    /// toggles the card away needs to ask. For a bar, "on screen" means *any* of
    /// its surfaces is - it is one element, and a `hide`/`show` verb speaks for
    /// all of them.
    pub fn is_visible(&self) -> bool {
        if let Some(surface) = self.surface.borrow().as_ref() {
            return surface.window.is_visible();
        }
        self.bars
            .borrow()
            .iter()
            .any(|surface| surface.window.is_visible())
    }

    /// Show or hide every surface this element owns - the one card, or all of a
    /// bar's.
    fn set_every_surface_visible(&self, visible: bool) {
        if let Some(surface) = self.surface.borrow().as_ref() {
            surface.window.set_visible(visible);
        }
        for surface in self.bars.borrow().iter() {
            surface.window.set_visible(visible);
        }
    }

    /// Keep the card up while the pointer rests on `widget` - or, on a
    /// touchscreen, while a finger is *down* on it.
    ///
    /// A card with buttons must not disappear while you are aiming at one - the
    /// auto-hide would otherwise be a race against the pointer.
    ///
    /// A finger is not a pointer here, and treating it as one is the bug this
    /// method exists to avoid in two directions. GDK turns a tap into an
    /// emulated pointer that *enters* the widget and then never leaves it (the
    /// cursor stays where the finger lifted), so a hover hold would keep the card
    /// up for the whole session after one tap. A press, on the other hand, is
    /// exactly as long as the finger is: it begins on the way down and ends on
    /// the way up. So the hold is the pointer's hover *while the machine is a
    /// laptop*, and the finger's press when it is a tablet (checked on every
    /// tick, because a session can be folded mid-card).
    pub fn stay_open_while_hovered(&self, widget: &impl IsA<gtk::Widget>) {
        let hovered = Rc::new(Cell::new(false));
        let motion = gtk::EventControllerMotion::new();
        {
            let hovered = hovered.clone();
            motion.connect_enter(move |_, _, _| hovered.set(true));
        }
        {
            let hovered = hovered.clone();
            motion.connect_leave(move |_| hovered.set(false));
        }
        widget.add_controller(motion);

        // Every button, and therefore a finger's emulated button 1: the count is
        // what keeps the card up from the moment the finger lands until it
        // lifts, including a drag that started on the card.
        let pressed = Rc::new(Cell::new(0_i32));
        let click = gtk::GestureClick::new();
        click.set_button(0);
        // Capture, so a press that a child button claims is still seen here: a
        // card must stay up while the finger is on *any* part of it, not only
        // on the parts nobody handles.
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let pressed = pressed.clone();
            click.connect_pressed(move |_, _, _, _| pressed.set(pressed.get() + 1));
        }
        {
            let pressed = pressed.clone();
            click.connect_released(move |_, _, _, _| pressed.set((pressed.get() - 1).max(0)));
        }
        {
            let pressed = pressed.clone();
            click.connect_stopped(move |_| pressed.set(0));
        }
        widget.add_controller(click);

        self.set_stay_open_when(move || pressed.get() > 0 || (hovered.get() && !touch::tablet()));
    }

    /// Keep the card up while `predicate` returns true, instead of hiding it
    /// after the element's duration. An element that has more than one reason to
    /// hold the card open combines them in one closure (the shell keeps a single
    /// predicate).
    pub fn set_stay_open_when(&self, predicate: impl Fn() -> bool + 'static) {
        *self.stay_open.borrow_mut() = Some(Box::new(predicate));
    }

    /// Make sure the surface exists and sits on the right monitor.
    fn ensure_surface(&self) {
        // A pinned surface never follows the focused output - see
        // `Opts::pin_output` and `Osd::pin_output`. Everything else does, because
        // a card is about what the user is doing right now - unless the settings
        // window's Display page has named a primary display, which the
        // notification cards then belong on (see `default_output`).
        let target = match &*self.pin.borrow() {
            Some(pinned) => Some(pinned.clone()),
            None => self.default_output(),
        };
        let current = self
            .surface
            .borrow()
            .as_ref()
            .and_then(|surface| surface.monitor.clone());
        let keep = match (&current, &target) {
            (None, _) => false,
            // Without hyprctl we cannot know where the focused output is;
            // keeping the surface we have beats rebuilding on a guess.
            (Some(_), None) => true,
            (Some(current), Some(target)) => current == target,
        };
        if keep {
            return;
        }

        let content = self.detach_content();
        if let Some(surface) = self.surface.borrow_mut().take() {
            surface.window.destroy();
        }
        let monitor = target
            .as_deref()
            .and_then(monitors::find)
            .or_else(monitors::first);
        let surface = self.make_surface(monitor, target);
        *self.surface.borrow_mut() = Some(surface);
        // Put the element's widgets back into the new card. They belong to the
        // element, not to the surface: everything holding a reference to them
        // (a follower's callback, a timer, a button) still points at these very
        // widgets, so a rebuild that lost them would leave an empty card and a
        // pile of updates going nowhere.
        if let Some(content) = content {
            self.attach_content(&content);
        }
    }

    /// Which output a card belongs on when nothing has pinned it: the settings
    /// window's **primary display** for a *notification* card, and the focused
    /// output otherwise.
    ///
    /// A notification card - the volume, media and session cards, which take no
    /// keyboard - is one of the things that appear by themselves, and designating
    /// a primary display is precisely how a user says "they belong here". A card
    /// the user *drives* with the keyboard - the switcher, the overview, the
    /// launcher, the settings window - deliberately keeps following the focused
    /// output: one that opened on another screen than the one being used would be
    /// unusable, so the primary display is not even consulted for it.
    fn default_output(&self) -> Option<String> {
        if self.opts.keyboard == Keyboard::None {
            if let Some(primary) = monitors::primary_connector() {
                return Some(primary);
            }
        }
        monitors::focused_connector()
    }

    /// Put the card on the output it belongs on, just before it is shown again.
    ///
    /// A notification card is built once, when its element starts, so without this
    /// the primary display (or the focused output) would only be re-read at the
    /// next start-up - and a primary display picked in the settings window would
    /// need that element restarted to mean anything. The call is cheap:
    /// [`Osd::ensure_surface`] rebuilds nothing when the answer has not changed.
    fn retarget(&self) {
        // A bar owns a surface per output and is reconciled by `sync_outputs`;
        // there is no single surface here for `ensure_surface` to move.
        if !matches!(self.placement(), Placement::Bar { .. }) {
            self.ensure_surface();
        }
    }

    /// Build one surface, with a card in it, for one output.
    ///
    /// Does not store anything: [`Osd::ensure_surface`] keeps the single surface
    /// of a card, and [`Osd::sync_outputs`] keeps one per output for a bar. The
    /// widgets in it are the caller's to fill (see [`put_content`]).
    fn make_surface(&self, monitor: Option<gdk::Monitor>, connector: Option<String>) -> Surface {
        let window = gtk::Window::new();
        window.add_css_class("osd");
        // The stylesheet's touch rules hang off this class: they are off while
        // the machine is a laptop, and come on the moment its keyboard is folded
        // away (see the module comment and `crate::touch`).
        apply_touch_class(&window, touch::tablet());
        window.init_layer_shell();
        window.set_namespace(Some(self.opts.namespace.as_str()));
        // A bar lives on the *top* layer, not the overlay one: a fullscreen
        // window is supposed to cover it, exactly as it covers everything else.
        // Cards keep the overlay layer, which is what draws them above a
        // fullscreen window - the whole point of an OSD.
        window.set_layer(match self.placement() {
            Placement::Bar { .. } => Layer::Top,
            _ => Layer::Overlay,
        });
        // The whole point of an OSD: it never takes the keyboard, so the window
        // you were typing in keeps focus while the card is up. The one element
        // that needs the keyboard (the switcher) asks for it through `Opts`,
        // and then only for as long as its card is on screen.
        window.set_keyboard_mode(match self.opts.keyboard {
            Keyboard::None => KeyboardMode::None,
            Keyboard::Exclusive => KeyboardMode::Exclusive,
        });
        // Ignore other surfaces' exclusive zones, and reserve none of our own
        // unless we are the bar - see the `Bar` arm below.
        window.set_exclusive_zone(-1);
        // Anchoring is what decides the surface's size, and layer-shell has no
        // other way of asking: one edge leaves it as small as its content (a
        // card), no edge centres it at that same size ([`Placement::Center`]),
        // and every edge stretches it over the output ([`Placement::Fill`]).
        // Three edges ([`Placement::Bar`]) give a full-width strip of a fixed
        // height, which is what a bar is.
        match self.placement() {
            Placement::Bottom => window.set_anchor(Edge::Bottom, true),
            Placement::TopCard { .. } => window.set_anchor(Edge::Top, true),
            Placement::TopRight { .. } => {
                window.set_anchor(Edge::Top, true);
                window.set_anchor(Edge::Right, true);
            }
            Placement::TopLeft { .. } => {
                window.set_anchor(Edge::Top, true);
                window.set_anchor(Edge::Left, true);
            }
            Placement::Bar {
                height,
                margin_top,
                exclusive,
                ..
            } => {
                for edge in [Edge::Top, Edge::Left, Edge::Right] {
                    window.set_anchor(edge, true);
                }
                // Reserving `height + margin_top` is what keeps windows from
                // being laid out under the bar *and* under the gap above it -
                // tiled windows then start where the bar ends.
                if exclusive {
                    window.set_exclusive_zone(height + margin_top);
                }
            }
            Placement::Fill => {
                for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                    window.set_anchor(edge, true);
                }
            }
            Placement::Center => {}
        }
        if let Some(monitor) = &monitor {
            window.set_monitor(Some(monitor));
        }

        if let Some((press, release)) = self.keys.borrow().as_ref() {
            attach_keys(&window, press, release);
        }

        let card = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        match self.placement() {
            // The CSS asks for this too (a card with no edge takes no rounding
            // and no border); the class is what makes that possible to target.
            Placement::Fill => {
                card.add_css_class("card");
                card.add_css_class("filling");
            }
            Placement::Bar {
                height,
                margin_top,
                margin_x,
                ..
            } => {
                card.add_css_class("bar");
                // The *surface* is already full width and `height` tall; the
                // margins are what turn it into a floating bar.
                card.set_size_request(-1, height);
                card.set_margin_top(margin_top);
                card.set_margin_start(margin_x);
                card.set_margin_end(margin_x);
            }
            Placement::TopCard { margin_top } => {
                card.add_css_class("card");
                card.set_size_request(self.width(), -1);
                card.set_margin_top(margin_top);
                card.set_margin_bottom(SHADOW_PAD);
                card.set_margin_start(SHADOW_PAD);
                card.set_margin_end(SHADOW_PAD);
            }
            Placement::TopRight {
                margin_top,
                margin_right,
            } => {
                card.add_css_class("card");
                card.set_size_request(self.width(), -1);
                card.set_margin_top(margin_top);
                card.set_margin_bottom(SHADOW_PAD);
                card.set_margin_start(SHADOW_PAD);
                card.set_margin_end(margin_right);
            }
            Placement::Bottom | Placement::Center => {
                card.add_css_class("card");
                card.set_size_request(self.width(), -1);
                card.set_margin_top(SHADOW_PAD);
                card.set_margin_bottom(SHADOW_PAD);
                card.set_margin_start(SHADOW_PAD);
                card.set_margin_end(SHADOW_PAD);
            }
            Placement::TopLeft {
                margin_top,
                margin_left,
            } => {
                card.add_css_class("card");
                card.set_size_request(self.width(), -1);
                card.set_margin_top(margin_top);
                card.set_margin_bottom(SHADOW_PAD);
                card.set_margin_start(margin_left);
                card.set_margin_end(SHADOW_PAD);
            }
        }
        window.set_child(Some(&card));
        // A bar is furniture, not a notification: it is up from the moment the
        // process has a surface, and it comes back up by itself after a rebuild
        // (`Osd::hide` is still how a `hide` verb takes it away).
        if matches!(self.placement(), Placement::Bar { .. }) {
            window.set_visible(true);
        }

        // Only the bottom margin is measured as a fraction of the screen, so it
        // is the only one that has to be recomputed when a monitor changes under
        // the card (docking, rotating, a new scale factor).
        if let Some(monitor) = &monitor {
            if self.placement() != Placement::Bottom {
                return Surface {
                    window,
                    card,
                    monitor: connector,
                };
            }
            set_bottom_margin(&window, monitor, self.opts.bottom_margin);
            // A monitor can change under us (docking, rotating, a new scale
            // factor) and the margin is a fraction of its height.
            let weak = window.downgrade();
            let fraction = self.opts.bottom_margin;
            monitor.connect_geometry_notify(move |monitor| {
                if let Some(window) = weak.upgrade() {
                    set_bottom_margin(&window, monitor, fraction);
                }
            });
            let weak = window.downgrade();
            let fraction = self.opts.bottom_margin;
            monitor.connect_scale_factor_notify(move |monitor| {
                if let Some(window) = weak.upgrade() {
                    set_bottom_margin(&window, monitor, fraction);
                }
            });
        }

        Surface {
            window,
            card,
            monitor: connector,
        }
    }

    /// Take the content widget out of the old card before the old surface is
    /// destroyed - otherwise destroying the window would take the element's
    /// widgets with it. Hands it back, so a rebuild can put it into the new card.
    fn detach_content(&self) -> Option<gtk::Widget> {
        let content = self.content.borrow_mut().take()?;
        if let Some(parent) = content.parent() {
            if let Some(card) = parent.downcast_ref::<gtk::Box>() {
                card.remove(&content);
            }
        }
        Some(content)
    }

    fn arm_hide(&self, duration: Duration) {
        if let Some(source) = self.hide.borrow_mut().take() {
            source.remove();
        }
        let me = self.me.clone();
        let source = glib::timeout_add_local_once(duration, move || {
            if let Some(osd) = me.upgrade() {
                osd.hide_unless_held();
            }
        });
        *self.hide.borrow_mut() = Some(source);
    }

    fn hide_unless_held(&self) {
        *self.hide.borrow_mut() = None;
        let held = self
            .stay_open
            .borrow()
            .as_ref()
            .is_some_and(|predicate| predicate());
        if held {
            // Re-check shortly: a drag that ends between two ticks should still
            // fade the card out without a full `duration` of extra waiting.
            self.arm_hide(Duration::from_millis(150));
            return;
        }
        // The auto-hide is a close, so it has to take the tap catcher with it:
        // a card that faded out under a dimmed full-screen surface would leave
        // the screen dim with nothing behind it. `hide` does the same, and an
        // element must not have to remember both.
        if let Some(catcher) = self.catcher.borrow().as_ref() {
            catcher.window.set_visible(false);
        }
        if let Some(surface) = self.surface.borrow().as_ref() {
            surface.window.set_visible(false);
        }
    }
}

/// Which outputs a bar belongs on: every one the session has, or just the
/// connector the config names.
///
/// This is the *decision* inside [`Osd::sync_outputs`], split out from the part
/// that talks to GDK and layer-shell, so that it can be tested without a
/// compositor - a monitor being plugged in is not something a unit test can do,
/// but "which screens should have a bar afterwards" is exactly what goes wrong
/// when one does.
fn wanted_outputs(available: &[String], pinned: Option<&str>) -> Vec<String> {
    available
        .iter()
        .filter(|connector| pinned.is_none_or(|pinned| pinned == connector.as_str()))
        .cloned()
        .collect()
}

/// Add or remove the `touch` class, which is what the stylesheet's fingertip
/// rules hang off (see [`crate::touch`]). Removing it when it is not there, or
/// adding it twice, is a no-op in GTK - which is why the caller does not have to
/// know which state the window is in.
fn apply_touch_class(window: &gtk::Window, touch: bool) {
    if touch {
        window.add_css_class("touch");
    } else {
        window.remove_css_class("touch");
    }
}

/// Lay the catcher's four strips out around the hole.
///
/// Top, bottom, left and right, each the full width or height of its band and
/// each hidden when the card reaches that edge of the screen (a strip of zero
/// size would not catch anything anyway, but a hidden one cannot be painted over
/// the card either).
fn place_strips(catcher: &Catcher, hole: Hole, geometry: Option<(i32, i32)>) {
    let Some((width, height)) = geometry else {
        return;
    };
    let hole = hole.grown(SHADOW_PAD);
    let left = hole.x.clamp(0, width);
    let top = hole.y.clamp(0, height);
    let right = (hole.x + hole.width).clamp(0, width);
    let bottom = (hole.y + hole.height).clamp(0, height);
    // (x, y, width, height) for the four bands.
    let bands = [
        (0, 0, width, top),
        (0, bottom, width, height - bottom),
        (0, top, left, bottom - top),
        (right, top, width - right, bottom - top),
    ];
    for (strip, (x, y, band_width, band_height)) in catcher.strips.iter().zip(bands) {
        let band_width = band_width.max(0);
        let band_height = band_height.max(0);
        strip.set_visible(band_width > 0 && band_height > 0);
        strip.set_size_request(band_width, band_height);
        if let Some(parent) = strip.parent().and_downcast::<gtk::Fixed>() {
            parent.move_(strip, x as f64, y as f64);
        }
    }
}

/// Put a surface's content widget into its card, with whatever the placement
/// wants around it.
/// Free rather than a method because both of the shell's two shapes need it: a
/// card re-attaches the element's widget to a rebuilt surface, and a bar fills
/// each fresh per-output surface as it is made. The widget is only appended if it
/// has no parent yet - a rebuild hands back the same widget, which is still where
/// it was if the surface did not actually change.
fn put_content(card: &gtk::Box, content: &gtk::Widget, placement: &Placement) {
    if content.parent().is_some() {
        return;
    }
    match placement {
        // A bar is full width and pads itself: its pills carry the vertical
        // rhythm, and the bar's own CSS carries the rest.
        //
        // `hexpand` is not a nicety here. A `gtk::Box` hands the slack around to
        // the children that ask for it, and the content asked for nothing - so
        // the bar's `CenterBox` was allocated its *natural* width and sat
        // left-aligned inside the card. That put the clock off-centre by however
        // much the pill row was narrower than the screen, and left the right-hand
        // pills floating in the middle instead of ending at the bar's edge.
        Placement::Bar { .. } => {
            content.set_hexpand(true);
            content.set_halign(gtk::Align::Fill);
        }
        // Every other surface is a card, and the card recipe deliberately has no
        // padding of its own.
        _ => {
            content.set_margin_start(CARD_PAD_X);
            content.set_margin_end(CARD_PAD_X);
            content.set_margin_top(CARD_PAD_Y);
            content.set_margin_bottom(CARD_PAD_Y);
        }
    }
    card.append(content);
}

/// Position the card: `fraction` is measured from the bottom edge of the
/// monitor and includes the transparent frame, so the *card* ends up that far
/// up the screen.
fn set_bottom_margin(window: &gtk::Window, monitor: &gdk::Monitor, fraction: f64) {
    let height = monitor.geometry().height() as f64;
    let margin = (height * fraction).round() as i32 - SHADOW_PAD;
    window.set_margin(Edge::Bottom, margin.max(0));
}

/// Put a key controller on a window, in the capture phase.
fn attach_keys(window: &gtk::Window, press: &KeyPressHandler, release: &KeyReleaseHandler) {
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let press = press.clone();
    controller.connect_key_pressed(move |_, key, _, state| press(&key, state));
    let release = release.clone();
    controller.connect_key_released(move |_, key, _, _| release(&key));
    window.add_controller(controller);
}

/// Builds an element's content widget (the card's child).
///
/// Called once, on the first start-up rather than in the element's `main`,
/// because GTK does not exist until the application has started. Whatever it
/// hands back stays alive for the whole session: the shell only re-parents a
/// card's widget when the OSD moves to another monitor, and a bar's per-output
/// factory is called again whenever a screen appears - so an element can keep
/// and update the very same widgets it built.
pub type Build = Box<dyn FnOnce(&Rc<Osd>) -> Content>;

/// What an element hands the shell when it has finished starting up.
///
/// Two shapes, because there are two kinds of element here. A **card** is one
/// widget on one screen at a time: the shell moves the surface to whichever
/// output the card belongs on, and the widget goes with it. A **bar** is on
/// every screen at once, and one widget cannot be in two surfaces - a GTK widget
/// has exactly one parent - so there is no single widget to hand over. It hands
/// over a *factory* instead, and the shell calls it once for each output the bar
/// belongs on.
pub enum Content {
    /// One widget, in one surface: every card.
    Single(gtk::Widget),
    /// How to build one surface's content, called once per output: the bar.
    PerOutput(Rc<dyn Fn(&str) -> gtk::Widget>),
}

/// Handles one command, from a fresh start or forwarded from another invocation
/// of the same binary. `Ok` with a non-empty message prints it on the caller's
/// stdout; `Err` prints on its stderr and exits non-zero.
pub type Handle = Rc<dyn Fn(&Rc<Osd>, &[String]) -> Result<String, String>>;

/// Run an element: one GTK application, one card, verbs on the command line.
///
/// The verbs are the element's own business - `build` creates its widgets and
/// `handle` reacts to what the user asked for. Everything else (the layer
/// surface, where it goes, when it leaves, and making sure a second invocation
/// of the binary reaches this process instead of opening a second card) is the
/// shell's.
pub fn run(opts: Opts, build: Build, handle: Handle) -> glib::ExitCode {
    // The elements' own icons (Lucide, linked into this library) have to be in
    // the process before the first card is built - including for a `status`
    // verb, which prints readings and builds no widgets at all. Registering
    // resources needs no display; attaching them to the icon theme does, and
    // that happens when the first icon is drawn (`icons::lucide`).
    crate::icons::register();

    let app = gtk::Application::builder()
        .application_id(opts.app_id.as_str())
        // Verbs arrive on the command line, so GApplication must not try to
        // interpret them as GTK options - and forwarding them to the running
        // instance is exactly the single-instance behaviour we want.
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    let state: Rc<RefCell<Option<Rc<Osd>>>> = Rc::new(RefCell::new(None));
    let pending: Rc<RefCell<Option<Build>>> = Rc::new(RefCell::new(Some(build)));

    let setup = {
        let state = state.clone();
        let pending = pending.clone();
        let opts = opts.clone();
        move |app: &gtk::Application| -> Rc<Osd> {
            if let Some(osd) = state.borrow().as_ref() {
                return osd.clone();
            }
            css::install(&opts.css);
            let osd = Osd::new(app, opts.clone());
            if let Some(build) = pending.borrow_mut().take() {
                match build(&osd) {
                    // One widget: the shell owns the surface it goes in.
                    Content::Single(content) => osd.set_content(&content),
                    // A factory: the shell asks it for one output's widgets at a
                    // time, now and whenever a screen appears.
                    Content::PerOutput(factory) => osd.set_output_content(factory),
                }
            }
            // The widgets exist and their surfaces are up, so the tablet watch
            // can start: it is what moves the `touch` class when the machine is
            // folded, and what the elements that own a size of their own listen
            // to (see `Osd::on_tablet_change`).
            osd.begin_touch();
            *state.borrow_mut() = Some(osd.clone());
            osd
        }
    };

    {
        let setup = setup.clone();
        app.connect_activate(move |app| {
            // Started without a command (Hyprland's `exec-once`): just be there,
            // so the first key press finds a daemon instead of paying for GTK
            // start-up before the card can appear.
            setup(app);
        });
    }
    {
        let setup = setup.clone();
        app.connect_command_line(move |app, cmdline| {
            let osd = setup(app);
            let args = command_args(&cmdline.arguments());
            match handle(&osd, &args) {
                Ok(message) if !message.is_empty() => {
                    cmdline.print_literal(&format!("{message}\n"));
                    glib::ExitCode::SUCCESS
                }
                Ok(_) => glib::ExitCode::SUCCESS,
                Err(error) => {
                    // The client's stderr, so the failure shows up in whatever
                    // ran the binary (journal, `hyprctl dispatch exec`, a shell).
                    cmdline.printerr_literal(&format!("hypr-osd: {error}\n"));
                    glib::ExitCode::FAILURE
                }
            }
        });
    }

    // A daemon that started child processes has to get the chance to stop them,
    // and a plain `kill` would not: POSIX does not reap children with their
    // parent. Quitting properly runs the `shutdown` signal, which is what
    // `Osd::on_shutdown` hooks into. (glib-rs does not export the signal
    // constants and they are stable, so they are spelled out.)
    const SIGINT: i32 = 2;
    const SIGTERM: i32 = 15;
    for signal in [SIGINT, SIGTERM] {
        let app = app.clone();
        glib::unix_signal_add_local(signal, move || {
            app.quit();
            glib::ControlFlow::Break
        });
    }

    app.run()
}

/// The command line without `argv[0]`.
///
/// `GApplicationCommandLine::arguments()` hands over the argv of the process that
/// was run: this binary when it started the daemon, or the *client* that
/// forwarded its arguments over D-Bus. In both cases the program name is the
/// first argument, and a program name is never a verb: it is either a path (it
/// contains a `/`) or exactly the name this process was started as.
///
/// Deliberately *not* `std::env::current_exe()`: that reads `/proc/self/exe`,
/// which grows a " (deleted)" suffix the moment the binary on disk is replaced -
/// and replacing it is exactly what re-running `install.sh` does. A daemon that
/// survives an install would then reject every command as an unknown verb.
fn command_args(argv: &[OsString]) -> Vec<String> {
    let program = std::env::args()
        .next()
        .and_then(|arg| file_name(&arg).map(str::to_owned));
    strip_program(argv, program.as_deref())
}

/// The last element of a path, or the whole string when there is no path.
fn file_name(path: &str) -> Option<&str> {
    path.rsplit('/').next().filter(|name| !name.is_empty())
}

fn strip_program(argv: &[OsString], program: Option<&str>) -> Vec<String> {
    let text = |arg: &OsString| arg.to_string_lossy().into_owned();
    let leading_program = argv
        .first()
        .map(text)
        .is_some_and(|first| first.contains('/') || Some(first.as_str()) == program);
    let skip = usize::from(leading_program);
    argv[skip..].iter().map(text).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    #[test]
    fn strips_the_program_name_from_the_arguments() {
        let name = Some("hypr-osd-volume");
        // Absolute path, relative path, bare name from $PATH - all three are
        // program names, and the verb behind them survives.
        assert_eq!(
            strip_program(&args(&["/usr/local/bin/hypr-osd-volume", "up"]), name),
            vec!["up"]
        );
        assert_eq!(
            strip_program(
                &args(&["./target/release/hypr-osd-volume", "set", "40"]),
                name
            ),
            vec!["set", "40"]
        );
        assert_eq!(
            strip_program(&args(&["hypr-osd-volume", "down"]), name),
            vec!["down"]
        );
        // A daemon start: no verb, an empty command line.
        assert!(strip_program(&args(&["/home/lars/.local/bin/hypr-osd-volume"]), name).is_empty());
    }

    #[test]
    fn a_bar_belongs_on_every_output_unless_the_config_names_one() {
        let outputs = || vec!["eDP-1".to_string(), "DP-1".to_string()];
        // The default: a bar on each screen.
        assert_eq!(wanted_outputs(&outputs(), None), outputs());
        // `output = DP-1` in the config: only that screen.
        assert_eq!(wanted_outputs(&outputs(), Some("DP-1")), vec!["DP-1"]);
        // A pinned connector that is not plugged in gets no bar at all, rather
        // than a bar somewhere else: the config said where it goes. (The bar
        // element falls back to the focused screen only when nothing is pinned.)
        assert!(wanted_outputs(&outputs(), Some("DP-9")).is_empty());
    }
}

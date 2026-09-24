//! The layer-shell card, and the application shell around it.
//!
//! An element owns exactly one card. The card lives in a borderless layer-shell
//! surface on the *overlay* layer, so it draws above fullscreen windows, takes
//! no keyboard focus (the window you were typing in keeps it) and can take
//! pointer input - the volume slider needs that, a plain indicator does not
//! care. Everything outside the card is transparent, which is what makes
//! `osd.lua`'s `ignore_alpha` layer rule able to let clicks through there.
//!
//! The element supplies its content widget (built once) and a command handler;
//! the shell takes care of where the card goes, when it disappears, and how a
//! second invocation of the binary reaches the running instance.

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
}

/// What the card does with the keyboard.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Keyboard {
    /// Never takes it. The window you were typing in keeps typing, which is the
    /// point of an OSD and the default for the whole collection.
    None,
    /// Grabs it while the card is up. Only for a card driven by keys you are
    /// holding *right now*: the switcher has to see the Alt release that ends a
    /// switch, and it cannot see it without the keyboard. An OSD that eats a
    /// keystroke is a bug; this is the one element where it is the feature.
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

struct Surface {
    window: gtk::Window,
    card: gtk::Box,
    /// Connector of the monitor this surface sits on, so a command can tell
    /// whether it has to be rebuilt for a different output.
    monitor: Option<String>,
}

pub struct Osd {
    opts: Opts,
    app: gtk::Application,
    /// Keeps the application alive between key presses: the card is hidden most
    /// of the time, and GApplication quits when nothing holds it.
    _hold: gio::ApplicationHoldGuard,
    me: Weak<Osd>,
    surface: RefCell<Option<Surface>>,
    /// The element's content widget, i.e. the card's child.
    content: RefCell<Option<gtk::Widget>>,
    hide: RefCell<Option<glib::SourceId>>,
    stay_open: RefCell<Option<Box<dyn Fn() -> bool>>>,
    /// Key handlers, installed on every surface the shell builds (see
    /// [`Osd::on_keys`]).
    keys: RefCell<Option<(KeyPressHandler, KeyReleaseHandler)>>,
}

impl Osd {
    fn new(app: &gtk::Application, opts: Opts) -> Rc<Self> {
        let hold = app.hold();
        Rc::new_cyclic(|me| Osd {
            opts,
            app: app.clone(),
            _hold: hold,
            me: me.clone(),
            surface: RefCell::new(None),
            content: RefCell::new(None),
            hide: RefCell::new(None),
            stay_open: RefCell::new(None),
            keys: RefCell::new(None),
        })
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

    /// Put the element's content into the card, on the focused monitor. Calling
    /// this on every command is deliberate: it is what moves the OSD to the
    /// screen you are actually using, rebuilding the surface only when the
    /// focused output changed.
    pub fn set_content(&self, widget: &impl IsA<gtk::Widget>) {
        let content = widget.clone().upcast::<gtk::Widget>();
        self.ensure_surface();
        let card = self
            .surface
            .borrow()
            .as_ref()
            .map(|surface| surface.card.clone());
        let Some(card) = card else {
            return;
        };
        if content.parent().is_none() {
            content.set_margin_start(CARD_PAD_X);
            content.set_margin_end(CARD_PAD_X);
            content.set_margin_top(CARD_PAD_Y);
            content.set_margin_bottom(CARD_PAD_Y);
            card.append(&content);
        }
        *self.content.borrow_mut() = Some(content);
    }

    /// Show the card and hide it again after `duration`, unless
    /// [`Osd::set_stay_open_when`] says otherwise by then.
    pub fn reveal(&self, duration: Duration) {
        if let Some(surface) = self.surface.borrow().as_ref() {
            surface.window.set_visible(true);
        }
        self.arm_hide(duration);
    }

    /// Hide immediately (and forget any pending timer).
    pub fn hide(&self) {
        if let Some(source) = self.hide.borrow_mut().take() {
            source.remove();
        }
        if let Some(surface) = self.surface.borrow().as_ref() {
            surface.window.set_visible(false);
        }
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
        if let Some(surface) = self.surface.borrow().as_ref() {
            surface.window.set_visible(true);
        }
    }

    /// Whether the card is on screen right now.
    ///
    /// The shell owns the auto-hide timer, so only it knows: a keybinding that
    /// toggles the card away needs to ask.
    pub fn is_visible(&self) -> bool {
        self.surface
            .borrow()
            .as_ref()
            .is_some_and(|surface| surface.window.is_visible())
    }

    /// Keep the card up while the pointer rests on `widget`.
    ///
    /// A card with buttons must not disappear while you are aiming at one - the
    /// auto-hide would otherwise be a race against the pointer.
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
        self.set_stay_open_when(move || hovered.get());
    }

    /// Keep the card up while `predicate` returns true, instead of hiding it
    /// after the element's duration. An element that has more than one reason to
    /// hold the card open combines them in one closure (the shell keeps a single
    /// predicate).
    pub fn set_stay_open_when(&self, predicate: impl Fn() -> bool + 'static) {
        *self.stay_open.borrow_mut() = Some(Box::new(predicate));
    }

    /// Make sure the surface exists and sits on the focused monitor.
    fn ensure_surface(&self) {
        let target = monitors::focused_connector();
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

        self.detach_content();
        if let Some(surface) = self.surface.borrow_mut().take() {
            surface.window.destroy();
        }
        let monitor = target
            .as_deref()
            .and_then(monitors::find)
            .or_else(monitors::first);
        self.build(monitor, target);
    }

    fn build(&self, monitor: Option<gdk::Monitor>, connector: Option<String>) {
        let window = gtk::Window::new();
        window.add_css_class("osd");
        window.init_layer_shell();
        window.set_namespace(Some(self.opts.namespace.as_str()));
        window.set_layer(Layer::Overlay);
        // The whole point of an OSD: it never takes the keyboard, so the window
        // you were typing in keeps focus while the card is up. The one element
        // that needs the keyboard (the switcher) asks for it through `Opts`,
        // and then only for as long as its card is on screen.
        window.set_keyboard_mode(match self.opts.keyboard {
            Keyboard::None => KeyboardMode::None,
            Keyboard::Exclusive => KeyboardMode::Exclusive,
        });
        // Ignore other surfaces' exclusive zones (e.g. the bar's) and reserve
        // none of our own.
        window.set_exclusive_zone(-1);
        // With no edge anchored, layer-shell centres the surface on the output -
        // which is exactly what [`Placement::Center`] wants.
        let anchored = self.opts.placement == Placement::Bottom;
        if anchored {
            window.set_anchor(Edge::Bottom, true);
        }
        if let Some(monitor) = &monitor {
            window.set_monitor(Some(monitor));
        }

        if let Some((press, release)) = self.keys.borrow().as_ref() {
            attach_keys(&window, press, release);
        }

        let card = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        card.add_css_class("card");
        card.set_size_request(self.opts.width, -1);
        card.set_margin_top(SHADOW_PAD);
        card.set_margin_bottom(SHADOW_PAD);
        card.set_margin_start(SHADOW_PAD);
        card.set_margin_end(SHADOW_PAD);
        window.set_child(Some(&card));

        // A centred card has no margin to keep up to date: the bottom edge of
        // the screen is what moves when a monitor does.
        if let Some(monitor) = &monitor {
            if !anchored {
                *self.surface.borrow_mut() = Some(Surface {
                    window,
                    card,
                    monitor: connector,
                });
                return;
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

        *self.surface.borrow_mut() = Some(Surface {
            window,
            card,
            monitor: connector,
        });
    }

    /// Take the content widget out of the old card before the old surface is
    /// destroyed - otherwise destroying the window would take the element's
    /// widgets with it.
    fn detach_content(&self) {
        if let Some(content) = self.content.borrow_mut().take() {
            if let Some(parent) = content.parent() {
                if let Some(card) = parent.downcast_ref::<gtk::Box>() {
                    card.remove(&content);
                }
            }
        }
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
        if let Some(surface) = self.surface.borrow().as_ref() {
            surface.window.set_visible(false);
        }
    }
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
/// because GTK does not exist until the application has started. The widget
/// stays alive for the whole session: the shell only re-parents it when the
/// OSD moves to another monitor, so an element can keep and update the very
/// same widgets it built.
pub type Build = Box<dyn FnOnce(&Rc<Osd>) -> gtk::Widget>;

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
                let content = build(&osd);
                osd.set_content(&content);
            }
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
}

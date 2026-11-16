//! `hypr-osd-session` - element #3: the session card.
//!
//! Lock, suspend, log out, reboot, shut down - the same menu the bar's popup
//! shows (the design was taken from the old `~/.config/waybar/scripts/popup.py`,
//! which waybar is no longer there to run), as a card that appears where
//! you are looking when you press the power button or `SUPER + SHIFT + L`.
//!
//! Verbs (`hypr-osd-session <verb>`):
//!
//! ```text
//!   toggle               show the card, or hide it again - what the power key
//!                        and SUPER + SHIFT + L are bound to
//!   show                 reveal it without toggling
//!   lock | suspend | logout | reboot | poweroff
//!                        run one action right now. No confirmation here: a verb
//!                        someone typed (or bound) is already deliberate, and the
//!                        card's "click again" rule exists for the clicks that
//!                        are not
//!   status               print what each action would run, and which of them
//!                        this machine cannot do
//!   (no verb)            start the daemon and wait for the power key
//! ```
//!
//! # On a tablet
//!
//! The card is opened by a tap (the bar's power pill, or a system gesture) and
//! there is no key that could toggle it away again, so on a tablet it gets a
//! **tap catcher** around itself: a tap anywhere outside the card closes it, and
//! the shell's dimmed bands are what make that a whole-screen target instead of a
//! lucky one. The rows are finger-sized by the stylesheet (`.touch button.row` in
//! `session.css`), the hint line says "tap outside to close", and the card's
//! default duration is longer than on a laptop - it is a menu, and a finger is
//! slower to read it than a pointer. A `duration_ms` written in the config is
//! used on both, so the config is never silently overruled; only the *default*
//! follows the machine.

mod actions;
mod view;

use std::cell::OnceCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::touch;
use hypr_osd_core::{css, run, Config, Content, Opts, Osd};

use view::SessionView;

/// D-Bus application id - and therefore the single-instance key: the power key
/// and the keybinding reach the running daemon instead of starting a second one.
const APP_ID: &str = "com.schells2.osd.session";
/// Layer-shell namespace. The whole collection shares the `hypr-osd` prefix so
/// one layer rule in `osd.lua` covers every element.
const NAMESPACE: &str = "hypr-osd";
/// Element name: `~/.config/hypr-osd/session.conf`.
const ELEMENT: &str = "session";

// Defaults; every one of them is overridable in the config file.
//
// Longer than the other cards: this one is a menu, and it is read before it is
// used. `duration_ms = 0` keeps it up until an action is picked or the key is
// pressed again.
const DEFAULT_DURATION_MS: u64 = 8000;
/// The same default on a tablet, where reading five rows and deciding is slower
/// (and where the way out is a tap rather than the key that opened it). A
/// `duration_ms` in the config overrides this exactly like the laptop default:
/// the setting is the user's, the default is the mode's.
const DEFAULT_TOUCH_DURATION_MS: u64 = 15_000;
/// 0 = as wide as the rows need to be.
const DEFAULT_WIDTH: i32 = 0;
const DEFAULT_BOTTOM_MARGIN: f64 = 0.12;

/// What the config file resolved to.
struct Settings {
    /// How long the card stays after the last change, when the config says.
    /// `None` means the element's own default, which is longer on a tablet; zero
    /// means "until dismissed".
    duration: Option<Duration>,
    /// The card's minimum width in pixels.
    width: i32,
    /// Distance from the bottom of the screen, as a fraction of its height.
    bottom_margin: f64,
}

impl Settings {
    fn load(config: &Config) -> Self {
        Settings {
            // `get` rather than `millis`: the typed getter cannot tell a value
            // that was written down from the default it would have supplied, and
            // the two modes have different defaults.
            duration: config
                .get("duration_ms")
                .and_then(|raw| raw.trim().parse::<u64>().ok())
                .map(Duration::from_millis),
            width: config.i32("width", DEFAULT_WIDTH),
            bottom_margin: config.float("bottom_margin", DEFAULT_BOTTOM_MARGIN),
        }
    }

    /// How long this reveal should last, on the machine as it is right now.
    ///
    /// Asked at reveal time rather than at start-up: a session that started with
    /// the cover attached can be folded into a tablet, and the menu revealed then
    /// should get the tablet's reading time.
    fn duration(&self) -> Duration {
        self.duration.unwrap_or_else(|| {
            Duration::from_millis(if touch::tablet() {
                DEFAULT_TOUCH_DURATION_MS
            } else {
                DEFAULT_DURATION_MS
            })
        })
    }
}

fn main() -> glib::ExitCode {
    let config = Config::load(ELEMENT);
    let settings = Rc::new(Settings::load(&config));

    let opts = Opts {
        app_id: APP_ID.to_string(),
        css: css::stylesheet(include_str!("session.css")),
        namespace: NAMESPACE.to_string(),
        width: settings.width,
        bottom_margin: settings.bottom_margin,
        // The rest is the OSD default: bottom-anchored, and no interest in the
        // keyboard - see `Opts` for the two elements that differ.
        ..Opts::default()
    };

    // The widgets are built inside the application's start-up (GTK does not
    // exist before that), while the command handler is installed before it - so
    // the two meet here.
    let view: Rc<OnceCell<Rc<SessionView>>> = Rc::new(OnceCell::new());

    let build = {
        let view = view.clone();
        let settings = settings.clone();
        Box::new(move |osd: &Rc<Osd>| {
            let session = Rc::new(SessionView::new(settings.width, settings.bottom_margin));
            // The daemon starts in whatever mode the machine is in, and the shell
            // only calls the tablet handlers on a *change*: the starting answer
            // has to be applied by hand.
            session.apply_touch(touch::tablet());
            session.hook(osd);

            // The machine can be folded (or unfolded) under the card: the rows'
            // touch rules follow the window's `touch` class on their own, but the
            // hint and the way out do not - a card that opened as a laptop card
            // and is now a tablet card must grow a tap catcher, and one that
            // stopped being a tablet card must lose it.
            let watch = osd.clone();
            let watched = session.clone();
            osd.on_tablet_change(move |tablet| {
                watched.apply_touch(tablet);
                if watch.is_visible() {
                    watched.arm_touch(&watch);
                } else {
                    watch.tap_catcher(None, || {});
                }
            });

            let _ = view.set(session.clone());
            Content::Single(session.root.clone().upcast::<gtk::Widget>())
        })
    };

    let handle = {
        let view = view.clone();
        let settings = settings.clone();
        Rc::new(
            move |osd: &Rc<Osd>, args: &[String]| -> Result<String, String> {
                let view = view.get().ok_or("the OSD has not finished starting up")?;
                let Some(verb) = args.first().map(String::as_str) else {
                    // No verb: started by Hyprland's `exec-once`, so just wait
                    // for the power key.
                    return Ok(String::new());
                };

                // An action verb: run it, wherever the card is.
                if let Some(id) = actions::Id::from_verb(verb) {
                    let action = actions::find(id)
                        .ok_or_else(|| format!("this build has no `{verb}` action"))?;
                    actions::run(&action)?;
                    if osd.is_visible() {
                        osd.hide();
                    }
                    return Ok(String::new());
                }

                match verb {
                    "toggle" => {
                        if osd.is_visible() {
                            osd.hide();
                        } else {
                            // A card that comes back must not come back armed - or
                            // stale: this is the moment a locker that was installed
                            // while the daemon was running gets noticed.
                            view.refresh();
                            view.disarm_all();
                            reveal(osd, &settings, view);
                        }
                        Ok(String::new())
                    }
                    "show" => {
                        view.refresh();
                        view.disarm_all();
                        reveal(osd, &settings, view);
                        Ok(String::new())
                    }
                    "status" => Ok(actions::all()
                        .iter()
                        .map(|action| action.describe())
                        .collect::<Vec<_>>()
                        .join("\n")),
                    other => Err(format!(
                        "unknown command `{other}` \
                         (toggle | show | lock | suspend | logout | reboot | poweroff | status)"
                    )),
                }
            },
        )
    };

    run(opts, build, handle)
}

/// Reveal the card: for the configured duration, or until it is dismissed when
/// the config asks for that (`duration_ms = 0`) - and, on a tablet, with the tap
/// catcher as the way out.
fn reveal(osd: &Rc<Osd>, settings: &Settings, view: &Rc<SessionView>) {
    let duration = settings.duration();
    if duration.is_zero() {
        osd.show();
    } else {
        osd.reveal(duration);
    }
    // Armed *after* the reveal, because the card's size is what the hole is made
    // of, and a card that is not on screen has no size to speak of.
    view.arm_touch(osd);
}

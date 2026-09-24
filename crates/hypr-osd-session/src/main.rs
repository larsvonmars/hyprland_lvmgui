//! `hypr-osd-session` - element #3: the session card.
//!
//! Lock, suspend, log out, reboot, shut down - the same menu the bar's popup
//! shows (see `~/.config/waybar/scripts/popup.py`), as a card that appears where
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

mod actions;
mod view;

use std::cell::OnceCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::{css, run, Config, Opts, Osd};

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
/// 0 = as wide as the rows need to be.
const DEFAULT_WIDTH: i32 = 0;
const DEFAULT_BOTTOM_MARGIN: f64 = 0.12;

/// What the config file resolved to.
struct Settings {
    /// How long the card stays after the last change; zero means "until
    /// dismissed".
    duration: Duration,
    /// The card's minimum width in pixels.
    width: i32,
    /// Distance from the bottom of the screen, as a fraction of its height.
    bottom_margin: f64,
}

impl Settings {
    fn load(config: &Config) -> Self {
        Settings {
            duration: config.millis("duration_ms", DEFAULT_DURATION_MS),
            width: config.i32("width", DEFAULT_WIDTH),
            bottom_margin: config.float("bottom_margin", DEFAULT_BOTTOM_MARGIN),
        }
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
        Box::new(move |osd: &Rc<Osd>| {
            let session = Rc::new(SessionView::new());
            session.hook(osd);
            let _ = view.set(session.clone());
            session.root.clone().upcast::<gtk::Widget>()
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
                            reveal(osd, &settings);
                        }
                        Ok(String::new())
                    }
                    "show" => {
                        view.refresh();
                        view.disarm_all();
                        reveal(osd, &settings);
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
/// the config asks for that (`duration_ms = 0`).
fn reveal(osd: &Rc<Osd>, settings: &Settings) {
    if settings.duration.is_zero() {
        osd.show();
    } else {
        osd.reveal(settings.duration);
    }
}

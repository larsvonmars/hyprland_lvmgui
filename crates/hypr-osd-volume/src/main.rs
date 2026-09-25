//! `hypr-osd-volume` - the collection's first element: a volume card.
//!
//! It owns the volume *step*, not just the display: `~/.config/hypr/osd.lua`
//! binds the volume keys to `hypr-osd-volume up | down | toggle`, the daemon
//! behind them runs `wpctl` and shows the card with the result, and dragging the
//! card's slider sets the volume the same way. One source of truth for what the
//! keys do, and the card can never show a value the sink does not have.
//!
//! Verbs (`hypr-osd-volume <verb>`):
//!
//! ```text
//!   up | raise      +step_percent, unmuting on the way (see unmute_on_raise)
//!   down | lower    -step_percent
//!   toggle | mute   mute/unmute, keeping the level
//!   set <percent>   an absolute level, e.g. `set 40`
//!   show            reveal the card without changing anything
//!   status          print the level and exit (no card)
//!   (no verb)       start the daemon and wait for the first key press
//! ```

mod sink;
mod view;

use std::cell::OnceCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::{css, run, Config, Content, Opts, Osd};

use view::VolumeView;

/// D-Bus application id - and therefore the single-instance key: a key press
/// reaches the running daemon instead of starting a second one.
const APP_ID: &str = "com.schells2.osd.volume";
/// Layer-shell namespace. The whole collection shares the `hypr-osd` prefix so
/// one layer rule in `osd.lua` covers every element.
const NAMESPACE: &str = "hypr-osd";
/// Element name: `~/.config/hypr-osd/volume.conf`.
const ELEMENT: &str = "volume";

// Defaults; every one of them is overridable in the config file.
const DEFAULT_STEP: f64 = 5.0;
const DEFAULT_MAX: f64 = 100.0;
const DEFAULT_DURATION_MS: u64 = 1400;
const DEFAULT_WIDTH: i32 = 340;
const DEFAULT_BOTTOM_MARGIN: f64 = 0.12;

/// What the config file resolved to.
struct Settings {
    /// Percent per key press.
    step: f64,
    /// The slider's ceiling, i.e. how far wpctl may amplify (100 = no
    /// over-amplification).
    max: f64,
    /// How long the card stays after the last change.
    duration: Duration,
    /// Unmute when a raise (or a drag) actually makes the sink louder.
    unmute_on_raise: bool,
    /// The card's width in pixels.
    width: i32,
    /// Distance from the bottom of the screen, as a fraction of its height.
    bottom_margin: f64,
}

impl Settings {
    fn load(config: &Config) -> Self {
        Settings {
            step: config.float("step_percent", DEFAULT_STEP),
            max: config.float("max_percent", DEFAULT_MAX),
            duration: config.millis("duration_ms", DEFAULT_DURATION_MS),
            unmute_on_raise: config.bool("unmute_on_raise", true),
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
        css: css::stylesheet(include_str!("volume.css")),
        namespace: NAMESPACE.to_string(),
        width: settings.width,
        bottom_margin: settings.bottom_margin,
        // The rest is the OSD default: bottom-anchored, and no interest in the
        // keyboard - see `Opts` for the two elements that differ.
        ..Opts::default()
    };

    // The widgets are built inside the application's start-up (GTK does not
    // exist before that), while the command handler is installed before it - so
    // the two meet here. The element keeps the same widget instances for the
    // whole session, which is what lets a command simply repaint the card.
    let view: Rc<OnceCell<Rc<VolumeView>>> = Rc::new(OnceCell::new());

    let build = {
        let view = view.clone();
        let settings = settings.clone();
        Box::new(move |osd: &Rc<Osd>| {
            let volume = Rc::new(VolumeView::new(&settings));
            volume.hook(osd);
            let _ = view.set(volume.clone());
            Content::Single(volume.root.clone().upcast::<gtk::Widget>())
        })
    };

    let handle = {
        let view = view.clone();
        let settings = settings.clone();
        Rc::new(
            move |osd: &Rc<Osd>, args: &[String]| -> Result<String, String> {
                let view = view.get().ok_or("the OSD has not finished starting up")?;
                let Some(verb) = args.first().map(String::as_str) else {
                    // No verb: started by Hyprland's `exec-once`, so just wait for
                    // the first key press (a card nobody asked for would be wrong).
                    return Ok(String::new());
                };

                let sink = match verb {
                    "up" | "raise" => sink::nudge(
                        settings.step / 100.0,
                        settings.max / 100.0,
                        settings.unmute_on_raise,
                    )?,
                    "down" | "lower" => {
                        sink::nudge(-settings.step / 100.0, settings.max / 100.0, false)?
                    }
                    "toggle" | "mute" => sink::toggle_mute()?,
                    "set" => {
                        let percent = args
                            .get(1)
                            .ok_or("`set` needs a percentage, e.g. `set 40`")?
                            .parse::<f64>()
                            .map_err(|_| "`set` needs a number, e.g. `set 40`")?;
                        sink::set_percent(percent, settings.max / 100.0, settings.unmute_on_raise)?
                    }
                    "show" => sink::read()?,
                    "status" => {
                        let sink = sink::read()?;
                        return Ok(format!(
                            "{}%{}",
                            sink::percent(&sink),
                            if sink.muted { " (muted)" } else { "" }
                        ));
                    }
                    other => {
                        return Err(format!(
                            "unknown command `{other}` \
                         (up | down | toggle | set <percent> | show | status)"
                        ))
                    }
                };

                view.render(&sink);
                osd.reveal(settings.duration);
                Ok(String::new())
            },
        )
    };

    run(opts, build, handle)
}

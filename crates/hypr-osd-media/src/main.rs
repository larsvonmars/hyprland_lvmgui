//! `hypr-osd-media` - element #2: a card that appears when something starts
//! playing, with the cover, the title, the artist and two skip buttons.
//!
//! Unlike the volume card, this element owns no *step* - it watches. A single
//! `playerctl metadata --follow` runs for the session and reports every metadata
//! change; an event that is playing, has something to show and is not the track
//! already on the card is "a new medium", and that is what reveals the card. The
//! buttons run `playerctl next/previous`, and the card then updates from the
//! follower like any other track change - a click and a key press take exactly
//! the same path.
//!
//! Verbs (`hypr-osd-media <verb>`):
//!
//! ```text
//!   next                 skip forward, then show what is playing
//!   previous             skip back, then show what is playing
//!   show                 reveal the card for the current track
//!   status               print "Title — Artist", no card
//!   (no verb)            start the daemon and wait for something to play
//! ```
//!
//! # On a tablet
//!
//! The card appears by itself (a track started), so unlike the switcher it needs
//! no new entry point - but it still has to be leaveable by finger: while the
//! machine is a tablet, every reveal arms a tap catcher around the card, so a tap
//! anywhere else closes it before the duration does. The two skip buttons are
//! icons explained by tooltips, which a touchscreen cannot reach; on a tablet the
//! card therefore shows a line under them saying what they do and how it closes
//! (see [`view::MediaView::apply_touch`]), the controls grow to 44px under
//! `.touch`, and the text column gives up exactly the width they grew by so the
//! card stays the width the config asked for. The default duration is longer on a
//! tablet - the title, the artist and the cover are all read at arm's length; a
//! `duration_ms` in the config is used on both and is never overruled.

mod view;

use std::cell::OnceCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::mpris::{self, Direction, Player};
use hypr_osd_core::touch;
use hypr_osd_core::{css, run, Config, Content, Opts, Osd};

use view::MediaView;

/// D-Bus application id - and therefore the single-instance key: a command
/// reaches the running daemon instead of starting a second one.
const APP_ID: &str = "com.schells2.osd.media";
/// Layer-shell namespace. The whole collection shares the `hypr-osd` prefix so
/// one layer rule in `osd.lua` covers every element.
const NAMESPACE: &str = "hypr-osd";
/// Element name: `~/.config/hypr-osd/media.conf`.
const ELEMENT: &str = "media";

// Defaults; every one of them is overridable in the config file.
//
// The card is shown for longer than the volume card on purpose: there is a
// title and an artist to read, and often a cover to look at.
const DEFAULT_DURATION_MS: u64 = 4000;
/// The same default on a tablet: a finger is not as quick as a keypress, and the
/// cover and the title are what the card is for. Config always wins (see
/// [`Settings::duration`]).
const DEFAULT_TOUCH_DURATION_MS: u64 = 7000;
const DEFAULT_WIDTH: i32 = 400;
const DEFAULT_BOTTOM_MARGIN: f64 = 0.12;

/// What the config file resolved to.
struct Settings {
    /// How long the card stays after the last change, when the config says.
    /// `None` means the element's own default, which is longer on a tablet.
    duration: Option<Duration>,
    /// Only show the card for something that is actually *playing*. A player
    /// that loads a track while paused (a queue, a browser restoring tabs) is
    /// not "a new medium starting to be played".
    only_when_playing: bool,
    /// The card's width in pixels.
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
            only_when_playing: config.bool("only_when_playing", true),
            width: config.i32("width", DEFAULT_WIDTH),
            bottom_margin: config.float("bottom_margin", DEFAULT_BOTTOM_MARGIN),
        }
    }

    /// How long a reveal should last, on the machine as it is right now.
    ///
    /// Asked when the card is shown rather than at start-up: a session that
    /// started with the cover attached can be folded into a tablet, and the card
    /// revealed then should get the tablet's reading time.
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
        css: css::stylesheet(include_str!("media.css")),
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
    let view: Rc<OnceCell<Rc<MediaView>>> = Rc::new(OnceCell::new());

    let build = {
        let view = view.clone();
        let settings = settings.clone();
        Box::new(move |osd: &Rc<Osd>| {
            let media = Rc::new(MediaView::new(settings.width, settings.bottom_margin));
            // The daemon starts in whatever mode the machine is in, and the shell
            // only calls the tablet handlers on a *change*: the starting answer
            // has to be applied by hand.
            media.apply_touch(touch::tablet());
            media.hook(osd);

            // The machine can be folded (or unfolded) under the card: the CSS
            // follows the window's `touch` class on its own, but the width split
            // and the way out do not.
            let watch = osd.clone();
            let watched = media.clone();
            osd.on_tablet_change(move |tablet| {
                watched.apply_touch(tablet);
                if watch.is_visible() {
                    watched.arm_touch(&watch);
                } else {
                    watch.tap_catcher(None, || {});
                }
            });

            // The card's trigger. Everything it needs to know arrives on this
            // one callback, so the rules for "is this worth showing?" live in
            // one place.
            let follower = mpris::follow({
                let media = media.clone();
                let osd = osd.clone();
                let settings = settings.clone();
                move |event| {
                    if settings.only_when_playing && !event.playing {
                        return;
                    }
                    if event.track.is_empty() {
                        return;
                    }
                    if !media.is_new_track(&event.track) {
                        return;
                    }
                    media.render(&event.track);
                    osd.reveal(settings.duration());
                    // A card that appeared by itself still needs a way out on a
                    // tablet - a tap anywhere outside it.
                    media.arm_touch(&osd);
                }
            });
            // The follower is a child process; it outlives its parent unless it
            // is stopped (see Osd::on_shutdown).
            osd.on_shutdown(move || follower.stop());

            let _ = view.set(media.clone());
            Content::Single(media.root.clone().upcast::<gtk::Widget>())
        })
    };

    let handle = {
        let view = view.clone();
        let settings = settings.clone();
        Rc::new(
            move |osd: &Rc<Osd>, args: &[String]| -> Result<String, String> {
                let view = view.get().ok_or("the OSD has not finished starting up")?;
                let Some(verb) = args.first().map(String::as_str) else {
                    // No verb: started by Hyprland's `exec-once`, so just watch
                    // for something to play.
                    return Ok(String::new());
                };

                match verb {
                    "next" | "previous" | "skip" => {
                        let direction = if verb == "previous" {
                            Direction::Previous
                        } else {
                            Direction::Next
                        };
                        mpris::skip(direction, &Player::Active)?;
                        // The card follows from the metadata change that skipping
                        // causes, so nothing is shown here: a player that refuses
                        // to skip would otherwise pop up with the same track.
                        Ok(String::new())
                    }
                    "show" => {
                        let event = mpris::current()?;
                        if event.track.is_empty() {
                            return Err("nothing is loaded in the player".to_string());
                        }
                        view.render(&event.track);
                        osd.reveal(settings.duration());
                        view.arm_touch(osd);
                        Ok(String::new())
                    }
                    "status" => {
                        let event = mpris::current()?;
                        let subtitle = event.track.subtitle();
                        Ok(match (event.track.title.as_str(), subtitle.as_str()) {
                            ("", "") => "nothing is playing".to_string(),
                            (title, "") => title.to_string(),
                            ("", subtitle) => subtitle.to_string(),
                            (title, subtitle) => format!("{title} — {subtitle}"),
                        })
                    }
                    other => Err(format!(
                        "unknown command `{other}` (next | previous | show | status)"
                    )),
                }
            },
        )
    };

    run(opts, build, handle)
}

//! The volume card: speaker icon, slider, percentage.
//!
//! Layout and behaviour, in one place because they are one thing: the icon and
//! the slider show the same value the slider *sets*, so a drag has to go back to
//! the sink and the sink has to come back to the icon. `render` is the only way
//! in, `hook` is the only way out.
//!
//! # By finger
//!
//! The card is a notification: it appears for `duration_ms` and leaves by itself,
//! so a touchscreen does not need a way *in* - only a way *out* before then. On a
//! tablet it therefore asks the shell for a **tap catcher** around itself
//! ([`VolumeView::arm_touch`]): a tap anywhere else closes the card immediately,
//! and the dimmed bands make that the whole screen instead of the card's
//! neighbours. The slider itself stays a drag - GTK scales take a touch drag
//! natively - and the stylesheet makes the trough and its thumb finger-sized
//! under `.touch` (`.touch scale.volume > trough > slider`), because a 14px thumb
//! on a 4px trough is not something a fingertip can find.
//!
//! The catcher's hole is the card's rectangle on its monitor; it is measured from
//! the widget at reveal time and derived from the placement exactly as the shell
//! applies it (see [`bottom_hole`] for why the arithmetic is repeated here).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::icons::{self, names};
use hypr_osd_core::monitors;
use hypr_osd_core::{touch, Hole, Osd, CARD_PAD_X, CARD_PAD_Y, SHADOW_PAD};

use crate::sink::{self, Sink};
use crate::Settings;

/// How large the speaker is drawn. The card is the bar's volume signal one size
/// larger, so the icon is a step up from the 14px a pill draws.
const ICON: i32 = 16;

pub struct VolumeView {
    /// The card's content, handed to the shell.
    pub root: gtk::Box,
    speaker: gtk::Image,
    scale: gtk::Scale,
    value: gtk::Label,
    /// The last percentage a *drag* pushed to the sink, so sweeping the pointer
    /// across one percent does not spawn wpctl over and over.
    applied: Cell<i32>,
    max: f64,
    duration: Duration,
    unmute: bool,
    /// The card's configured width and placement, kept for the tap catcher's
    /// geometry (see [`VolumeView::arm_touch`]).
    width: i32,
    bottom_margin: f64,
}

impl VolumeView {
    pub fn new(settings: &Settings) -> Self {
        // The icon is fixed-width rather than icon-width so the slider does not
        // shift when the level moves between one third and the next: the three
        // speakers are not the same width. Its size is the pixel size, not a
        // font size - it is a drawing now, not a character.
        let speaker = icons::lucide(names::VOLUME_HIGH, ICON);
        speaker.add_css_class("speaker");
        speaker.set_valign(gtk::Align::Center);

        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, settings.max, 1.0);
        scale.add_css_class("volume");
        scale.set_hexpand(true);
        scale.set_valign(gtk::Align::Center);
        // The number is a label of our own, in the bar's type and with room for
        // "100%", so it keeps its place while the fill moves.
        scale.set_draw_value(false);
        scale.set_round_digits(0);
        scale.set_increments(settings.step, settings.step * 2.0);

        let value = gtk::Label::new(None);
        value.add_css_class("value");
        value.set_valign(gtk::Align::Center);
        value.set_xalign(1.0);
        value.set_width_chars(4);

        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.append(&speaker);
        root.append(&scale);
        root.append(&value);

        VolumeView {
            root,
            speaker,
            scale,
            value,
            applied: Cell::new(-1),
            max: settings.max,
            duration: settings.duration,
            unmute: settings.unmute_on_raise,
            width: settings.width,
            bottom_margin: settings.bottom_margin,
        }
    }

    /// Wire the slider to the sink. Dragging it *is* a volume change - same step
    /// the keys take, just continuous - and it reveals the card exactly like a
    /// key press, so the two ways in behave identically.
    pub fn hook(self: &Rc<Self>, osd: &Rc<Osd>) {
        let duration = self.duration;
        let unmute = self.unmute;

        let me = Rc::clone(self);
        let card = osd.clone();
        // `change-value` is the *user's* intent - a drag, a scroll, an arrow key
        // - and not emitted for programmatic `set_value`, which is what makes it
        // the right place for the round trip to the sink.
        self.scale
            .connect_change_value(move |_scale, _scroll, value| {
                let percent = value.round() as i32;
                if percent != me.applied.get() {
                    me.applied.set(percent);
                    match sink::set_percent(percent as f64, me.max, unmute) {
                        Ok(sink) => me.render(&sink),
                        Err(error) => eprintln!("hypr-osd-volume: {error}"),
                    }
                }
                card.reveal(duration);
                glib::Propagation::Proceed
            });

        // Keep the number in step with the fill while dragging.
        let value = self.value.clone();
        self.scale.connect_value_changed(move |scale| {
            value.set_text(&format!("{}%", scale.value().round() as i32));
        });

        // A drag can pause - a finger resting on the slider - and the card must
        // not disappear under the pointer while it does. This is the card's own
        // predicate rather than `stay_open_while_hovered` because the slider's
        // `ACTIVE` state already says exactly that, and it is true for a finger
        // and a mouse alike (`GtkScale` takes a touch drag natively).
        let scale = self.scale.clone();
        osd.set_stay_open_when(move || scale.state_flags().contains(gtk::StateFlags::ACTIVE));
    }

    /// Put the shell's tap catcher around the card while it is up on a tablet, or
    /// take it away.
    ///
    /// Called each time the card is revealed (a key press while it is already up
    /// re-reveals it) and again if the machine is folded or unfolded under it.
    /// Every way the card leaves - the `duration_ms` timer, a tap on one of the
    /// catcher's bands, the next reveal - runs the shell's `hide`, which takes the
    /// catcher with it, so there is nothing to clean up here.
    pub fn arm_touch(&self, osd: &Rc<Osd>) {
        let hide = {
            let osd = osd.clone();
            move || osd.hide()
        };
        let hole = touch::tablet()
            .then(|| monitor_hole(osd, &self.root, self.width, self.bottom_margin))
            .flatten();
        match hole {
            Some(hole) => osd.tap_catcher(Some(hole), hide),
            // Not a tablet (or no monitor to measure): the mouse card is exactly
            // what it always was.
            None => osd.tap_catcher(None, hide),
        }
    }

    /// Paint a state that came from somewhere other than the slider (a key
    /// press, a mute); whether the card then appears is the caller's decision.
    pub fn render(&self, sink: &Sink) {
        let percent = sink::percent(sink);
        self.applied.set(percent);
        self.scale.set_value(percent as f64);
        self.value.set_text(&format!("{percent}%"));
        icons::set_lucide(&self.speaker, for_level(percent, sink.muted), ICON);
        // One signal, three places: the bar paints a muted sink red, so the
        // icon goes red, and the fill follows it.
        for widget in [
            self.speaker.clone().upcast::<gtk::Widget>(),
            self.scale.clone().upcast(),
            self.root.clone().upcast(),
        ] {
            set_class(&widget, "muted", sink.muted);
        }
    }
}

// ---------------------------------------------------------------------------
// The card's rectangle on the screen
// ---------------------------------------------------------------------------

/// The card's rectangle on its monitor, in the logical pixels the catcher's
/// strips are laid out in, or `None` when there is no monitor to measure against.
fn monitor_hole(
    osd: &Osd,
    root: &impl IsA<gtk::Widget>,
    width: i32,
    bottom_margin: f64,
) -> Option<Hole> {
    let screen = monitor_size(osd)?;
    Some(bottom_hole(screen, card_size(root, width), bottom_margin))
}

/// The logical size of the monitor the card is on.
///
/// GDK's geometry is already in layout pixels - the card's own width and margins
/// are measured in the same space, so no scale factor is applied here.
fn monitor_size(osd: &Osd) -> Option<(i32, i32)> {
    let monitor = osd
        .monitor()
        .as_deref()
        .and_then(monitors::find)
        .or_else(monitors::first)?;
    let geometry = monitor.geometry();
    Some((geometry.width(), geometry.height()))
}

/// The card's size in logical pixels: the content as it will be drawn, plus the
/// shell's card padding, and never narrower than the configured width.
///
/// Measured rather than read from the card's last layout: the widget may have
/// been laid out for the pointer-sized scale before the `.touch` rules applied,
/// and the catcher must not be built around the card as it used to be. The
/// configured width is a floor, exactly as the shell applies it.
fn card_size(root: &impl IsA<gtk::Widget>, width: i32) -> (i32, i32) {
    let root = root.as_ref();
    // Measured, not read from the allocation: the widget may have been laid out
    // for the label it was showing a moment ago (a row that just turned into its
    // "click again" text), and a stale allocation would hand the catcher a hole
    // that no longer matches the card.
    let (_, natural_width, _, _) = root.measure(gtk::Orientation::Horizontal, -1);
    let (_, natural_height, _, _) = root.measure(gtk::Orientation::Vertical, natural_width);
    (
        width.max(natural_width + 2 * CARD_PAD_X),
        natural_height + 2 * CARD_PAD_Y,
    )
}

/// Where a bottom-anchored card of `card` size sits on a `screen`-sized monitor,
/// with its bottom edge `bottom_margin` of the screen's height up.
///
/// Split out from the measuring so the arithmetic can be tested without a
/// compositor. It mirrors the shell: `set_bottom_margin` keeps `SHADOW_PAD` back
/// from the transparent frame, so the *card* ends up at `screen.height *
/// bottom_margin`, and the layer surface is centred horizontally because it is
/// anchored to the bottom edge only.
fn bottom_hole(screen: (i32, i32), card: (i32, i32), bottom_margin: f64) -> Hole {
    let (screen_width, screen_height) = screen;
    let (card_width, card_height) = card;
    // The same clamp the shell applies: a margin smaller than the shadow pad
    // would put the frame off the bottom edge, so it is pinned to 0 instead.
    let margin = ((f64::from(screen_height) * bottom_margin).round() as i32 - SHADOW_PAD).max(0);
    let bottom = screen_height - margin - SHADOW_PAD;
    Hole {
        x: (screen_width - card_width) / 2,
        y: bottom - card_height,
        width: card_width,
        height: card_height,
    }
}

/// Which of the three speakers a level wears: mute (or nothing) takes the
/// crossed one whatever the number says, then the two thirds.
///
/// The three names are the collection's shared vocabulary, so this card and the
/// bar's status pill cannot disagree about what a level looks like.
fn for_level(percent: i32, muted: bool) -> &'static str {
    if muted || percent <= 33 {
        names::VOLUME_OFF
    } else if percent <= 66 {
        names::VOLUME_LOW
    } else {
        names::VOLUME_HIGH
    }
}

fn set_class(widget: &gtk::Widget, name: &str, on: bool) {
    if on {
        widget.add_css_class(name);
    } else {
        widget.remove_css_class(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This machine: 2736x1824 at scale 2 is 1368x912 logical pixels, and the
    /// default `bottom_margin = 0.12` is 109 px up from the bottom.
    #[test]
    fn a_bottom_card_sits_where_the_shell_puts_it() {
        let hole = bottom_hole((1368, 912), (340, 70), 0.12);
        assert_eq!(
            hole,
            Hole {
                x: (1368 - 340) / 2,
                y: 912 - 109 - 70,
                width: 340,
                height: 70,
            }
        );
    }

    /// A margin smaller than the transparent frame is pinned to the screen edge,
    /// exactly like `set_bottom_margin` does.
    #[test]
    fn a_tiny_margin_is_clamped_to_the_screen_edge() {
        let hole = bottom_hole((1368, 912), (340, 70), 0.0);
        assert_eq!(hole.y + hole.height, 912 - SHADOW_PAD);
    }
}

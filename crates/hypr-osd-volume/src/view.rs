//! The volume card: speaker icon, slider, percentage.
//!
//! Layout and behaviour, in one place because they are one thing: the icon and
//! the slider show the same value the slider *sets*, so a drag has to go back to
//! the sink and the sink has to come back to the icon. `render` is the only way
//! in, `hook` is the only way out.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::icons::{self, names};
use hypr_osd_core::Osd;

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

        // A drag can pause - finger resting on the slider - and the card must
        // not disappear under the pointer while it does.
        let scale = self.scale.clone();
        osd.set_stay_open_when(move || scale.state_flags().contains(gtk::StateFlags::ACTIVE));
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

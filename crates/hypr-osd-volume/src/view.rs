//! The volume card: speaker glyph, slider, percentage.
//!
//! Layout and behaviour, in one place because they are one thing: the glyph and
//! the slider show the same value the slider *sets*, so a drag has to go back to
//! the sink and the sink has to come back to the glyph. `render` is the only
//! way in, `hook` is the only way out.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::Osd;

use crate::sink::{self, Sink};
use crate::Settings;

/// The bar's own three symbols (the `format-icons` of the wireplumber module in
/// `~/.config/waybar/config.jsonc`), so the OSD and the bar never disagree
/// about what a level looks like: off/low, down, up - one per third, with mute
/// forcing the first one.
const GLYPH_OFF: &str = "\u{f026}";
const GLYPH_LOW: &str = "\u{f027}";
const GLYPH_HIGH: &str = "\u{f028}";

pub struct VolumeView {
    /// The card's content, handed to the shell.
    pub root: gtk::Box,
    glyph: gtk::Label,
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
        let glyph = gtk::Label::new(None);
        glyph.add_css_class("glyph");
        glyph.set_valign(gtk::Align::Center);

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
        root.append(&glyph);
        root.append(&scale);
        root.append(&value);

        VolumeView {
            root,
            glyph,
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
        self.glyph.set_text(glyph(percent, sink.muted));
        // One signal, three places: the bar paints a muted sink red, so the
        // glyph goes red, and the fill follows it.
        for widget in [
            self.glyph.clone().upcast::<gtk::Widget>(),
            self.scale.clone().upcast(),
            self.root.clone().upcast(),
        ] {
            set_class(&widget, "muted", sink.muted);
        }
    }
}

fn glyph(percent: i32, muted: bool) -> &'static str {
    if muted || percent <= 33 {
        GLYPH_OFF
    } else if percent <= 66 {
        GLYPH_LOW
    } else {
        GLYPH_HIGH
    }
}

fn set_class(widget: &gtk::Widget, name: &str, on: bool) {
    if on {
        widget.add_css_class(name);
    } else {
        widget.remove_css_class(name);
    }
}

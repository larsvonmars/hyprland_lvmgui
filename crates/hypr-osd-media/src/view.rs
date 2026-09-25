//! The media card: cover, title, subtitle and the two skip buttons.
//!
//! The card is a *view*, not a controller: it paints a [`Track`] and the buttons
//! ask the player to skip. What that results in comes back through the follower,
//! so a click and a track change from anywhere else end up in the same place -
//! `is_new_track` is what decides whether an event is worth showing.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use hypr_osd_core::mpris::{self, Direction, Track};
use hypr_osd_core::{Osd, CARD_PAD_X};

use crate::art;

/// The cover is a square tile: 56px, radius 12 like every other tile in the
/// theme, next to two comfortable lines of text.
const ART: i32 = 56;

/// Gap between the cover, the text and the buttons.
const GAP: i32 = 14;

/// The two skip buttons and the gap between them, pinned.
///
/// The CSS asks for 32px buttons, but what a button *is* also includes the
/// theme's own padding and the glyph's font metrics - it comes out 36px here,
/// and only once the theme has been resolved, which is later than the first
/// measurement. Pinning the pair keeps the text column below exact, so the card
/// is exactly as wide as the config asks for instead of a few pixels more.
const CONTROLS: i32 = 36 + 36 + 6;

/// The generic "something is playing" icon - the same one the bar's media pill
/// falls back to - used when a player reports no artwork, or artwork that cannot
/// be fetched.
const GLYPH_MUSIC: &str = "\u{f001}";

/// The skip keys wear step-backward/forward, which is what the bar's media pill
/// uses for them too.
const GLYPH_PREVIOUS: &str = "\u{f048}";
const GLYPH_NEXT: &str = "\u{f051}";

pub struct MediaView {
    /// The card's content, handed to the shell.
    pub root: gtk::Box,
    art: gtk::Stack,
    cover: gtk::Image,
    title: gtk::Label,
    subtitle: gtk::Label,
    previous: gtk::Button,
    next: gtk::Button,
    /// `mpris:artUrl` currently on the card, so a slow download cannot paint
    /// itself over the track that replaced it.
    art_url: RefCell<String>,
    /// The track on the card, to tell "a new medium" from the stream of
    /// events a player emits while one track loads.
    current: RefCell<Track>,
}

impl MediaView {
    pub fn new(width: i32) -> Self {
        // An image with a pixel size, not a picture: a picture never shrinks
        // below the artwork's own size, so a 500x500 cover would size the whole
        // card. `set_pixel_size` pins it to tile size whatever arrives.
        let cover = gtk::Image::new();
        cover.add_css_class("cover");
        cover.set_pixel_size(ART);
        // Together with the CSS border-radius this is what keeps the cover
        // inside its rounded tile.
        cover.set_overflow(gtk::Overflow::Hidden);

        let fallback = gtk::Label::new(Some(GLYPH_MUSIC));
        fallback.add_css_class("art-fallback");
        fallback.set_size_request(ART, ART);

        let art = gtk::Stack::new();
        art.add_named(&cover, Some("cover"));
        art.add_named(&fallback, Some("fallback"));
        art.set_visible_child_name("fallback");
        art.set_valign(gtk::Align::Center);

        let title = gtk::Label::new(None);
        title.add_css_class("title");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let subtitle = gtk::Label::new(None);
        subtitle.add_css_class("subtitle");
        subtitle.set_xalign(0.0);
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        text.append(&title);
        text.append(&subtitle);

        let previous = control(GLYPH_PREVIOUS, "Previous track");
        let next = control(GLYPH_NEXT, "Next track");
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        controls.set_valign(gtk::Align::Center);
        controls.set_size_request(CONTROLS, -1);
        controls.append(&previous);
        controls.append(&next);

        // The text column gets whatever the card has left after the cover, the
        // two gaps, the buttons and the card's own padding - a fixed width, so a
        // long title ellipsises instead of growing the card (an OSD that changes
        // width from track to track moves every pixel of itself with it).
        let room = (width - (ART + 2 * GAP + CONTROLS + 2 * CARD_PAD_X)).max(ART);
        text.set_size_request(room, -1);
        cap_width(&title, room);
        cap_width(&subtitle, room);

        let root = gtk::Box::new(gtk::Orientation::Horizontal, GAP);
        root.append(&art);
        root.append(&text);
        root.append(&controls);

        MediaView {
            root,
            art,
            cover,
            title,
            subtitle,
            previous,
            next,
            art_url: RefCell::new(String::new()),
            current: RefCell::new(Track::default()),
        }
    }

    /// Wire the card to the world: the buttons skip, and hovering holds it open.
    pub fn hook(self: &Rc<Self>, osd: &Rc<Osd>) {
        let previous = self.previous.clone();
        previous.connect_clicked(move |_| skip(Direction::Previous));
        let next = self.next.clone();
        next.connect_clicked(move |_| skip(Direction::Next));

        // A card with buttons must not vanish while you aim at one.
        osd.stay_open_while_hovered(&self.root);
    }

    /// Whether `track` is a different medium than the one on the card, recording
    /// it when it is.
    ///
    /// A player announces one track with several events: the metadata, then the
    /// status flipping to playing, sometimes the artwork later. Only the first
    /// of them is "a new medium starting to be played", and comparing whole
    /// tracks (artwork included) is what tells them apart - while a player that
    /// simply pauses and resumes stays quiet.
    pub fn is_new_track(&self, track: &Track) -> bool {
        let mut current = self.current.borrow_mut();
        if *current == *track {
            return false;
        }
        *current = track.clone();
        true
    }

    /// Paint a track: text now, cover as soon as it is readable.
    pub fn render(self: &Rc<Self>, track: &Track) {
        self.title.set_text(&track.title);
        let subtitle = track.subtitle();
        self.subtitle.set_text(&subtitle);
        self.subtitle.set_visible(!subtitle.is_empty());

        let requested = track.art.clone();
        if *self.art_url.borrow() == requested {
            return;
        }
        *self.art_url.borrow_mut() = requested.clone();

        // A new track starts on the glyph: leaving the previous cover up until
        // the new one arrives would misrepresent what is playing.
        self.art.set_visible_child_name("fallback");
        if requested.is_empty() {
            return;
        }

        let me = Rc::clone(self);
        let expected = requested.clone();
        art::load(&requested, move |texture| {
            // The cover only lands if it is still the track on the card.
            if *me.art_url.borrow() != expected {
                return;
            }
            match texture {
                Some(texture) => {
                    me.cover.set_paintable(Some(&texture));
                    me.art.set_visible_child_name("cover");
                }
                None => me.art.set_visible_child_name("fallback"),
            }
        });
    }
}

fn control(glyph: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::with_label(glyph);
    button.add_css_class("control");
    button.set_tooltip_text(Some(tooltip));
    button.set_valign(gtk::Align::Center);
    button
}

/// Cap a label's *natural* width at `pixels`.
///
/// GtkLabel has no pixel cap: `max-width-chars` is the only knob and it counts
/// characters, so the width of one character is measured in the label's own
/// font. Measuring a sample rather than asking Pango for its average character
/// width matters - that average is an estimate, and the character count derived
/// from it overshoots, which is how a card ends up wider than the width in the
/// config. Without any cap at all, a 60-character title would widen the card
/// instead of ellipsising inside it.
fn cap_width(label: &gtk::Label, pixels: i32) {
    const SAMPLE: &str = "0000000000";
    let (sample_width, _) = label.create_pango_layout(Some(SAMPLE)).pixel_size();
    let char_width = f64::from(sample_width) / SAMPLE.chars().count() as f64;
    if char_width <= 0.0 {
        return;
    }
    label.set_max_width_chars((f64::from(pixels) / char_width).floor().max(1.0) as i32);
}

fn skip(direction: Direction) {
    if let Err(error) = mpris::skip(direction) {
        eprintln!("hypr-osd-media: {error}");
    }
}

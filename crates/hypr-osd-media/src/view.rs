//! The media card: cover, title, subtitle and the two skip buttons.
//!
//! The card is a *view*, not a controller: it paints a [`Track`] and the buttons
//! ask the player to skip. What that results in comes back through the follower,
//! so a click and a track change from anywhere else end up in the same place -
//! `is_new_track` is what decides whether an event is worth showing.
//!
//! # By finger
//!
//! The two skip buttons are drawn as icons and explained by a tooltip, and a
//! touchscreen has neither a cursor to hover nor a right button to ask with - so
//! on a tablet the card grows a line under it that says what the two arrows do
//! and how to get rid of the card, and the buttons themselves are the shared
//! `button.control` chip grown to a 44px target by the stylesheet (`.touch
//! button.control`). The card is a notification - it appears by itself when a
//! track starts - so the way *in* is untouched; what a tablet needs is a way
//! *out* before the duration runs out, and that is the shell's tap catcher around
//! the card ([`MediaView::arm_touch`]): a tap anywhere else closes it.
//!
//! Growing the controls changes how much room the title column has, and the card
//! is exactly as wide as the config asked - so [`MediaView::apply_touch`]
//! recomputes that split rather than letting the card widen from track to track
//! (or from laptop to tablet). It is called when the widgets are built and again
//! whenever the machine is folded or unfolded.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use hypr_osd_core::icons::{self, names};
use hypr_osd_core::monitors;
use hypr_osd_core::mpris::{self, Direction, Player, Track};
use hypr_osd_core::{art, touch, Hole, Osd, CARD_PAD_X, CARD_PAD_Y, SHADOW_PAD};

/// The cover is a square tile: 56px, radius 12 like every other tile in the
/// theme, next to two comfortable lines of text.
const ART: i32 = 56;

/// Gap between the cover, the text and the buttons - and between the card's row
/// and its touch hint under it.
const GAP: i32 = 14;
const HINT_GAP: i32 = 6;

/// The two skip buttons and the gap between them, pinned.
///
/// The buttons are 32px - that is what the CSS asks for and, with an icon in
/// them instead of a text glyph, what they measure: a button's size used to
/// follow the font metrics of the character in it, which is why this constant
/// was 36+36+6. Pinning the pair keeps the text column below exact, so the card
/// is exactly as wide as the config asks for instead of a few pixels more.
const CONTROLS: i32 = 32 + 32 + 6;

/// The same pair on a tablet: `button.control` becomes a 44px target under
/// `.touch` (the platform's fingertip size, see `touch::TARGET`), so the text
/// column gives up the difference instead of widening the card - which is also
/// why the card's own content width is pinned rather than left to wrap.
const CONTROL_TOUCH: i32 = 44;
const CONTROLS_TOUCH: i32 = CONTROL_TOUCH + CONTROL_TOUCH + 6;

/// How large the icons are drawn: the two buttons, and the stand-in on the
/// cover's tile when a player reports no artwork (or artwork we could not
/// fetch).
const CONTROL_ICON: i32 = 16;
const ART_ICON: i32 = 26;

/// The line under the card on a tablet: the two buttons are icons, and an icon
/// with no tooltip is a picture - this says what they do and how the card leaves.
const TOUCH_HINT: &str = "Skip back · skip forward · tap outside to close";

pub struct MediaView {
    /// The card's content, handed to the shell.
    pub root: gtk::Box,
    art: gtk::Stack,
    cover: gtk::Image,
    /// The cover/title/buttons row, inside the card's root so the touch hint can
    /// sit under it.
    row: gtk::Box,
    text: gtk::Box,
    /// The two buttons' row: pinned to the width [`apply_touch`](Self::apply_touch)
    /// computes, so the card cannot grow when the stylesheet grows the buttons.
    controls: gtk::Box,
    title: gtk::Label,
    subtitle: gtk::Label,
    hint: gtk::Label,
    previous: gtk::Button,
    next: gtk::Button,
    /// The card's configured width and placement, kept for the touch split and
    /// the tap catcher's geometry.
    width: i32,
    bottom_margin: f64,
    /// `mpris:artUrl` currently on the card, so a slow download cannot paint
    /// itself over the track that replaced it.
    art_url: RefCell<String>,
    /// The track on the card, to tell "a new medium" from the stream of
    /// events a player emits while one track loads.
    current: RefCell<Track>,
}

impl MediaView {
    pub fn new(width: i32, bottom_margin: f64) -> Self {
        // An image with a pixel size, not a picture: a picture never shrinks
        // below the artwork's own size, so a 500x500 cover would size the whole
        // card. `set_pixel_size` pins it to tile size whatever arrives.
        let cover = gtk::Image::new();
        cover.add_css_class("cover");
        cover.set_pixel_size(ART);
        // Together with the CSS border-radius this is what keeps the cover
        // inside its rounded tile.
        cover.set_overflow(gtk::Overflow::Hidden);

        // The stand-in for artwork that is not there (or not fetchable): the
        // same "something is playing" icon the bar's media pill falls back to,
        // in the tile the cover would have filled.
        let fallback = icons::lucide(names::MUSIC, ART_ICON);
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

        let previous = control(names::SKIP_BACK, "Previous track");
        let next = control(names::SKIP_FORWARD, "Next track");
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        controls.set_valign(gtk::Align::Center);
        controls.append(&previous);
        controls.append(&next);

        // The hint under the row is only shown on a tablet: on a laptop the
        // tooltips do the explaining, and a card that is gone in four seconds
        // should not spend one of them on a line of text.
        let hint = gtk::Label::new(Some(TOUCH_HINT));
        hint.add_css_class("hint");
        hint.set_xalign(0.0);
        hint.set_ellipsize(gtk::pango::EllipsizeMode::End);
        hint.set_visible(false);
        cap_width(&hint, width - 2 * CARD_PAD_X);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, GAP);
        row.append(&art);
        row.append(&text);
        row.append(&controls);

        let root = gtk::Box::new(gtk::Orientation::Vertical, HINT_GAP);
        root.append(&row);
        root.append(&hint);

        MediaView {
            root,
            art,
            cover,
            row,
            text,
            controls,
            title,
            subtitle,
            hint,
            previous,
            next,
            width,
            bottom_margin,
            art_url: RefCell::new(String::new()),
            current: RefCell::new(Track::default()),
        }
    }

    /// Split the card's width between the text column and everything around it,
    /// for the mode the machine is in right now.
    ///
    /// The text column gets whatever the card has left after the cover, the two
    /// gaps, the buttons and the card's own padding - a fixed width, so a long
    /// title ellipsises instead of growing the card (an OSD that changes width
    /// from track to track moves every pixel of itself with it). Only the
    /// buttons' size changes with the mode, so this is where the split is
    /// recomputed: a 44px target is 12px wider than a 32px one, twice over, and
    /// the text gives exactly that up.
    pub fn apply_touch(&self, tablet: bool) {
        let controls = if tablet { CONTROLS_TOUCH } else { CONTROLS };
        self.controls.set_size_request(controls, -1);
        let room = (self.width - (ART + 2 * GAP + controls + 2 * CARD_PAD_X)).max(ART);
        self.text.set_size_request(room, -1);
        self.row.set_size_request(self.width - 2 * CARD_PAD_X, -1);
        cap_width(&self.title, room);
        cap_width(&self.subtitle, room);
        self.hint.set_visible(tablet);
    }

    /// Put the shell's tap catcher around the card while it is up on a tablet, or
    /// take it away.
    ///
    /// Called each time the card is revealed - it can appear by itself when a
    /// track starts, or be re-revealed by `show` - and again if the machine is
    /// folded or unfolded under it. Every close path runs the shell's `hide`,
    /// which takes the catcher with it.
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

    /// Wire the card to the world: the buttons skip, and pressing it holds it
    /// open.
    pub fn hook(self: &Rc<Self>, osd: &Rc<Osd>) {
        let previous = self.previous.clone();
        previous.connect_clicked(move |_| skip(Direction::Previous));
        let next = self.next.clone();
        next.connect_clicked(move |_| skip(Direction::Next));

        // A card with buttons must not vanish while you aim at one - and on a
        // tablet, while the finger is still down on it.
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
/// GDK's geometry is already in layout pixels, which is the space the card's own
/// width and margins are measured in, so no scale factor is applied.
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
/// Measured rather than read from the card's last layout: a track without a
/// subtitle is a shorter card than the one before it, and the catcher's hole has
/// to follow the card that is on screen now. The configured width is a floor,
/// exactly as the shell applies it.
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

/// One of the two skip buttons: the icon, a chip shape from the stylesheet, and
/// the tooltip that says what it does - the icon alone is a picture, and a
/// picture is not a label. (On a tablet the tooltip cannot be reached, which is
/// what the hint line under the card is for.)
fn control(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_child(Some(&icons::lucide(icon, CONTROL_ICON)));
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
    // The card watches whatever is playing and never chooses a player: it has no
    // buttons for that, so its skips go to the same one the media keys reach.
    if let Err(error) = mpris::skip(direction, &Player::Active) {
        eprintln!("hypr-osd-media: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This machine: 2736x1824 at scale 2 is 1368x912 logical pixels, and the
    /// default `bottom_margin = 0.12` is 109 px up from the bottom. The card is
    /// the configured 400px wide and 70px tall on a laptop.
    #[test]
    fn a_bottom_card_sits_where_the_shell_puts_it() {
        let hole = bottom_hole((1368, 912), (400, 70), 0.12);
        assert_eq!(
            hole,
            Hole {
                x: (1368 - 400) / 2,
                y: 912 - 109 - 70,
                width: 400,
                height: 70,
            }
        );
    }

    /// A margin smaller than the transparent frame is pinned to the screen edge,
    /// exactly like `set_bottom_margin` does.
    #[test]
    fn a_tiny_margin_is_clamped_to_the_screen_edge() {
        let hole = bottom_hole((1368, 912), (400, 70), 0.0);
        assert_eq!(hole.y + hole.height, 912 - SHADOW_PAD);
    }

    /// The touch split is what keeps a 44px-button card the same width as the
    /// configured one: the text column gives the 24px the two buttons grow by.
    #[test]
    fn the_touch_controls_are_paid_for_out_of_the_text_column() {
        let width = 400;
        let pointer = (width - (ART + 2 * GAP + CONTROLS + 2 * CARD_PAD_X)).max(ART);
        let touch = (width - (ART + 2 * GAP + CONTROLS_TOUCH + 2 * CARD_PAD_X)).max(ART);
        assert_eq!(pointer - touch, CONTROLS_TOUCH - CONTROLS);
        assert!(touch > 0);
        // A finger target is the shared one, not a number made up here.
        assert_eq!(CONTROL_TOUCH, touch::TARGET);
    }
}

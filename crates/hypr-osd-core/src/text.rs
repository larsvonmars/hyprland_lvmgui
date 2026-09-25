//! The label helpers the tile views share.
//!
//! Both elements that draw windows put text under a tile and, when nothing
//! better can be drawn, a letter *in* one. Neither has anything to do with the
//! window list, and both elements would otherwise grow their own copy - which is
//! how two cards end up with two different ideas of what a stand-in looks like.

use gtk::prelude::*;

/// The first letter of a class, for the tile that stands in for a picture that
/// could not be had: better than a generic glyph, because it still says *which*
/// application this is.
pub fn initial(class: &str) -> String {
    class
        .chars()
        .find(|character| character.is_alphanumeric())
        .map(|character| character.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string())
}

/// Cap a label so that it cannot widen the card it sits in.
///
/// An ellipsised label still reports its *full* text as its natural width, so
/// without this one long window title would decide how wide the whole card is.
/// The media card learned this lesson first, for the same reason.
pub fn cap_width(label: &gtk::Label, pixels: i32) {
    const SAMPLE: &str = "0000000000";
    let (sample_width, _) = label.create_pango_layout(Some(SAMPLE)).pixel_size();
    let char_width = f64::from(sample_width) / SAMPLE.chars().count() as f64;
    if char_width <= 0.0 {
        return;
    }
    label.set_max_width_chars((f64::from(pixels) / char_width).floor().max(1.0) as i32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stand_in_letter_is_the_first_one_there_is() {
        assert_eq!(initial("firefox"), "F");
        // Classes are not always words, and not always lowercase.
        assert_eq!(initial("0ad"), "0");
        assert_eq!(initial("org.gnome.Nautilus"), "O");
        // A class that is empty (or has no letters at all) still gets something:
        // an empty tile reads as a bug.
        assert_eq!(initial(""), "?");
        assert_eq!(initial("---"), "?");
    }
}

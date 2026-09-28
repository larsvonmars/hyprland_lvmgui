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

/// Cap a label so that it cannot widen the card it sits in, and answer with the
/// number of characters it settled on.
///
/// An ellipsised label still reports its *full* text as its natural width, so
/// without this one long window title would decide how wide the whole card is.
/// The media card learned this lesson first, for the same reason.
///
/// The count comes back so that a **wrapping** label can be pinned from both
/// sides: the application panel's tiles wrap a long name over two lines, and a
/// wrapping label's natural width is the unwrapped text - `max_width_chars`
/// bounds the ask but does not give Pango a width to wrap at. Handing the same
/// count to `set_width_chars` does, which is what keeps a tile the size the grid
/// planned for.
pub fn cap_width(label: &gtk::Label, pixels: i32) -> i32 {
    const SAMPLE: &str = "0000000000";
    let (sample_width, _) = label.create_pango_layout(Some(SAMPLE)).pixel_size();
    let char_width = f64::from(sample_width) / SAMPLE.chars().count() as f64;
    if char_width <= 0.0 {
        return 0;
    }
    let chars = (f64::from(pixels) / char_width).floor().max(1.0) as i32;
    label.set_max_width_chars(chars);
    chars
}

/// Split a name over at most two lines, each within `capacity` characters.
///
/// The applications panel draws a tile's name over two lines, and it has to do the
/// breaking here rather than with `Label::set_wrap`: a *wrapping* label measures
/// its natural width from the whole unwrapped text, so one long application name
/// widened its tile - and with it the grid, the scroller and the card (measured:
/// 617px of tile pane where 550 was planned). Explicit line breaks *are* measured
/// line by line, so a name broken here cannot do that.
///
/// The break is at the last space that keeps the first line inside the budget;
/// failing that the line is cut where the budget ends and both lines are
/// ellipsised, so what comes back is never wider than the caller asked for.
pub fn two_lines(name: &str, capacity: usize) -> String {
    let capacity = capacity.max(5);
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= capacity {
        return name.to_string();
    }
    // A space *at* the budget is a valid break - it leaves the first line exactly
    // as wide as it is allowed to be - which is why the search reaches one past it.
    let search = (capacity + 1).min(chars.len());
    match chars[..search]
        .iter()
        .rposition(|character| character.is_whitespace())
    {
        Some(index) if index > 0 => format!(
            "{}\n{}",
            fit(&chars[..index], capacity),
            fit(&chars[index + 1..], capacity)
        ),
        _ => format!(
            "{}\n{}",
            fit(&chars, capacity),
            fit(&chars[capacity..], capacity)
        ),
    }
}

/// One line of at most `capacity` characters, ellipsised if it had to be cut.
fn fit(chars: &[char], capacity: usize) -> String {
    if chars.len() <= capacity {
        return chars.iter().collect();
    }
    let mut line: String = chars[..capacity - 1].iter().collect();
    line.push('\u{2026}');
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_name_is_left_alone() {
        assert_eq!(two_lines("Firefox", 14), "Firefox");
        // Exactly at the budget is still one line.
        assert_eq!(two_lines("12345678901234", 14), "12345678901234");
    }

    #[test]
    fn a_long_name_breaks_at_a_space() {
        // The break is the *last* space that fits, so both lines are used evenly.
        assert_eq!(
            two_lines("CachyOS Kernel Manager", 14),
            "CachyOS Kernel\nManager"
        );
        assert_eq!(
            two_lines("Avahi Zeroconf Browser", 14),
            "Avahi Zeroconf\nBrowser"
        );
    }

    #[test]
    fn a_name_with_no_useful_space_is_cut_and_ellipsised() {
        // No space at all within the budget, so both lines are cut mid-word - and
        // the two of them still spell the name from its start, one after the other.
        assert_eq!(
            two_lines("Betterbird-E-Mail und -Nachrichten", 14),
            "Betterbird-E-\u{2026}\nail und -Nach\u{2026}"
        );
    }

    #[test]
    fn no_line_is_ever_wider_than_the_budget() {
        for name in [
            "Firefox",
            "CachyOS Kernel Manager",
            "Betterbird-E-Mail und -Nachrichten",
            "Supercalifragilisticexpialidocious",
            "a b c d e f g h i j k l m n o p",
        ] {
            let text = two_lines(name, 12);
            for line in text.lines() {
                assert!(
                    line.chars().count() <= 12,
                    "{line:?} is longer than the budget"
                );
            }
        }
    }

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

//! Window thumbnails, captured by `grim -T`.
//!
//! Hyprland implements the two protocols this needs - a list of foreign
//! toplevels, and image capture *per toplevel* - and `hyprctl -j clients` hands
//! out the `stableId` that names one. That pair is what makes a real overview
//! possible: the capture is the window's own buffer, so it is complete even when
//! the window sits behind another one, and it exists even when the window is on
//! a workspace that is not on screen at all - which is where most of the windows
//! in a workspace overview are.
//!
//! The alternative would be a screenshot of the screen and a crop per window,
//! which is what an overview has to do on compositors with no such protocol. It
//! shows whatever is *on top* of the window you wanted, and nothing at all for a
//! window on another workspace.
//!
//! Three choices are worth knowing about:
//!
//! * **JPEG, not PNG.** The capture is thrown away and taken again every time
//!   the card opens, and a JPEG of a screen full of text is a third of the size
//!   and three times faster to encode. At tile size the artefacts are invisible.
//! * **Scaled down by grim itself** (`-s`), to a little more than the tile that
//!   will hold it. Capturing at native size would be sharpest, but a texture
//!   costs four bytes per pixel for as long as the card is up, and a dozen
//!   full-size windows is a couple of hundred megabytes of them.
//! * **`-s` is relative to the monitor's scale factor**, not to the window: grim
//!   hands over the window's *buffer*, which on a 2x screen has twice the pixels
//!   the window has layout pixels. The factor therefore has to be computed from
//!   both, which is what [`factor`] does.
//!
//! What the capture does *not* contain is the compositor's own decoration:
//! Hyprland draws the border, the rounding and the shadow around the client's
//! buffer, so the tile has to draw its own frame. That is what the CSS is for.
//!
//! A capture that is still running when the card closes finishes anyway, into a
//! tile nobody is looking at. That costs a tenth of a second of a background
//! process and nothing else, which is cheaper than the bookkeeping it would take
//! to cancel one.

use gtk::gdk;
use gtk::glib;
use hypr_osd_core::output;
use hypr_osd_core::windows::Window;

/// JPEG quality. Thumbnails are small, and the artefacts of a slightly lower
/// quality are invisible at tile size - while the file gets smaller and the
/// encode faster.
const QUALITY: u32 = 85;

/// Detail asked for, relative to the tile that will hold the picture: two device
/// pixels per layout pixel, so a thumbnail stays crisp on a HiDPI screen.
const DETAIL: f64 = 2.0;

/// Never ask grim to scale by less than this. It resamples with a filter of its
/// own rather than by dropping pixels, but a factor this small is already four
/// times below the window's logical size; below it, a tile would only get softer
/// while saving nothing anybody would notice.
const MIN_FACTOR: f64 = 0.25;

/// Never ask for more pixels than the window has.
const MAX_FACTOR: f64 = 1.0;

/// Capture `window` and hand the picture to `on_done` - on the main loop, once
/// the compositor and grim are done with it.
///
/// `scale` is the monitor's scale factor and `tile_width` the width of the tile
/// the picture is for; together they decide how much detail to ask for (see
/// [`factor`]). `on_done` gets `None` when there is nothing to show: a window
/// Hyprland gave no capture id (or that is gone already), grim not installed, or
/// an answer that does not decode. The tile keeps its placeholder then.
pub fn capture(
    window: &Window,
    scale: f64,
    tile_width: i32,
    on_done: impl FnOnce(Option<gdk::Texture>) + 'static,
) {
    if window.stable_id.is_empty() {
        on_done(None);
        return;
    }
    let args = vec![
        // By toplevel, i.e. the window's own buffer rather than the screen.
        "-T".to_string(),
        window.stable_id.clone(),
        "-s".to_string(),
        format!("{}", factor(window, scale, tile_width)),
        "-t".to_string(),
        "jpeg".to_string(),
        "-q".to_string(),
        QUALITY.to_string(),
        // `-` is stdout: no temporary file to write, read and clean up.
        "-".to_string(),
    ];
    output::read("grim", &args, move |data| {
        on_done(data.and_then(|data| decode(&data)));
    });
}

/// The bytes grim wrote, as a picture GTK can draw.
fn decode(data: &[u8]) -> Option<gdk::Texture> {
    gdk::Texture::from_bytes(&glib::Bytes::from(data)).ok()
}

/// How far to scale the capture down, as grim's `-s` factor.
///
/// grim's factor multiplies the *monitor* scale factor, so what a window has
/// naturally is its layout width times the screen's scale: 1324 layout pixels is
/// a 2648 pixel buffer on a 2x screen, and a factor of 0.5 hands over exactly
/// the logical size. The wanted width comes from the tile: a little more than it
/// can show, so that scaling down to the tile stays sharp.
fn factor(window: &Window, scale: f64, tile_width: i32) -> f64 {
    let natural = f64::from(window.size.0) * scale;
    if natural <= 0.0 {
        // No size to reason from (a window that is still starting up): ask for
        // the window's own buffer and let the tile scale it.
        return MAX_FACTOR;
    }
    let wanted = f64::from(tile_width) * DETAIL;
    (wanted / natural).clamp(MIN_FACTOR, MAX_FACTOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(size: (i32, i32)) -> Window {
        Window {
            address: "0x559e6cdb4a30".to_string(),
            class: "firefox".to_string(),
            title: "docs".to_string(),
            workspace: "3".to_string(),
            stable_id: "1800000b".to_string(),
            size,
        }
    }

    #[test]
    fn a_big_window_is_scaled_down_and_a_small_one_is_not_touched() {
        // A 1324x820 window on a 2x screen is a 2648 pixel buffer; a 260 pixel
        // tile wants 520 of them, which is below the floor this asks for, so the
        // floor wins - a quarter of the buffer.
        assert_eq!(factor(&window((1324, 820)), 2.0, 260), MIN_FACTOR);
        // A small window: 400 layout pixels is an 800 pixel buffer, and 520 of
        // them is a reasonable fraction of that - no need to squeeze it.
        assert!((factor(&window((400, 300)), 2.0, 260) - 0.65).abs() < 1e-9);
        // A window with nothing to scale down: asking for more than it has would
        // only make a blurry picture bigger.
        assert_eq!(factor(&window((120, 90)), 2.0, 260), MAX_FACTOR);
    }

    #[test]
    fn a_window_with_no_size_is_taken_as_it_is() {
        assert_eq!(factor(&window((0, 0)), 2.0, 260), MAX_FACTOR);
    }

    #[test]
    fn a_bigger_tile_asks_for_more_detail() {
        let small = factor(&window((800, 600)), 1.0, 200);
        let large = factor(&window((800, 600)), 1.0, 400);
        assert!(large > small, "{large} should ask for more than {small}");
    }
}

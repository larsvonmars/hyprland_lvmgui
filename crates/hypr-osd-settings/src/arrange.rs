//! The Display page's arrangement editor: the connected outputs drawn to scale,
//! dragged into their places, and clicked for their settings.
//!
//! This is the window's one *gesture* control. A monitor's position is a pair of
//! numbers (`monitor:eDP-1:position = 1903x1440`), and the only honest way to
//! choose them is to see the screens beside one another at their relative sizes
//! and move one - the picture every desktop system offers, because a hand-typed
//! pair of numbers is a puzzle rather than a decision.
//!
//! The things that make the picture trustworthy:
//!
//! * **The boxes are layout pixels.** A monitor reports its resolution in physical
//!   pixels and its scale separately, while a position is measured in layout
//!   pixels (see [`monitors::Display::logical_size`]). A 2x screen is therefore
//!   half as wide here as its resolution suggests - which is also how Windows and
//!   macOS draw their arrangements, and what keeps a HiDPI panel from looking
//!   twice the desk it really is.
//! * **Nothing is cropped.** The whole layout is fitted into the canvas, so an
//!   output far to the left (a negative position) is still on the picture, and an
//!   arrangement that grows sideways simply gets smaller rather than falling off
//!   the edge.
//! * **A drop snaps.** A screen dropped four pixels from another one's edge is
//!   meant to be *on* it: Hyprland would otherwise leave a sliver of desktop
//!   between them that the pointer cannot cross. Edges snap to edges - and to the
//!   layout's origin - the way the system arrangements do it.
//! * **The landing spot is always visible.** The dragged box simply follows the
//!   pointer, while a dashed ghost sits on the cell the drop would settle into -
//!   for a nudge and for a stack alike - and a hairline guide marks the edge,
//!   the centre or the origin axis the snap answered to. The preview and the
//!   write are the same [`placed`] call, so the two cannot disagree.
//! * **A click chooses.** A press that never moved toggles the box's `selected`
//!   mark and reports it, so the element can hang the per-output editor (mode,
//!   scale, rotation, mirror) off the box as a popover. A drag chooses its box
//!   too - but quietly, without opening anything.
//! * **A finger works it too.** A box is only as big as its screen's share of
//!   the picture, and a stacked layout can draw one smaller than a fingertip -
//!   so the *hit* area (never the drawn rectangle, which is the information) is
//!   grown to [`touch::TARGET`] when the press came from a finger, and the
//!   nearest box's centre wins where two grown areas overlap. The mouse keeps
//!   the exact rectangle, because a pointer can be aimed. A drag also reports
//!   itself as activity, so the element's idle-close watchdog cannot take the
//!   window away while one is in progress.
//!
//! The editor is *persistent*: the element keeps the [`Arranger`] across page
//! re-renders and calls [`Arranger::update`] with the compositor's fresh answer,
//! which re-fits the boxes in place - a new mode or scale re-sizes a box, a drop
//! re-places it - instead of rebuilding the canvas and asking `hyprctl` again.
//!
//! The widget knows nothing about config files: it answers with the connector and
//! the position a screen was dropped at, or the box that was clicked, and the
//! element writes it (see `Panel::arrangement`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use hypr_osd_core::monitors;
use hypr_osd_core::text;
use hypr_osd_core::touch;

/// The air a screen keeps from the canvas edge.
const PAD: i32 = 10;
/// The tallest the canvas may get, so the picture cannot push the rows below it
/// out of the window - and the shortest, so a single small screen still reads as
/// a rectangle rather than as a letterbox.
const MAX_HEIGHT: i32 = 200;
/// A layout that is taller than it is wide - screens stacked above one another -
/// gets more room than that. Fitting a stack into the height of a wide canvas
/// would shrink both screens to a stripe the moment the second one went
/// underneath the first, which is exactly the arrangement this editor has to
/// make easy.
const MAX_TALL_HEIGHT: i32 = 300;
const MIN_HEIGHT: i32 = 88;
/// The gap two neighbouring screens keep, so their borders can be told apart
/// while their *positions* still touch exactly.
const GAP: i32 = 2;
/// How close (in canvas pixels) a dropped edge has to come before it snaps.
const SNAP: f64 = 9.0;
/// How far (in canvas pixels) the pointer has to travel along the drag's axis
/// before the stack is *meant*. A short drag still nudges a screen; a deliberate
/// one places it above, below or beside its neighbour.
const STACK_TRAVEL: f64 = 24.0;

/// How a drop is reported: the connector, and the position it landed on.
type MoveHandler = Rc<dyn Fn(&str, &str)>;
/// How a click is reported: the connector, and its box - so the element can
/// hang the per-output editor off it as a popover.
type SelectHandler = Rc<dyn Fn(&str, &gtk::Widget)>;

/// What the editor tells its element: where a drop landed, which box a click
/// chose, and that the user is still working the picture.
pub struct Handlers {
    /// A drop: the connector, and the position it landed on (`1903x1440`).
    pub on_move: MoveHandler,
    /// A click: the connector, and its box.
    pub on_select: SelectHandler,
    /// Somebody pressed, dragged or let go of the canvas. The element re-arms
    /// its idle-close watchdog with it: a drag is the longest a finger ever
    /// stays on this window without a key or a scroll.
    pub on_activity: Rc<dyn Fn()>,
}

/// A rectangle in layout coordinates: where an output sits and how large it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Rect {
    fn right(&self) -> i32 {
        self.x + self.w
    }

    fn bottom(&self) -> i32 {
        self.y + self.h
    }
}

/// The smallest rectangle that holds every one of `rects` - what the canvas is
/// fitted to, so the picture is the whole layout rather than one screen's guess.
fn extent(rects: &[Rect]) -> Option<Rect> {
    let first = rects.first()?;
    let mut left = first.x;
    let mut top = first.y;
    let mut right = first.right();
    let mut bottom = first.bottom();
    for rect in &rects[1..] {
        left = left.min(rect.x);
        top = top.min(rect.y);
        right = right.max(rect.right());
        bottom = bottom.max(rect.bottom());
    }
    Some(Rect {
        x: left,
        y: top,
        w: (right - left).max(1),
        h: (bottom - top).max(1),
    })
}

/// Where a snap landed: the position, and the alignment lines it snapped to.
///
/// The guides are layout coordinates of *lines* - the shared edge, the
/// adjacency, the neighbour's centre, or an origin axis - which the drag draws
/// across the canvas while the ghost sits on them. `None` on an axis means the
/// drop kept the position it was dragged to there, so no line would be honest.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Snapped {
    x: i32,
    y: i32,
    guide_x: Option<i32>,
    guide_y: Option<i32>,
}

/// The whole decision a drop makes: the rectangle it settles into, and the
/// guide lines the preview draws while the box is on its way there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Placed {
    rect: Rect,
    guide_x: Option<i32>,
    guide_y: Option<i32>,
}

/// Where a dropped output really goes: the nearest edge it was close to, in each
/// axis, if any was close enough - and which line that edge is.
///
/// The candidates are the ones that make two screens *flush* - sharing a top, a
/// bottom, a left or a right edge, or sitting exactly beside one another - plus
/// the origin, which is where a layout usually starts. `threshold` is in layout
/// pixels (the caller converts the canvas's pixels), so the snap feels the same
/// at every zoom.
fn snap(rect: Rect, others: &[Rect], threshold: i32) -> Snapped {
    // The rectangle's own value is deliberately *not* in these lists: it would
    // always be a distance of zero and so always win. It is the fallback instead
    // (`unwrap_or`), which is exactly the right answer when nothing is close -
    // and a fallback has no line to draw, so its guide is `None`.
    //
    // Every candidate carries the line it aligns *with*: a shared edge is the
    // edge itself, sitting beside a screen is the edge it sits against, a
    // centre is the other screen's middle line, and the origin's lines are the
    // axes themselves.
    let mut xs = vec![(0, 0)];
    let mut ys = vec![(0, 0)];
    for other in others {
        xs.push((other.x, other.x));
        xs.push((other.right() - rect.w, other.right()));
        xs.push((other.right(), other.right()));
        xs.push((other.x - rect.w, other.x));
        ys.push((other.y, other.y));
        ys.push((other.bottom() - rect.h, other.bottom()));
        ys.push((other.bottom(), other.bottom()));
        ys.push((other.y - rect.h, other.y));
        // The middles as well: stacking one screen above or below another is
        // where "lined up" means *centred*, and the two sizes differ on almost
        // every real pair - a small laptop under a large monitor, say.
        xs.push((
            other.x + (other.w - rect.w).div_euclid(2),
            other.x + other.w.div_euclid(2),
        ));
        ys.push((
            other.y + (other.h - rect.h).div_euclid(2),
            other.y + other.h.div_euclid(2),
        ));
    }
    // On a tie the first candidate wins, which is why the origin leads: an
    // arrangement that is already at `0` snaps to itself rather than being
    // dragged onto a neighbour that happens to be as close.
    let nearest = |candidates: &[(i32, i32)], value: i32| {
        candidates
            .iter()
            .copied()
            .filter(|(candidate, _)| (candidate - value).abs() <= threshold)
            .min_by_key(|(candidate, _)| (candidate - value).abs())
            .map(|(candidate, guide)| (candidate, Some(guide)))
            .unwrap_or((value, None))
    };
    let (x, guide_x) = nearest(&xs, rect.x);
    let (y, guide_y) = nearest(&ys, rect.y);
    Snapped {
        x,
        y,
        guide_x,
        guide_y,
    }
}

/// Which way a drag went, for a screen that has to be pushed clear of another.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Axis {
    Horizontal,
    Vertical,
}

/// Do two layout rectangles overlap? Touching edges do not: two screens that
/// share a border are side by side, which is the whole point of the exercise.
fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}

/// Push `rect` off every screen it overlaps, always along one axis and always in
/// one direction.
///
/// "Strict" is the point. Two outputs on top of each other are not a layout:
/// Hyprland draws one workspace over the other, and the pointer cannot reach the
/// lower one at all. So a drop *never* lands overlapping, and the direction the
/// gesture was going decides which side the screen ends up on - pulled downwards
/// it goes underneath, pulled upwards above, dragged sideways it goes beside.
///
/// Pushing always moves the same way, so the loop cannot oscillate between two
/// obstacles: every pass leaves the screen further along the axis, and a chain of
/// screens in the way is passed one at a time.
fn separate(rect: Rect, others: &[Rect], axis: Axis, sign: i32) -> Rect {
    let mut rect = rect;
    for _ in 0..=others.len() {
        let Some(other) = others.iter().find(|other| overlaps(rect, **other)) else {
            return rect;
        };
        rect = match axis {
            Axis::Vertical if sign < 0 => Rect {
                y: other.y - rect.h,
                ..rect
            },
            Axis::Vertical => Rect {
                y: other.bottom(),
                ..rect
            },
            Axis::Horizontal if sign < 0 => Rect {
                x: other.x - rect.w,
                ..rect
            },
            Axis::Horizontal => Rect {
                x: other.right(),
                ..rect
            },
        };
    }
    // Not reachable with the number of screens a desktop has, but "strict" has to
    // mean strict: a rectangle that survived every pass goes below all of them
    // rather than on top of one.
    let bottom = others.iter().map(Rect::bottom).max().unwrap_or(rect.y);
    Rect {
        y: rect.y.max(bottom),
        ..rect
    }
}

/// Where a screen goes when it is dragged past the edge of the picture: beyond
/// everything, on the side the gesture was going.
///
/// This is what makes a vertical arrangement reachable at all. The canvas only
/// ever shows the layout *as it is*, so the space underneath a screen may simply
/// not be on the picture yet - and dragging to that edge (and a little past it)
/// has to mean "under there", the way it does in a system's own arrangement
/// editor. The position along the other axis is kept, so a screen dragged
/// straight down lands directly below where it was.
fn beyond(rect: Rect, others: &[Rect], axis: Axis, sign: i32) -> Rect {
    match axis {
        Axis::Vertical if sign < 0 => match others.iter().map(|other| other.y).min() {
            Some(top) => Rect {
                y: top - rect.h,
                ..rect
            },
            None => rect,
        },
        Axis::Vertical => match others.iter().map(Rect::bottom).max() {
            Some(bottom) => Rect { y: bottom, ..rect },
            None => rect,
        },
        Axis::Horizontal if sign < 0 => match others.iter().map(|other| other.x).min() {
            Some(left) => Rect {
                x: left - rect.w,
                ..rect
            },
            None => rect,
        },
        Axis::Horizontal => match others.iter().map(Rect::right).max() {
            Some(right) => Rect { x: right, ..rect },
            None => rect,
        },
    }
}

/// Where a dropped screen settles: pushed clear of everything it overlaps, then
/// nudged onto a neighbour's edge if it is within reach, and then checked again -
/// a snap that would put it back on top of another screen is not a snap, and its
/// guide line would be a lie.
fn settle(rect: Rect, others: &[Rect], axis: Axis, sign: i32, threshold: i32) -> Placed {
    let cleared = separate(rect, others, axis, sign);
    let snapped = snap(cleared, others, threshold);
    let rect = separate(
        Rect {
            x: snapped.x,
            y: snapped.y,
            ..cleared
        },
        others,
        axis,
        sign,
    );
    let kept = rect.x == snapped.x && rect.y == snapped.y;
    Placed {
        rect,
        guide_x: if kept { snapped.guide_x } else { None },
        guide_y: if kept { snapped.guide_y } else { None },
    }
}

/// Do two spans share a *meaningful* part of themselves?
///
/// A pixel or two of overlap is rounding, not an arrangement: the canvas maps
/// layout pixels to a fraction of one, so two screens that really do line up can
/// come out a pixel apart. A screen dropped there is meant to be *centred*
/// against its neighbour, not left askew because the arithmetic overlapped by one
/// pixel - which is exactly the bug the `drop` verb caught.
fn shares_1d(a: i32, alen: i32, b: i32, blen: i32) -> bool {
    let overlap = (a + alen).min(b + blen) - a.max(b);
    overlap >= alen.min(blen) / 4
}

/// The gap between two spans, or `0` when they overlap.
fn gap_1d(a: i32, alen: i32, b: i32, blen: i32) -> i32 {
    if a + alen <= b {
        b - (a + alen)
    } else if b + blen <= a {
        a - (b + blen)
    } else {
        0
    }
}

/// The screen a drag is stacking against: the one nearest to it on the axis it is
/// *not* moving along.
///
/// This is what makes "put it underneath" work without having to steer it there.
/// A vertical drag moves a screen a few canvas pixels at most - the picture is
/// fitted to the layout as it is, so there is no room under anything *yet* - and
/// aiming by position is therefore hopeless. Aiming by intent works: the nearest
/// screen is the one it goes above or below, however wide the gap between the two
/// happens to be. (A *short* drag is still a nudge: that is [`STACK_TRAVEL`]'s
/// job, not a distance limit here.)
fn neighbour(rect: Rect, others: &[Rect], axis: Axis) -> Option<Rect> {
    others.iter().copied().min_by_key(|other| match axis {
        Axis::Vertical => gap_1d(rect.x, rect.w, other.x, other.w),
        Axis::Horizontal => gap_1d(rect.y, rect.h, other.y, other.h),
    })
}

/// Put `rect` on the `sign` side of `target`, lined up with it on the other axis:
/// the offset is kept when the two already share that axis, and the screen is
/// centred against the target when they do not - which is what "underneath" and
/// "beside" look like.
fn stacked(rect: Rect, target: Rect, axis: Axis, sign: i32) -> Rect {
    match axis {
        Axis::Vertical => Rect {
            x: if shares_1d(rect.x, rect.w, target.x, target.w) {
                rect.x
            } else {
                target.x + (target.w - rect.w).div_euclid(2)
            },
            y: if sign < 0 {
                target.y - rect.h
            } else {
                target.bottom()
            },
            ..rect
        },
        Axis::Horizontal => Rect {
            x: if sign < 0 {
                target.x - rect.w
            } else {
                target.right()
            },
            y: if shares_1d(rect.y, rect.h, target.y, target.h) {
                rect.y
            } else {
                target.y + (target.h - rect.h).div_euclid(2)
            },
            ..rect
        },
    }
}

/// The whole decision a drop makes, in layout coordinates.
///
/// `past` means the pointer left the canvas in the drag's direction (the screen
/// goes beyond everything); `magnet` means the gesture was a deliberate one, so
/// the screen is stacked against the nearest neighbour instead of merely nudged.
/// Either way the screen is then pushed clear of anything it still overlaps and
/// snapped onto an edge.
///
/// This is one function rather than a pipeline in the gesture handlers because
/// the *ghost* during a drag and the *drop* on release have to agree exactly:
/// the picture shows where the box will land.
fn placed(
    aim: Rect,
    others: &[Rect],
    axis: Axis,
    sign: i32,
    threshold: i32,
    magnet: bool,
    past: bool,
) -> Placed {
    let dropped = if past {
        beyond(aim, others, axis, sign)
    } else {
        aim
    };
    let aimed = match (magnet, neighbour(dropped, others, axis)) {
        (true, Some(target)) => stacked(dropped, target, axis, sign),
        _ => dropped,
    };
    settle(aimed, others, axis, sign, threshold)
}

/// Which way a drag is going - its axis, the direction along it, and how far the
/// pointer has travelled that way (in canvas pixels).
fn gesture_of(dx: f64, dy: f64) -> (Axis, i32, f64) {
    if dx.abs() > dy.abs() {
        (Axis::Horizontal, if dx < 0.0 { -1 } else { 1 }, dx.abs())
    } else {
        (Axis::Vertical, if dy < 0.0 { -1 } else { 1 }, dy.abs())
    }
}

/// Did the pointer leave the canvas along the way it was going?
///
/// The picture clamps the *box*, but the gesture is not clamped: the pointer can
/// travel past the canvas's edge, and doing that is how a screen is put where
/// there is no room to show it yet.
fn overshot(
    raw_x: i32,
    raw_y: i32,
    width: i32,
    height: i32,
    cell: Rect,
    axis: Axis,
    sign: i32,
) -> bool {
    match axis {
        Axis::Vertical if sign < 0 => raw_y < 0,
        Axis::Vertical => raw_y > (height - cell.h).max(0),
        Axis::Horizontal if sign < 0 => raw_x < 0,
        Axis::Horizontal => raw_x > (width - cell.w).max(0),
    }
}

/// The map from layout coordinates to canvas pixels and back.
///
/// One scale factor for both axes, because a stretched picture would be a lie
/// about the sizes; the padding is per axis, because a layout that is limited by
/// the canvas's height has width to spare and is centred in it.
#[derive(Clone, Copy)]
struct Map {
    origin_x: i32,
    origin_y: i32,
    /// Canvas pixels per layout pixel.
    scale: f64,
    pad_x: i32,
    pad_y: i32,
}

impl Map {
    /// The cell a layout rectangle occupies on the canvas. (The drawn box is inset
    /// by [`GAP`] inside it, so neighbouring boxes show their borders.)
    fn cell(&self, rect: Rect) -> Rect {
        Rect {
            x: self.pad_x + ((rect.x - self.origin_x) as f64 * self.scale).round() as i32,
            y: self.pad_y + ((rect.y - self.origin_y) as f64 * self.scale).round() as i32,
            w: ((rect.w as f64 * self.scale).round() as i32).max(12),
            h: ((rect.h as f64 * self.scale).round() as i32).max(12),
        }
    }

    fn layout_x(&self, canvas: i32) -> i32 {
        self.origin_x + (((canvas - self.pad_x) as f64) / self.scale).round() as i32
    }

    fn layout_y(&self, canvas: i32) -> i32 {
        self.origin_y + (((canvas - self.pad_y) as f64) / self.scale).round() as i32
    }

    /// Where a layout x-coordinate falls on the canvas - the column a vertical
    /// guide line is drawn at.
    fn canvas_x(&self, layout: i32) -> i32 {
        self.pad_x + ((layout - self.origin_x) as f64 * self.scale).round() as i32
    }

    /// The same, for a layout y-coordinate and a horizontal guide.
    fn canvas_y(&self, layout: i32) -> i32 {
        self.pad_y + ((layout - self.origin_y) as f64 * self.scale).round() as i32
    }

    /// The snap distance in layout pixels.
    fn threshold(&self) -> i32 {
        (SNAP / self.scale).round().max(1.0) as i32
    }
}

/// The layout rectangle of each output, in the compositor's order.
fn rectangles(displays: &[monitors::Display]) -> Vec<Rect> {
    displays
        .iter()
        .map(|display| {
            let (w, h) = display.logical_size();
            Rect {
                x: display.x,
                y: display.y,
                w: w.max(1),
                h: h.max(1),
            }
        })
        .collect()
}

/// The rectangle a press may land in to mean the cell `cell`.
///
/// For a pointer this is the cell itself - a mouse can be aimed, and a click a
/// few pixels off a screen is meant for the canvas, not for the screen. For a
/// finger it is the cell *grown around its centre* until both sides reach
/// [`touch::TARGET`]: a screen in a stacked layout is drawn smaller than its
/// share of the canvas, and "the laptop is the little box underneath" cannot
/// require a 20px-accurate tap. A cell that is already big enough is not touched,
/// so a layout of two large screens behaves exactly as it did.
fn reachable(cell: Rect, sloppy: bool) -> Rect {
    if !sloppy {
        return cell;
    }
    let grow_x = (touch::TARGET - cell.w).max(0) / 2;
    let grow_y = (touch::TARGET - cell.h).max(0) / 2;
    Rect {
        x: cell.x - grow_x,
        y: cell.y - grow_y,
        w: cell.w + 2 * grow_x,
        h: cell.h + 2 * grow_y,
    }
}

/// The screen a press at `(x, y)` means: the one whose (grown) cell holds the
/// point, or `None` on the bare canvas.
///
/// The grown areas of two neighbouring screens overlap - they are flush, that is
/// the point of the picture - so the nearest *centre* decides which one a finger
/// meant. With a pointer the cells do not overlap at all, and the answer is
/// exactly the box that was clicked.
fn hit_of(cells: &[Rect], x: f64, y: f64, sloppy: bool) -> Option<usize> {
    let mut best: Option<(f64, usize)> = None;
    for (index, cell) in cells.iter().enumerate() {
        let area = reachable(*cell, sloppy);
        let inside = x >= f64::from(area.x)
            && x < f64::from(area.x + area.w)
            && y >= f64::from(area.y)
            && y < f64::from(area.y + area.h);
        if !inside {
            continue;
        }
        let centre_x = f64::from(cell.x) + f64::from(cell.w) / 2.0;
        let centre_y = f64::from(cell.y) + f64::from(cell.h) / 2.0;
        let distance = (x - centre_x).powi(2) + (y - centre_y).powi(2);
        if best.is_none_or(|(best_distance, _)| distance < best_distance) {
            best = Some((distance, index));
        }
    }
    best.map(|(_, index)| index)
}

/// The canvas a set of layout rectangles is drawn in: the map from layout pixels
/// to canvas pixels, and the canvas's own width and height.
///
/// Split out of the widget so that the `drop` verb can answer "what would a drag
/// of these many canvas pixels do" without a pointer or a window - with the same
/// arithmetic, so the answer cannot drift from the behaviour.
fn frame(rects: &[Rect], width: i32) -> Option<(Map, i32, i32)> {
    let bounds = extent(rects)?;
    let width = width.max(180);
    // Fit the layout into the pane's width and the canvas's height, whichever
    // runs out first - and never above the height the rows below can spare. A
    // stack (taller than wide) gets the taller ceiling.
    let tallest = if bounds.h > bounds.w {
        MAX_TALL_HEIGHT
    } else {
        MAX_HEIGHT
    };
    let fit_width = (width - 2 * PAD).max(60) as f64;
    let fit_height = (tallest - 2 * PAD).max(40) as f64;
    let scale = (fit_width / f64::from(bounds.w)).min(fit_height / f64::from(bounds.h));
    let drawn_w = (f64::from(bounds.w) * scale).round() as i32;
    let drawn_h = (f64::from(bounds.h) * scale).round() as i32;
    let height = (drawn_h + 2 * PAD).clamp(MIN_HEIGHT, tallest);
    let map = Map {
        origin_x: bounds.x,
        origin_y: bounds.y,
        scale,
        pad_x: ((width - drawn_w) / 2).max(PAD),
        pad_y: ((height - drawn_h) / 2).max(PAD),
    };
    Some((map, width, height))
}

/// What a drag on one output's box would mean: the position it would be put at,
/// given a gesture of `dx`/`dy` canvas pixels - the units a `GtkGestureDrag`
/// reports, not layout pixels. `None` when `output` is not among `displays`.
///
/// The `drop` verb uses this to answer "why did that drop write that number"
/// without a pointer, and with the same [`frame`] and [`placed`] the gesture uses.
pub fn would_drop(
    displays: &[monitors::Display],
    width: i32,
    output: &str,
    dx: f64,
    dy: f64,
) -> Option<String> {
    let rects = rectangles(displays);
    let index = displays.iter().position(|display| display.name == output)?;
    let (map, canvas_width, canvas_height) = frame(&rects, width)?;
    let before = rects[index];
    let cell = map.cell(before);
    let raw_x = cell.x + dx.round() as i32;
    let raw_y = cell.y + dy.round() as i32;
    let pointer = Rect {
        x: raw_x.clamp(0, (canvas_width - cell.w).max(0)),
        y: raw_y.clamp(0, (canvas_height - cell.h).max(0)),
        ..cell
    };
    let others: Vec<Rect> = rects
        .iter()
        .enumerate()
        .filter(|(other, _)| *other != index)
        .map(|(_, rect)| *rect)
        .collect();
    let (axis, sign, travel) = gesture_of(dx, dy);
    let past = overshot(raw_x, raw_y, canvas_width, canvas_height, cell, axis, sign);
    let settled = placed(
        Rect {
            x: map.layout_x(pointer.x),
            y: map.layout_y(pointer.y),
            ..before
        },
        &others,
        axis,
        sign,
        map.threshold(),
        travel >= STACK_TRAVEL,
        past,
    );
    Some(format!("{}x{}", settled.rect.x, settled.rect.y))
}

/// One record per screen: its box, the label a drag borrows, and what the drag
/// arithmetic needs. The gesture itself lives on the *canvas* rather than on
/// the boxes, because a `GtkGestureDrag`'s offset is measured in its own
/// widget's coordinates - and a box that is being moved would carry that
/// coordinate system along with it, so the offset would stop meaning anything
/// after the first frame.
struct Screen {
    /// The connector, which the drop and the click both report.
    name: String,
    /// The layout rectangle the canvas was drawn from: what a drop is compared
    /// against to decide whether anything actually changed.
    before: Cell<Rect>,
    /// The cell the screen occupies on the canvas now.
    cell: Cell<Rect>,
    /// The logical size the size label shows between drags (`1368 × 912`).
    logical: Cell<(i32, i32)>,
    item: gtk::CenterBox,
    /// The size line under the name - which a drag borrows to show the landing
    /// position instead.
    size: gtk::Label,
    /// The other screens' layout rectangles, for snapping.
    others: RefCell<Vec<Rect>>,
}

/// The widgets and the drag state the gesture closures share with the
/// [`Arranger`] - split out so the closures hold this rather than the editor.
struct Shared {
    canvas: gtk::Fixed,
    /// The landing spot's outline, shown while a drag travels.
    ghost: gtk::Box,
    /// The snap's alignment lines, vertical and horizontal.
    guide_v: gtk::Box,
    guide_h: gtk::Box,
    screens: RefCell<Vec<Rc<Screen>>>,
    map: Cell<Map>,
    /// The canvas's size: the width is the pane's (fixed for the editor's
    /// lifetime), the height is the fit's and changes with the layout's shape.
    width: Cell<i32>,
    height: Cell<i32>,
    /// The press's target, its cell then, and whether it was the chosen one at
    /// the time: the deltas are measured from there rather than from the last
    /// update, which would make the box accelerate away from the pointer - and
    /// the click's toggle needs the press-time answer, because the press itself
    /// has already chosen the box by the time the release looks.
    held: Cell<Option<(usize, i32, i32, bool)>>,
    /// The connector a click (or a drag) chose - the popover's subject.
    selected: RefCell<Option<String>>,
    handlers: Handlers,
}

/// The arrangement editor as a persistent object: the canvas, its boxes, and
/// the drag state, kept alive by the element across page re-renders.
///
/// Re-rendering the Display page does not rebuild the editor - it calls
/// [`Arranger::update`] with the compositor's fresh answer, which re-fits the
/// picture in place, so a drop (or a scale change from the popover) does not
/// flicker the canvas and does not cost another `hyprctl -j monitors`.
pub struct Arranger {
    root: gtk::Box,
    shared: Rc<Shared>,
}

impl Arranger {
    /// Build the editor for the connected outputs. `None` when the compositor
    /// reported none: a page with nothing to arrange needs no canvas.
    ///
    /// `on_move` is told the connector and the position (`"1903x1440"`) a screen
    /// was *dropped* at - only when the drop actually changed something, so a
    /// click on a screen does not write a line that says nothing new.
    /// `on_select` is told the connector a click chose, and the box itself.
    pub fn new(
        width: i32,
        displays: &[monitors::Display],
        handlers: Handlers,
    ) -> Option<Rc<Arranger>> {
        let rects = rectangles(displays);
        let (map, width, height) = frame(&rects, width)?;

        let canvas = gtk::Fixed::new();
        canvas.add_css_class("arrange");
        canvas.set_size_request(width, height);

        // The ghost and the two guides go on the canvas first, so the boxes
        // draw over them; all three are hidden until a drag gives them
        // something to say.
        let ghost = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        ghost.add_css_class("arrange-ghost");
        ghost.set_visible(false);
        canvas.put(&ghost, 0.0, 0.0);
        let guide_v = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        guide_v.add_css_class("arrange-guide-v");
        guide_v.set_size_request(1, height);
        guide_v.set_visible(false);
        canvas.put(&guide_v, 0.0, 0.0);
        let guide_h = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        guide_h.add_css_class("arrange-guide-h");
        guide_h.set_size_request(width, 1);
        guide_h.set_visible(false);
        canvas.put(&guide_h, 0.0, 0.0);

        let screens: Vec<Rc<Screen>> = displays
            .iter()
            .enumerate()
            .map(|(index, display)| screen_for(display, &rects, index, &map, &canvas))
            .collect();

        let shared = Rc::new(Shared {
            canvas: canvas.clone(),
            ghost,
            guide_v,
            guide_h,
            screens: RefCell::new(screens),
            map: Cell::new(map),
            width: Cell::new(width),
            height: Cell::new(height),
            held: Cell::new(None),
            selected: RefCell::new(None),
            handlers,
        });

        let gesture = gtk::GestureDrag::new();
        {
            let shared = shared.clone();
            // Whether the press came from a finger decides whether the hit
            // areas grow; a mouse keeps the exact rectangle it always had.
            gesture.connect_drag_begin(move |gesture, x, y| {
                shared.drag_begin(x, y, touch::is_touch(gesture));
            });
        }
        {
            let shared = shared.clone();
            gesture.connect_drag_update(move |_, dx, dy| shared.drag_update(dx, dy));
        }
        {
            let shared = shared.clone();
            gesture.connect_drag_end(move |_, dx, dy| shared.drag_end(dx, dy));
        }
        canvas.add_controller(gesture);

        let hint = gtk::Label::new(Some(
            "Drag a display \u{b7} click it for its settings \u{b7} screens never overlap",
        ));
        hint.add_css_class("arrange-hint");
        hint.set_xalign(0.0);
        text::cap_width(&hint, (width - 8).max(120));
        hint.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.add_css_class("arrange-wrap");
        root.append(&canvas);
        root.append(&hint);
        Some(Rc::new(Arranger { root, shared }))
    }

    /// The widget the page appends (and re-appends) above the rows.
    pub fn root(&self) -> &gtk::Box {
        &self.root
    }

    /// The connector a click chose, when one is chosen - the popover's subject,
    /// so the element can refill the open popover after a write.
    pub fn selected(&self) -> Option<String> {
        self.shared.selected.borrow().clone()
    }

    /// Re-fit the picture to what the compositor reports now.
    ///
    /// When the outputs are the same ones in the same order (a drop, a new mode
    /// or scale, a monitor switched off), the boxes that exist are re-placed,
    /// re-sized and re-labelled rather than rebuilt: the canvas does not
    /// flicker, and a popover hanging off a box keeps its parent. Only a
    /// different *set* of outputs - a plug or an unplug - clears the canvas and
    /// builds new boxes. The chosen box keeps its `selected` mark if its output
    /// still exists.
    pub fn update(&self, displays: &[monitors::Display]) {
        let shared = &self.shared;
        let rects = rectangles(displays);
        let same = {
            let screens = shared.screens.borrow();
            let have: Vec<&str> = screens.iter().map(|screen| screen.name.as_str()).collect();
            same_outputs(&have, displays)
        };
        if !same {
            for screen in shared.screens.borrow().iter() {
                shared.canvas.remove(&screen.item);
            }
            shared.screens.borrow_mut().clear();
        }
        let Some((map, width, height)) = frame(&rects, shared.width.get()) else {
            // No outputs at all: nothing to draw, and nothing to drag.
            return;
        };
        shared.map.set(map);
        shared.height.set(height);
        shared.canvas.set_size_request(width, height);
        shared.guide_v.set_size_request(1, height);
        shared.guide_h.set_size_request(width, 1);

        if shared.screens.borrow().is_empty() {
            let screens: Vec<Rc<Screen>> = displays
                .iter()
                .enumerate()
                .map(|(index, display)| screen_for(display, &rects, index, &map, &shared.canvas))
                .collect();
            *shared.screens.borrow_mut() = screens;
        } else {
            for (index, screen) in shared.screens.borrow().iter().enumerate() {
                let display = &displays[index];
                let rect = rects[index];
                let cell = map.cell(rect);
                screen.before.set(rect);
                screen.cell.set(cell);
                let (logical_w, logical_h) = display.logical_size();
                screen.logical.set((logical_w, logical_h));
                screen
                    .size
                    .set_text(&format!("{logical_w} \u{d7} {logical_h}"));
                screen
                    .item
                    .set_size_request((cell.w - 2 * GAP).max(8), (cell.h - 2 * GAP).max(8));
                shared.canvas.move_(
                    &screen.item,
                    f64::from(cell.x + GAP),
                    f64::from(cell.y + GAP),
                );
                if display.focused {
                    screen.item.add_css_class("focused");
                } else {
                    screen.item.remove_css_class("focused");
                }
                if display.disabled {
                    screen.item.add_css_class("disabled");
                } else {
                    screen.item.remove_css_class("disabled");
                }
                screen.item.set_tooltip_text(Some(&format!(
                    "{} - drag to move it in the layout",
                    display.summary()
                )));
                *screen.others.borrow_mut() = rects
                    .iter()
                    .enumerate()
                    .filter(|(other, _)| *other != index)
                    .map(|(_, rect)| *rect)
                    .collect();
            }
        }
        // The choice survives a re-fit if its output does; a hotplug that took
        // the chosen output away takes the choice with it.
        match shared.selected.borrow().clone() {
            Some(name) if displays.iter().any(|display| display.name == name) => {
                shared.select(Some(&name));
            }
            Some(_) => shared.select(None),
            None => {}
        }
    }
}

impl Shared {
    /// The box a press landed on: its index, and its record. `sloppy` grows the
    /// hit areas to a fingertip (see [`reachable`]); a pointer keeps the exact
    /// rectangle.
    fn hit(&self, x: f64, y: f64, sloppy: bool) -> Option<(usize, Rc<Screen>)> {
        let screens = self.screens.borrow();
        let cells: Vec<Rect> = screens.iter().map(|screen| screen.cell.get()).collect();
        let index = hit_of(&cells, x, y, sloppy)?;
        Some((index, screens.get(index)?.clone()))
    }

    /// Mark one box as the chosen one - the only place the `selected` class
    /// moves, so two boxes can never wear it at once.
    fn select(&self, name: Option<&str>) {
        for screen in self.screens.borrow().iter() {
            if Some(screen.name.as_str()) == name {
                screen.item.add_css_class("selected");
            } else {
                screen.item.remove_css_class("selected");
            }
        }
        *self.selected.borrow_mut() = name.map(str::to_string);
    }

    /// Where a drag of `dx`/`dy` canvas pixels from the press's cell stands:
    /// the cell the pointer is over, and the place the screen would settle if
    /// the drag ended now - pointer and ghost, in one computation, because the
    /// two are shown on every update and must never come from different
    /// arithmetic.
    fn aim(&self, screen: &Screen, from_x: i32, from_y: i32, dx: f64, dy: f64) -> (Rect, Placed) {
        let map = self.map.get();
        let (width, height) = (self.width.get(), self.height.get());
        let cell = screen.cell.get();
        let raw_x = from_x + dx.round() as i32;
        let raw_y = from_y + dy.round() as i32;
        let pointer = Rect {
            x: raw_x.clamp(0, (width - cell.w).max(0)),
            y: raw_y.clamp(0, (height - cell.h).max(0)),
            ..cell
        };
        let (axis, sign, travel) = gesture_of(dx, dy);
        let past = overshot(raw_x, raw_y, width, height, cell, axis, sign);
        let landing = placed(
            Rect {
                x: map.layout_x(pointer.x),
                y: map.layout_y(pointer.y),
                ..screen.before.get()
            },
            &screen.others.borrow(),
            axis,
            sign,
            map.threshold(),
            travel >= STACK_TRAVEL,
            past,
        );
        (pointer, landing)
    }

    fn drag_begin(&self, x: f64, y: f64, sloppy: bool) {
        // A drag is a person working the picture, whichever box it started on
        // (or on none): the element's watchdog is re-armed before anything else
        // can decide the press was a miss.
        (self.handlers.on_activity)();
        let Some((index, screen)) = self.hit(x, y, sloppy) else {
            self.held.set(None);
            return;
        };
        let cell = screen.cell.get();
        let was = self.selected.borrow().as_deref() == Some(screen.name.as_str());
        self.held.set(Some((index, cell.x, cell.y, was)));
        // A drag is a choice too: the box being moved is the one a popover would
        // be about - but opening that popover is the click's job, not the
        // drag's, so this only moves the mark.
        self.select(Some(&screen.name));
        screen.item.add_css_class("dragging");
        screen.item.set_cursor_from_name(Some("grabbing"));
        // Lifted above its siblings for the journey: re-appending puts it last
        // on the Fixed, so it draws over everything it crosses.
        self.canvas.remove(&screen.item);
        self.canvas.put(
            &screen.item,
            f64::from(cell.x + GAP),
            f64::from(cell.y + GAP),
        );
    }

    fn drag_update(&self, dx: f64, dy: f64) {
        (self.handlers.on_activity)();
        let Some((index, from_x, from_y, _)) = self.held.get() else {
            return;
        };
        let screen = self.screens.borrow()[index].clone();
        let map = self.map.get();
        let cell = screen.cell.get();
        let max_x = (self.width.get() - cell.w).max(0);
        let max_y = (self.height.get() - cell.h).max(0);
        let (pointer, landing) = self.aim(&screen, from_x, from_y, dx, dy);
        // The box itself follows the pointer, always; where it would *land* is
        // the ghost's story.
        self.canvas.move_(
            &screen.item,
            f64::from(pointer.x + GAP),
            f64::from(pointer.y + GAP),
        );
        screen.cell.set(pointer);
        // The landing spot: the cell `placed` settles on, pinned to the canvas's
        // edge when the drop would park the screen beyond the picture. Hidden
        // where it would only underline the box itself.
        let shown = map.cell(landing.rect);
        let shown = Rect {
            x: shown.x.clamp(0, max_x),
            y: shown.y.clamp(0, max_y),
            ..shown
        };
        if shown.x == pointer.x && shown.y == pointer.y {
            self.ghost.set_visible(false);
        } else {
            self.ghost
                .set_size_request((shown.w - 2 * GAP).max(8), (shown.h - 2 * GAP).max(8));
            self.canvas.move_(
                &self.ghost,
                f64::from(shown.x + GAP),
                f64::from(shown.y + GAP),
            );
            self.ghost.set_visible(true);
        }
        // The lines the snap answered to: a shared edge, an adjacency, a centre,
        // or an origin axis.
        match landing.guide_x {
            Some(guide) => {
                self.canvas
                    .move_(&self.guide_v, f64::from(map.canvas_x(guide)), 0.0);
                self.guide_v.set_visible(true);
            }
            None => self.guide_v.set_visible(false),
        }
        match landing.guide_y {
            Some(guide) => {
                self.canvas
                    .move_(&self.guide_h, 0.0, f64::from(map.canvas_y(guide)));
                self.guide_h.set_visible(true);
            }
            None => self.guide_h.set_visible(false),
        }
        // The size line doubles as the drag's readout: where the box will go, in
        // the layout's own numbers.
        screen
            .size
            .set_text(&format!("{}x{}", landing.rect.x, landing.rect.y));
    }

    fn drag_end(&self, dx: f64, dy: f64) {
        (self.handlers.on_activity)();
        let Some((index, from_x, from_y, was)) = self.held.take() else {
            return;
        };
        let screen = self.screens.borrow()[index].clone();
        screen.item.remove_css_class("dragging");
        screen.item.set_cursor_from_name(Some("grab"));
        self.ghost.set_visible(false);
        self.guide_v.set_visible(false);
        self.guide_h.set_visible(false);
        let (logical_w, logical_h) = screen.logical.get();
        screen
            .size
            .set_text(&format!("{logical_w} \u{d7} {logical_h}"));
        // A click is not a move, and a click must not snap either: looking at
        // the picture cannot pin a position nobody chose. It is a *choice*
        // instead - and a second click on the chosen box lets it go again.
        if dx.abs() < 1.0 && dy.abs() < 1.0 {
            if was {
                self.select(None);
            } else {
                (self.handlers.on_select)(&screen.name, screen.item.upcast_ref());
            }
            return;
        }
        let map = self.map.get();
        let (_, landing) = self.aim(&screen, from_x, from_y, dx, dy);
        // Put the picture where the value will be - the ghost already showed
        // this, and the write repeats it.
        let cell = screen.cell.get();
        let landed = map.cell(landing.rect);
        let x = landed.x.clamp(0, (self.width.get() - cell.w).max(0));
        let y = landed.y.clamp(0, (self.height.get() - cell.h).max(0));
        self.canvas
            .move_(&screen.item, f64::from(x + GAP), f64::from(y + GAP));
        screen.cell.set(Rect { x, y, ..cell });
        let before = screen.before.get();
        if (landing.rect.x, landing.rect.y) != (before.x, before.y) {
            (self.handlers.on_move)(
                &screen.name,
                &format!("{}x{}", landing.rect.x, landing.rect.y),
            );
        }
    }
}

/// Do the boxes on the canvas already show exactly these outputs, in this
/// order? Then an update can re-fit them in place instead of rebuilding them.
fn same_outputs(have: &[&str], displays: &[monitors::Display]) -> bool {
    have.len() == displays.len()
        && have
            .iter()
            .zip(displays)
            .all(|(name, display)| *name == display.name)
}

/// One screen's record: its box on the canvas, and what the drag arithmetic
/// needs. Shared by [`Arranger::new`] and the rebuild half of
/// [`Arranger::update`].
fn screen_for(
    display: &monitors::Display,
    rects: &[Rect],
    index: usize,
    map: &Map,
    canvas: &gtk::Fixed,
) -> Rc<Screen> {
    let cell = map.cell(rects[index]);
    // The box is inset inside its cell, so two screens that touch still show
    // two borders - while their *positions* are the cells', not the boxes'.
    let (item, size) = box_for(
        display,
        (cell.w - 2 * GAP).max(8),
        (cell.h - 2 * GAP).max(8),
    );
    canvas.put(&item, f64::from(cell.x + GAP), f64::from(cell.y + GAP));
    let others = rects
        .iter()
        .enumerate()
        .filter(|(other, _)| *other != index)
        .map(|(_, rect)| *rect)
        .collect();
    Rc::new(Screen {
        name: display.name.clone(),
        before: Cell::new(rects[index]),
        cell: Cell::new(cell),
        logical: Cell::new(display.logical_size()),
        item,
        size,
        others: RefCell::new(others),
    })
}

/// One screen's box: its connector and the size it takes in the layout, centred
/// in a rectangle whose proportions are the layout's. The size label is handed
/// back as well, because a drag borrows it to show the landing position.
fn box_for(display: &monitors::Display, width: i32, height: i32) -> (gtk::CenterBox, gtk::Label) {
    let name = gtk::Label::new(Some(&display.name));
    name.add_css_class("arrange-name");
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text::cap_width(&name, (width - 6).max(16));

    let (logical_w, logical_h) = display.logical_size();
    let size = gtk::Label::new(Some(&format!("{logical_w} \u{d7} {logical_h}")));
    size.add_css_class("arrange-size");
    size.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text::cap_width(&size, (width - 6).max(16));

    let words = gtk::Box::new(gtk::Orientation::Vertical, 0);
    words.set_halign(gtk::Align::Center);
    words.set_valign(gtk::Align::Center);
    words.append(&name);
    words.append(&size);

    let item = gtk::CenterBox::new();
    item.add_css_class("arrange-item");
    item.set_orientation(gtk::Orientation::Vertical);
    item.set_center_widget(Some(&words));
    item.set_size_request(width, height);
    if display.focused {
        item.add_css_class("focused");
    }
    if display.disabled {
        item.add_css_class("disabled");
    }
    item.set_tooltip_text(Some(&format!(
        "{} - drag to move it in the layout",
        display.summary()
    )));
    item.set_cursor_from_name(Some("grab"));
    (item, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extent_holds_every_screen() {
        let rects = [
            Rect {
                x: 1903,
                y: 1440,
                w: 1368,
                h: 912,
            },
            Rect {
                x: 1368,
                y: 0,
                w: 2560,
                h: 1440,
            },
        ];
        // The layout of this machine: the laptop sits below the external screen
        // and further right, but not past its right edge - so the box is DP-1's
        // 2560-wide one, 2352 tall.
        assert_eq!(
            extent(&rects),
            Some(Rect {
                x: 1368,
                y: 0,
                w: 2560,
                h: 2352,
            })
        );
        assert_eq!(extent(&[]), None);
    }

    #[test]
    fn a_screen_dropped_near_an_edge_snaps_flush_to_it() {
        let other = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let dropped = Rect {
            x: 1926,
            y: 0,
            w: 1280,
            h: 1024,
        };
        // Directly beside it, five pixels of a gap: that is meant to be a shared
        // edge, because a gap that small cannot be crossed by the pointer.
        let snapped = snap(dropped, &[other], 12);
        assert_eq!((snapped.x, snapped.y), (1920, 0));
        // A top-aligned screen picks up the other one's top.
        let below = Rect {
            x: 0,
            y: 1074,
            w: 1280,
            h: 1024,
        };
        let snapped = snap(below, &[other], 12);
        assert_eq!((snapped.x, snapped.y), (0, 1080));
        // And the origin is a snap target of its own.
        let near_origin = Rect {
            x: 5,
            y: -4,
            w: 640,
            h: 480,
        };
        let snapped = snap(near_origin, &[other], 12);
        assert_eq!((snapped.x, snapped.y), (0, 0));
    }

    #[test]
    fn a_screen_dropped_far_from_every_edge_stays_where_it_was_put() {
        let other = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let dropped = Rect {
            x: 2400,
            y: 300,
            w: 1280,
            h: 1024,
        };
        let snapped = snap(dropped, &[other], 12);
        assert_eq!((snapped.x, snapped.y), (2400, 300));
        // Nothing snapped to means no line to draw.
        assert_eq!((snapped.guide_x, snapped.guide_y), (None, None));
    }

    /// A snap also says *which* line it snapped to - the shared edge, the
    /// adjacency, the neighbour's centre or an origin axis - which is what the
    /// drag draws as a guide while the ghost sits on it.
    #[test]
    fn a_snap_reports_the_line_it_snapped_to() {
        let other = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        // Beside it: the line the two screens share is the other's right edge.
        let beside = snap(
            Rect {
                x: 1926,
                y: 300,
                w: 1280,
                h: 1024,
            },
            &[other],
            12,
        );
        assert_eq!((beside.x, beside.guide_x), (1920, Some(1920)));
        assert_eq!((beside.y, beside.guide_y), (300, None));
        // Flush underneath it: the shared line is the other's bottom edge.
        let below = snap(
            Rect {
                x: 0,
                y: 1074,
                w: 1280,
                h: 1024,
            },
            &[other],
            12,
        );
        assert_eq!((below.y, below.guide_y), (1080, Some(1080)));
        // Centred against it: the guide is the other's middle line.
        let centred = snap(
            Rect {
                x: 316,
                y: 2000,
                w: 1280,
                h: 1024,
            },
            &[other],
            12,
        );
        assert_eq!((centred.x, centred.guide_x), (320, Some(960)));
        assert_eq!((centred.y, centred.guide_y), (2000, None));
        // And the origin's axes are lines too.
        let origin = snap(
            Rect {
                x: 5,
                y: -4,
                w: 640,
                h: 480,
            },
            &[other],
            12,
        );
        assert_eq!(origin.guide_x, Some(0));
        assert_eq!(origin.guide_y, Some(0));
    }

    /// A screen dropped on another is pushed clear - downwards when the gesture
    /// went down, which is what makes stacking easy: aim roughly below, and it
    /// lands directly underneath.
    #[test]
    fn a_screen_dropped_on_another_is_pushed_underneath_it() {
        let above = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let dropped = Rect {
            x: 100,
            y: 900,
            w: 1280,
            h: 1024,
        };
        assert!(overlaps(dropped, above));
        let below = separate(dropped, &[above], Axis::Vertical, 1);
        assert!(!overlaps(below, above));
        assert_eq!(below.y, 1080);
        assert_eq!(below.x, 100, "a push must not move the other axis");
    }

    /// Pulled upwards over the same screen, it lands *above* it instead.
    #[test]
    fn a_screen_dragged_upwards_lands_above_the_one_it_was_over() {
        let below = Rect {
            x: 0,
            y: 1080,
            w: 1920,
            h: 1080,
        };
        let dropped = Rect {
            x: 0,
            y: 1000,
            w: 1280,
            h: 1024,
        };
        let above = separate(dropped, &[below], Axis::Vertical, -1);
        assert!(!overlaps(above, below));
        assert_eq!(above.y, 1080 - 1024);
    }

    /// A sideways drag pushes sideways even when that is the longer way round:
    /// the gesture decides the side, not the arithmetic.
    #[test]
    fn a_sideways_drag_pushes_sideways() {
        let other = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let dropped = Rect {
            x: 100,
            y: 500,
            w: 1280,
            h: 1024,
        };
        let beside = separate(dropped, &[other], Axis::Horizontal, 1);
        assert!(!overlaps(beside, other));
        assert_eq!(beside.x, 1920);
        assert_eq!(beside.y, 500, "a push must not move the other axis");
    }

    /// Two screens in the way are passed one after the other.
    #[test]
    fn a_push_goes_past_every_screen_in_its_way() {
        let first = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let second = Rect {
            x: 0,
            y: 1080,
            w: 1920,
            h: 1080,
        };
        let dropped = Rect {
            x: 200,
            y: 900,
            w: 1280,
            h: 1024,
        };
        let below = separate(dropped, &[first, second], Axis::Vertical, 1);
        assert!(!overlaps(below, first) && !overlaps(below, second));
        assert_eq!(below.y, 2160);
    }

    /// The whole drop: pushed clear, then snapped onto the neighbour's edge - and
    /// the snap must not undo the push.
    #[test]
    fn a_drop_settles_clear_and_on_a_neighbours_edge() {
        let above = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let dropped = Rect {
            x: 6,
            y: 900,
            w: 1280,
            h: 1024,
        };
        let settled = settle(dropped, &[above], Axis::Vertical, 1, 12);
        assert!(!overlaps(settled.rect, above));
        assert_eq!((settled.rect.x, settled.rect.y), (0, 1080));
    }

    /// The case that made this necessary: two screens sit side by side and a
    /// downward drag on the right-hand one has to put it *underneath* the other -
    /// centred, because their widths differ - even though the picture has no room
    /// for it down there yet.
    #[test]
    fn a_vertical_drag_puts_a_side_by_side_screen_underneath_the_other() {
        let desk = Rect {
            x: 1368,
            y: 0,
            w: 2560,
            h: 1440,
        };
        let laptop = Rect {
            x: 3928,
            y: 368,
            w: 1368,
            h: 912,
        };
        let under = placed(laptop, &[desk], Axis::Vertical, 1, 72, true, false);
        assert_eq!(under.rect.y, 1440);
        assert_eq!(under.rect.x, 1368 + (2560 - 1368) / 2);
        assert!(!overlaps(under.rect, desk));

        let above = placed(laptop, &[desk], Axis::Vertical, -1, 72, true, false);
        assert_eq!(above.rect.y, -912);
        assert_eq!(above.rect.x, under.rect.x);
    }

    /// The same intent sideways: a stacked pair, and one of them dragged out to
    /// the side lands beside the other, centred.
    #[test]
    fn a_horizontal_drag_puts_a_stacked_screen_beside_the_other() {
        let top = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let bottom = Rect {
            x: 0,
            y: 1080,
            w: 1280,
            h: 1024,
        };
        let beside = placed(bottom, &[top], Axis::Horizontal, 1, 12, true, false);
        assert_eq!(beside.rect.x, 1920);
        assert_eq!(beside.rect.y, (1080 - 1024) / 2);
        assert!(!overlaps(beside.rect, top));
    }

    /// The live layout this was built against: a portrait desk monitor with the
    /// laptop sitting a thousand pixels to its right. A deliberate downward drag
    /// still stacks the laptop under it.
    #[test]
    fn a_vertical_drag_stacks_across_a_wide_gap() {
        let desk = Rect {
            x: 1368,
            y: 0,
            w: 1440,
            h: 2560,
        };
        let laptop = Rect {
            x: 3928,
            y: 264,
            w: 1368,
            h: 912,
        };
        let under = placed(laptop, &[desk], Axis::Vertical, 1, 64, true, false);
        assert_eq!(
            (under.rect.x, under.rect.y),
            (1368 + (1440 - 1368) / 2, 2560)
        );
        assert!(!overlaps(under.rect, desk));
    }

    /// One pixel of rounding is not an arrangement: a screen dropped one layout
    /// pixel inside its neighbour is centred, not left askew.
    #[test]
    fn a_pixel_of_rounding_is_not_lined_up() {
        let laptop = Rect {
            x: 0,
            y: 264,
            w: 1368,
            h: 912,
        };
        let desk = Rect {
            x: 1367,
            y: 0,
            w: 2560,
            h: 1440,
        };
        let under = placed(desk, &[laptop], Axis::Vertical, 1, 74, true, false);
        assert_eq!(under.rect.x, -596);
    }

    /// The live layout at the time this was written - the laptop on the left,
    /// the desk monitor on the right - stacks either way round. A wider screen
    /// centred under a narrower one sticks out on both sides, which is correct.
    #[test]
    fn the_live_layout_stacks_either_way() {
        let laptop = Rect {
            x: 0,
            y: 264,
            w: 1368,
            h: 912,
        };
        let desk = Rect {
            x: 1368,
            y: 0,
            w: 2560,
            h: 1440,
        };
        let laptop_under = placed(laptop, &[desk], Axis::Vertical, 1, 74, true, false);
        assert_eq!((laptop_under.rect.x, laptop_under.rect.y), (1964, 1440));
        let desk_under = placed(desk, &[laptop], Axis::Vertical, 1, 74, true, false);
        assert_eq!((desk_under.rect.x, desk_under.rect.y), (-596, 1176));
    }

    /// A short drag is still a nudge: the magnet is only meant for a deliberate
    /// gesture, so a screen can be moved a little without being stacked.
    #[test]
    fn a_short_drag_does_not_stack() {
        let desk = Rect {
            x: 1368,
            y: 0,
            w: 2560,
            h: 1440,
        };
        let laptop = Rect {
            x: 3928,
            y: 368,
            w: 1368,
            h: 912,
        };
        let nudged = Rect { y: 400, ..laptop };
        assert_eq!(
            placed(nudged, &[desk], Axis::Vertical, 1, 72, false, false).rect,
            nudged
        );
    }

    /// The nearest screen is the one a deliberate drag stacks against, however
    /// wide the gap: the picture has no room to express "somewhere else".
    #[test]
    fn a_deliberate_drag_stacks_against_the_nearest_screen() {
        let other = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let far = Rect {
            x: 5000,
            y: 4000,
            w: 1280,
            h: 1024,
        };
        let under = placed(far, &[other], Axis::Vertical, 1, 12, true, false);
        assert_eq!((under.rect.x, under.rect.y), ((1920 - 1280) / 2, 1080));
        assert!(!overlaps(under.rect, other));
        // A short drag of the same screen is a nudge, though.
        assert_eq!(
            placed(far, &[other], Axis::Vertical, 1, 12, false, false).rect,
            far
        );
    }

    /// Dragging past the picture's edge parks the screen beyond everything: the
    /// space underneath a screen is often not on the canvas yet, and that is
    /// precisely where a stack is built.
    #[test]
    fn a_screen_dragged_past_the_edge_parks_beyond_everything() {
        let first = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let second = Rect {
            x: 0,
            y: 1080,
            w: 1920,
            h: 1080,
        };
        let before = Rect {
            x: 300,
            y: 0,
            w: 1280,
            h: 1024,
        };
        let below = beyond(before, &[first, second], Axis::Vertical, 1);
        assert_eq!((below.x, below.y), (300, 2160));
        let above = beyond(before, &[first, second], Axis::Vertical, -1);
        assert_eq!(above.y, -1024);
        // Nothing to get past leaves the rectangle alone.
        assert_eq!(beyond(before, &[], Axis::Horizontal, 1), before);
    }

    /// Two screens that only share a border are side by side, not overlapping -
    /// the arrangement this editor exists to produce.
    #[test]
    fn touching_edges_are_not_a_collision() {
        let left = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let right = Rect {
            x: 1920,
            y: 0,
            w: 1280,
            h: 1024,
        };
        assert!(!overlaps(left, right));
        assert!(!overlaps(right, left));
        assert_eq!(separate(right, &[left], Axis::Horizontal, 1), right);
    }

    #[test]
    fn a_canvas_position_maps_back_to_the_layout_it_came_from() {
        let map = Map {
            origin_x: 1368,
            origin_y: 0,
            scale: 0.5,
            pad_x: 10,
            pad_y: 10,
        };
        let rect = Rect {
            x: 1903,
            y: 1440,
            w: 1368,
            h: 912,
        };
        let cell = map.cell(rect);
        // A 2x-monitor's box is drawn at half its resolution: 684x456.
        assert_eq!((cell.w, cell.h), (684, 456));
        // A position the scale can express exactly comes back unchanged, which is
        // what keeps a drop from creeping a pixel every time it is repeated.
        let exact = Rect {
            x: 1368,
            y: 0,
            w: 2560,
            h: 1440,
        };
        let drawn = map.cell(exact);
        assert_eq!(map.layout_x(drawn.x), 1368);
        assert_eq!(map.layout_y(drawn.y), 0);
    }

    /// A `monitors::Display` for the tests: a 2x panel is twice the pixels but
    /// not twice the layout, which is exactly what the picture must get right.
    fn display(
        name: &str,
        width: i32,
        height: i32,
        x: i32,
        y: i32,
        scale: f64,
    ) -> monitors::Display {
        monitors::Display {
            name: name.to_string(),
            width,
            height,
            refresh: 59.95,
            x,
            y,
            scale,
            transform: 0,
            mirror: "none".to_string(),
            disabled: false,
            focused: false,
            modes: Vec::new(),
        }
    }

    /// The ghost the drag shows and the drop the release writes are the same
    /// arithmetic: [`would_drop`] answers with [`placed`]'s rectangle, computed
    /// from the same frame and the same gesture - so the landing spot drawn
    /// under the pointer cannot disagree with the line the drop writes.
    #[test]
    fn the_ghost_and_the_drop_agree() {
        let desk = display("DP-1", 2560, 1440, 1368, 0, 1.0);
        let laptop = display("eDP-1", 2736, 1824, 3928, 368, 2.0);
        let displays = vec![desk, laptop];
        // A deliberate downward drag of the laptop stacks it under the desk,
        // centred, exactly as `a_vertical_drag_puts_a_side_by_side_screen_underneath_the_other`.
        let written = would_drop(&displays, 536, "eDP-1", 0.0, 120.0);
        assert_eq!(written.as_deref(), Some("1964x1440"));

        // The way there, computed the way the gesture computes its ghost.
        let rects = rectangles(&displays);
        let (map, width, height) = frame(&rects, 536).expect("two displays fit a canvas");
        let cell = map.cell(rects[1]);
        let raw_y = cell.y + 120;
        let pointer = Rect {
            x: cell.x,
            y: raw_y.clamp(0, (height - cell.h).max(0)),
            ..cell
        };
        let past = overshot(cell.x, raw_y, width, height, cell, Axis::Vertical, 1);
        let landing = placed(
            Rect {
                x: map.layout_x(pointer.x),
                y: map.layout_y(pointer.y),
                ..rects[1]
            },
            &rects[..1],
            Axis::Vertical,
            1,
            map.threshold(),
            true,
            past,
        );
        assert_eq!(
            format!("{}x{}", landing.rect.x, landing.rect.y),
            written.unwrap(),
            "the preview and the drop are the same `placed` call"
        );
        // And the stack answered to the desk's bottom edge and its middle line.
        assert_eq!(landing.guide_y, Some(1440));
    }

    /// The update's cheap path: the same outputs in the same order re-fit the
    /// boxes that exist; any other answer - a reordering, a plug, an unplug -
    /// rebuilds them.
    #[test]
    fn only_the_same_outputs_in_the_same_order_update_in_place() {
        let laptop = display("eDP-1", 2736, 1824, 0, 264, 2.0);
        let desk = display("DP-1", 2560, 1440, 1368, 0, 1.0);
        let both = vec![laptop, desk];
        assert!(same_outputs(&["eDP-1", "DP-1"], &both));
        assert!(!same_outputs(&["DP-1", "eDP-1"], &both));
        assert!(!same_outputs(&["eDP-1"], &both));
        assert!(!same_outputs(&["eDP-1", "DP-2"], &both));
    }

    /// A box drawn smaller than a fingertip (a stacked layout can do that) is
    /// hittable from a strip of canvas around it - but only for a finger: a
    /// pointer keeps the exact rectangle it has always had, so a click beside a
    /// screen is still a click on the canvas.
    #[test]
    fn a_fingertip_can_land_beside_a_box_drawn_smaller_than_a_target() {
        let small = Rect {
            x: 10,
            y: 10,
            w: 20,
            h: 16,
        };
        // Twelve pixels to the right of it: no part of the drawn box.
        assert_eq!(hit_of(&[small], 40.0, 20.0, false), None);
        // The grown area is TARGET wide and tall, centred on the cell.
        let grown = reachable(small, true);
        assert_eq!((grown.w, grown.h), (touch::TARGET, touch::TARGET));
        assert_eq!(grown.x + grown.w / 2, small.x + small.w / 2);
        assert_eq!(grown.y + grown.h / 2, small.y + small.h / 2);
        assert_eq!(hit_of(&[small], 40.0, 20.0, true), Some(0));
        // A box that is already big enough is left exactly alone.
        let big = Rect {
            x: 0,
            y: 0,
            w: 300,
            h: 200,
        };
        assert_eq!(reachable(big, true), big);
    }

    /// Two grown hit areas overlap because the boxes are flush - the nearest
    /// centre decides, so a finger picks the screen it is really over rather than
    /// whichever happened to be built first.
    #[test]
    fn the_nearest_centre_wins_where_two_grown_areas_overlap() {
        let left = Rect {
            x: 0,
            y: 0,
            w: 20,
            h: 20,
        };
        let right = Rect {
            x: 20,
            y: 0,
            w: 20,
            h: 20,
        };
        // Both grown cells cover x = 18..22; the point decides.
        assert_eq!(hit_of(&[left, right], 22.0, 10.0, true), Some(1));
        assert_eq!(hit_of(&[left, right], 18.0, 10.0, true), Some(0));
        // With a pointer the cells do not overlap and the box is the answer.
        assert_eq!(hit_of(&[left, right], 20.0, 10.0, false), Some(1));
        assert_eq!(hit_of(&[left, right], 19.0, 10.0, false), Some(0));
        // The bare canvas is nobody's box.
        assert_eq!(hit_of(&[left, right], 100.0, 100.0, true), None);
        assert_eq!(hit_of(&[], 0.0, 0.0, true), None);
    }
}

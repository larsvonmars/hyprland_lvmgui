//! The overview's arithmetic: the order the tiles go in, and how big they are.
//!
//! Both decisions live here, as pure functions over plain numbers, because both
//! are the kind that go subtly wrong - an ordering that buries the workspace you
//! are on in the middle of the card, a scale that misses the screen by exactly
//! one row. `view.rs` draws what this returns, `main.rs` decides what happens
//! when a tile is chosen; neither of them has to do arithmetic, and a test can
//! check both decisions without a compositor.

use hypr_osd_core::windows::Window;

/// The shape a tile falls back to when Hyprland reported no size for a window
/// (a window that is still starting up): a little wider than tall, which is what
/// most windows are.
const DEFAULT_ASPECT: f64 = 1.6;

/// How narrow and how wide a tile may become. A window that is nothing like a
/// rectangle - a slim palette, a banner, a terminal strip - keeps its
/// proportions only up to a point: past it the tile is either a sliver that
/// shows nothing, or a strip that eats a whole row and pushes everything else
/// onto the next one.
const MIN_ASPECT: f64 = 0.62;
const MAX_ASPECT: f64 = 2.4;

/// The room the tiles have, in layout pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub width: i32,
    pub height: i32,
}

/// What the tiles may be, in layout pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Smallest tile height worth drawing: below this a thumbnail is a smudge
    /// with a border.
    pub tile_min: i32,
    /// Largest tile height the card will use, however much room is left over;
    /// past it the overview stops being an overview and becomes two big windows.
    pub tile_max: i32,
    /// Space between tiles, and between rows of them.
    pub gap: i32,
    /// What every tile spends on its title line instead of on its picture.
    pub title: i32,
}

/// What one plan decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The height of every tile, picture and title together.
    pub tile_height: i32,
    /// Each tile's width, in the order the tiles are drawn: the window's own
    /// shape, at this plan's scale.
    pub widths: Vec<i32>,
    /// The rows, each holding the indices of the tiles that share it. This is
    /// what makes "down" a row rather than a tile, the way the arrow keys of a
    /// grid are supposed to behave.
    pub rows: Vec<Vec<usize>>,
}

/// Fit every tile into `area`, each in its window's own shape, as large as will
/// fit.
///
/// The scale is *solved*, not guessed: tile height is the one number the layout
/// hangs off (a taller tile is a wider tile, because the shape comes from the
/// window), so "does it fit" is a question about a single number - and the
/// largest one that fits is the answer. The alternative, a fixed thumbnail size
/// and a scrolling card, would put windows you cannot see on the one card whose
/// whole point is seeing them at once.
pub fn plan(windows: &[Window], area: Area, limits: Limits) -> Plan {
    if windows.is_empty() {
        return Plan {
            tile_height: limits.tile_min,
            widths: Vec::new(),
            rows: Vec::new(),
        };
    }
    // The search is a *bisection* rather than a walk down from the largest tile:
    // a smaller tile is a narrower one (the shape comes from the window), so
    // fewer or the same tiles fit per row and the block is shorter - "does it
    // fit" is monotone in the height, and the largest height that fits can be
    // found in eight steps instead of two hundred. That is the whole cost of
    // opening the card, and it happens on a key press.
    let mut low = limits.tile_min;
    let mut high = limits.tile_max;
    let mut best: Option<Plan> = None;
    while low <= high {
        let height = low + (high - low) / 2;
        let candidate = at(windows, height, area, limits);
        if fits(&candidate, area, limits.gap) {
            best = Some(candidate);
            low = height + 1;
        } else {
            high = height - 1;
        }
    }
    // Nothing fits: keep the smallest tile and let it overflow. A card that
    // shows too much is still readable; one that shows nothing is not.
    best.unwrap_or_else(|| at(windows, limits.tile_min, area, limits))
}

/// Every window laid out as if the tile height were `tile_height`.
fn at(windows: &[Window], tile_height: i32, area: Area, limits: Limits) -> Plan {
    let picture = (tile_height - limits.title).max(1);
    let widths: Vec<i32> = windows
        .iter()
        .map(|window| width_of(window, picture, area.width))
        .collect();
    let rows = pack(&widths, area.width, limits.gap);
    Plan {
        tile_height,
        widths,
        rows,
    }
}

/// One tile's width: the window's shape at this size, within bounds.
fn width_of(window: &Window, picture: i32, available: i32) -> i32 {
    let aspect = window
        .aspect()
        .unwrap_or(DEFAULT_ASPECT)
        .clamp(MIN_ASPECT, MAX_ASPECT);
    ((f64::from(picture) * aspect).round() as i32).clamp(1, available.max(1))
}

/// Fill rows left to right, starting a new one when the next tile would not fit.
///
/// A tile wider than the whole row still gets a row to itself (clamped by
/// [`width_of`]), because there is nowhere else to put it.
fn pack(widths: &[i32], available: i32, gap: i32) -> Vec<Vec<usize>> {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut row: Vec<usize> = Vec::new();
    let mut used = 0;
    for (index, width) in widths.iter().enumerate() {
        let with_gap = if row.is_empty() {
            *width
        } else {
            used + gap + *width
        };
        if !row.is_empty() && with_gap > available {
            rows.push(std::mem::take(&mut row));
            used = *width;
        } else {
            used = with_gap;
        }
        row.push(index);
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

/// Whether the block of rows fits the height, the gaps between them included.
fn fits(plan: &Plan, area: Area, gap: i32) -> bool {
    let rows = plan.rows.len() as i32;
    let needed = rows * plan.tile_height + (rows - 1).max(0) * gap;
    needed <= area.height
}

/// A rectangle on the card, in whatever widget's coordinates the caller is
/// working in - what a dragged tile is compared against.
///
/// GTK's own `graphene::Rect` would do except that it is not `Eq` and cannot be
/// built in a test without a display; a dropped window is a decision worth
/// testing, so the numbers are plain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

impl Span {
    /// A span from GTK's floating-point answer, in thousandths of a pixel - the
    /// rounding of a widget position is not something a hit test should be made
    /// to care about.
    pub fn from_rect(x: f64, y: f64, width: f64, height: f64) -> Self {
        let round = |value: f64| (value * 1000.0).round() as i64;
        Span {
            x: round(x),
            y: round(y),
            width: round(width).max(0),
            height: round(height).max(0),
        }
    }

    /// Whether `point` is inside - the edges count as inside, because a drop on
    /// the very edge of a chip is a drop on that chip.
    pub fn holds(&self, point: (f64, f64)) -> bool {
        let (x, y) = (
            (point.0 * 1000.0).round() as i64,
            (point.1 * 1000.0).round() as i64,
        );
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

/// Which of `spans` holds `point`, as an index.
///
/// The *last* one wins, so a caller can hand them over in the order the card
/// draws them and let the thing in front be the answer - a tile that overlaps
/// the strip (it cannot today, but a plan is free to change) would be the one a
/// drop lands on.
pub fn under(point: (f64, f64), spans: &[Span]) -> Option<usize> {
    spans.iter().rposition(|span| span.holds(point))
}

/// Put the windows in the order the card draws them.
///
/// Not the alt-tab order, which is what comes in: this is a card about
/// *workspaces*, so it groups them. The workspace you are on comes first - it is
/// the one you just came from, and the one the chips highlight - then the others
/// by number, and within a workspace the order is left exactly as it was, which
/// is most-recently-used first, because `windows::list` sorted it that way.
pub fn order(windows: &mut [Window], focused: &str) {
    // `sort_by_key` is stable, so windows that compare equal keep their MRU
    // order - which is the whole reason this is a sort and not a second sort.
    windows.sort_by_key(|window| rank(window, focused));
}

/// The sort key: the focused workspace first, then by number, then whatever is
/// not a number at all (`special:magic`).
fn rank(window: &Window, focused: &str) -> (bool, i32) {
    let number = window.workspace.parse::<i32>().unwrap_or(i32::MAX);
    (window.workspace != focused, number)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(workspace: &str, size: (i32, i32)) -> Window {
        Window {
            address: "0x559e6cdb4a30".to_string(),
            class: "kitty".to_string(),
            title: "a window".to_string(),
            workspace: workspace.to_string(),
            stable_id: "18000005".to_string(),
            size,
        }
    }

    const LIMITS: Limits = Limits {
        tile_min: 84,
        tile_max: 300,
        gap: 18,
        title: 26,
    };

    fn area(width: i32, height: i32) -> Area {
        Area { width, height }
    }

    /// Every window exactly once, and never wider than the room there is.
    fn assert_sane(plan: &Plan, count: usize, area: Area) {
        let laid_out: Vec<usize> = plan.rows.iter().flatten().copied().collect();
        assert_eq!(laid_out, (0..count).collect::<Vec<_>>());
        for row in &plan.rows {
            let used: i32 = row.iter().map(|index| plan.widths[*index]).sum::<i32>()
                + (row.len() as i32 - 1).max(0) * LIMITS.gap;
            assert!(used <= area.width, "row {row:?} is {used} wide");
        }
    }

    #[test]
    fn one_window_gets_the_biggest_tile_there_is() {
        let windows = [window("1", (1324, 820))];
        let plan = plan(&windows, area(1200, 700), LIMITS);
        assert_eq!(plan.tile_height, LIMITS.tile_max);
        assert_eq!(plan.rows, vec![vec![0]]);
        assert_sane(&plan, 1, area(1200, 700));
    }

    #[test]
    fn a_full_desktop_is_scaled_down_until_it_fits() {
        let windows: Vec<Window> = (0..12).map(|_| window("1", (1324, 820))).collect();
        let room = area(1200, 520);
        let plan = plan(&windows, room, LIMITS);
        assert!(plan.tile_height < LIMITS.tile_max);
        assert!(plan.tile_height >= LIMITS.tile_min);
        assert_sane(&plan, 12, room);
        assert!(fits(&plan, room, LIMITS.gap));
    }

    #[test]
    fn a_tile_keeps_the_window_s_own_shape() {
        let windows = [window("1", (1324, 820)), window("1", (820, 1324))];
        let plan = plan(&windows, area(1200, 700), LIMITS);
        assert!(
            plan.widths[0] > plan.widths[1],
            "a landscape window ({}) must be wider than a portrait one ({})",
            plan.widths[0],
            plan.widths[1]
        );
    }

    #[test]
    fn an_extreme_shape_is_kept_within_bounds() {
        let picture = LIMITS.tile_max - LIMITS.title;
        let windows = [
            window("1", (3000, 300)), // a banner
            window("1", (300, 3000)), // a sliver
        ];
        let plan = plan(&windows, area(4000, 2000), LIMITS);
        assert_eq!(
            plan.widths[0],
            (f64::from(picture) * MAX_ASPECT).round() as i32
        );
        assert_eq!(
            plan.widths[1],
            (f64::from(picture) * MIN_ASPECT).round() as i32
        );
    }

    #[test]
    fn a_window_with_no_size_still_gets_a_tile_of_the_usual_shape() {
        let picture = LIMITS.tile_max - LIMITS.title;
        let windows = [window("1", (0, 0))];
        let plan = plan(&windows, area(4000, 2000), LIMITS);
        assert_eq!(
            plan.widths[0],
            (f64::from(picture) * DEFAULT_ASPECT).round() as i32
        );
    }

    #[test]
    fn a_row_wraps_rather_than_sticking_out() {
        assert_eq!(pack(&[100, 100, 100], 250, 10), vec![vec![0, 1], vec![2]]);
        // …but a single tile that is too wide still gets a row: it has to go
        // somewhere, and clipping it would lose the window entirely.
        assert_eq!(pack(&[400, 100], 250, 10), vec![vec![0], vec![1]]);
        assert_eq!(pack(&[], 250, 10), Vec::<Vec<usize>>::new());
    }

    #[test]
    fn the_workspace_you_are_on_comes_first() {
        let mut windows = [
            window("5", (800, 600)),
            window("1", (800, 600)),
            window("3", (800, 600)),
            window("1", (800, 600)),
        ];
        order(&mut windows, "3");
        let order: Vec<&str> = windows.iter().map(|w| w.workspace.as_str()).collect();
        assert_eq!(order, ["3", "1", "1", "5"]);
    }

    #[test]
    fn the_order_inside_a_workspace_is_left_alone() {
        // The list arrives most-recently-used first; grouping must not shuffle
        // that, or the tiles would move around every time the card opens.
        let mut windows = [
            window("2", (800, 600)),
            window("2", (700, 600)),
            window("2", (600, 600)),
        ];
        order(&mut windows, "2");
        let widths: Vec<i32> = windows.iter().map(|w| w.size.0).collect();
        assert_eq!(widths, [800, 700, 600]);
    }

    #[test]
    fn a_named_workspace_sorts_after_the_numbered_ones() {
        let mut windows = [window("special:magic", (800, 600)), window("2", (800, 600))];
        order(&mut windows, "1");
        assert_eq!(windows[0].workspace, "2");
        assert_eq!(windows[1].workspace, "special:magic");
    }

    /// The bisection must answer what walking the heights down from the largest
    /// would have answered - it is only faster, not different.
    #[test]
    fn the_solved_height_is_the_largest_one_that_fits() {
        let shapes = [
            (1324, 820),
            (820, 1324),
            (800, 600),
            (1600, 400),
            (400, 900),
        ];
        // From one window up: an empty card answers with the smallest tile by
        // design, which is not a search at all.
        for count in 1..=13 {
            for (index, shape) in shapes.iter().enumerate() {
                let windows: Vec<Window> = (0..count)
                    .map(|position| {
                        window(
                            "1",
                            if position % 2 == index % 2 {
                                *shape
                            } else {
                                (900, 700)
                            },
                        )
                    })
                    .collect();
                for room in [(1200, 700), (1200, 520), (700, 900), (2400, 1400)] {
                    let area = area(room.0, room.1);
                    let plan = plan(&windows, area, LIMITS);
                    let expected = (LIMITS.tile_min..=LIMITS.tile_max)
                        .rev()
                        .find(|height| fits(&at(&windows, *height, area, LIMITS), area, LIMITS.gap))
                        .map(|height| at(&windows, height, area, LIMITS))
                        .unwrap_or_else(|| at(&windows, LIMITS.tile_min, area, LIMITS));
                    assert_eq!(plan, expected, "{count} windows of {shape:?} in {room:?}");
                }
            }
        }
    }

    #[test]
    fn a_drop_is_answered_by_the_thing_under_the_pointer() {
        let chips = [
            Span::from_rect(0.0, 0.0, 60.0, 32.0),
            Span::from_rect(68.0, 0.0, 60.0, 32.0),
        ];
        assert_eq!(under((10.0, 10.0), &chips), Some(0));
        assert_eq!(under((70.0, 10.0), &chips), Some(1));
        // The gap between two chips belongs to neither...
        assert_eq!(under((64.0, 10.0), &chips), None);
        // ...and neither does the air beside them.
        assert_eq!(under((200.0, 10.0), &chips), None);
        // An edge counts as inside: a drop that lands exactly on the border of a
        // chip is a drop on that chip.
        assert_eq!(under((68.0, 0.0), &chips), Some(1));
        assert_eq!(under((128.0, 32.0), &chips), Some(1));
    }

    #[test]
    fn the_last_span_wins_where_two_overlap() {
        // Targets arrive in drawing order, so the thing in front is the answer.
        let spans = [
            Span::from_rect(0.0, 0.0, 100.0, 100.0),
            Span::from_rect(50.0, 50.0, 100.0, 100.0),
        ];
        assert_eq!(under((25.0, 25.0), &spans), Some(0));
        assert_eq!(under((75.0, 75.0), &spans), Some(1));
        assert_eq!(under((150.0, 20.0), &spans), None);
    }
}

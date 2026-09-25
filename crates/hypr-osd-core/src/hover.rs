//! Where the pointer is, and what that means for a panel.
//!
//! Two of this desktop's surfaces unfold from the bar: the island popup from the
//! clock, in the middle, and the system popup from the status pill at the right
//! end. Both work the same way - the bar's pill *asks* for the panel when the
//! pointer arrives, and the panel decides when to go away again, because leaving
//! it means moving somewhere the bar cannot see: onto the panel itself.
//!
//! Two things are worth knowing about the sampling:
//!
//! * It happens **only while a panel is being tracked** (a hover that has not
//!   opened yet, or a panel that is up). An idle popup costs nothing at all,
//!   where the scripts this replaces asked the compositor for the pointer every 50
//!   ms for the whole session.
//! * It is a *poll*, not a subscription, because Hyprland has no pointer-motion
//!   event on its event socket - `cursorpos` on the command socket is what there
//!   is. It is a local socket round-trip, measured in microseconds.
//!
//! This module is the half that needs no GTK: the geometry of the two zones, the
//! pointer itself, and the state machine that turns the two into "open" and
//! "close". The elements own their surfaces; this owns the arithmetic, so the
//! timing rules can be tested with made-up clock readings instead of a pointer.

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::hypripc;

/// A rectangle in *layout* pixels, in the compositor's global layout
/// coordinates - the same space `grim -g` takes, and the same space
/// `j/monitors` reports a monitor's position in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn centre_x(&self) -> f64 {
        self.x + self.width / 2.0
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    /// Whether a point is inside, with `pad` pixels of grace on every side. The
    /// grace matters: the pointer has to cross the seam between the pill and the
    /// panel, and a pixel of rounding error there would close the panel.
    pub fn contains(&self, point: (f64, f64), pad: f64) -> bool {
        let (x, y) = point;
        x >= self.x - pad
            && x <= self.x + self.width + pad
            && y >= self.y - pad
            && y <= self.bottom() + pad
    }
}

/// The monitor the pointer is on, in layout pixels.
///
/// Hyprland reports a monitor's position and its size in *physical* pixels with
/// a separate scale factor, so the size is divided but the position is not:
/// `x`/`y` are already layout coordinates, which is what the pointer is in too.
pub fn focused_monitor() -> Option<Rect> {
    let monitors = hypripc::json("j/monitors")?;
    let list = monitors.as_array()?;
    let monitor = list
        .iter()
        .find(|monitor| {
            monitor
                .get("focused")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .or_else(|| list.first())?;
    monitor_rect(monitor)
}

/// The monitor with this connector (`eDP-1`, `DP-2`, ...), in the same pixels.
///
/// A panel is measured against the monitor the *bar* is on, which is not always
/// the focused one: the bar pins itself to the output it started on (see
/// `Opts::pin_output`), while "focused" follows the active workspace - and the
/// pointer can be resting on the bar of a screen that does not have the focus.
/// Working the geometry out against the wrong monitor puts the panel's hover
/// zones on another screen entirely, so the bar tells its popups which connector
/// it is on and they ask for that one.
pub fn monitor_of(connector: &str) -> Option<Rect> {
    let monitors = hypripc::json("j/monitors")?;
    let monitor = monitors
        .as_array()?
        .iter()
        .find(|monitor| monitor.get("name").and_then(Value::as_str) == Some(connector))?;
    monitor_rect(monitor)
}

/// The arithmetic of [`focused_monitor`], against a monitor exactly as Hyprland
/// writes it.
fn monitor_rect(monitor: &Value) -> Option<Rect> {
    let number = |key: &str| monitor.get(key).and_then(Value::as_f64);
    let scale = number("scale").filter(|scale| *scale > 0.0).unwrap_or(1.0);
    let width = number("width")?;
    let height = number("height")?;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some(Rect {
        x: number("x").unwrap_or(0.0),
        y: number("y").unwrap_or(0.0),
        width: (width / scale).round(),
        height: (height / scale).round(),
    })
}

/// The pointer, in the same coordinates.
pub fn cursor() -> Option<(f64, f64)> {
    parse_cursor(&hypripc::request("cursorpos")?)
}

/// `cursorpos` answers `123,456`.
fn parse_cursor(text: &str) -> Option<(f64, f64)> {
    let (x, y) = text.trim().split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// How the bar sits, which is what a panel's own position is derived from.
#[derive(Clone, Copy, Debug)]
pub struct BarGeometry {
    /// The bar's own height.
    pub height: f64,
    /// How far below the top edge it floats.
    pub margin_top: f64,
    /// The inset at each side of the screen - the bar's own `margin_x`.
    pub margin_x: f64,
    /// The gap between the bar's bottom edge and the panel's top edge.
    pub gap: f64,
    /// The transparent frame the shell keeps around a card (its `SHADOW_PAD`),
    /// so the shadow is not clipped. The panel's *visible* edge is this far
    /// inside the surface, and it is the visible edge a pointer has to be over.
    pub shadow_pad: f64,
}

/// Which pill a panel unfolds from, and therefore where it hangs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The middle of the bar - the clock, and the island popup under it. The
    /// clock is the bar's centre child, so its hot zone can be *derived* from
    /// the monitor's centre without the bar saying anything.
    Centre,
    /// The right end of the bar - the status pill, and the system popup under
    /// it, its right edge flush with the bar's. There is no deriving this one:
    /// the tray and the power button sit to its right, so how far from the edge
    /// it starts depends on what else is in the bar. The bar hands the pill's
    /// rectangle over instead (see `hypr-osd-stats`).
    Right,
}

/// A few pixels of grace above and below the bar, so the seam at its top edge is
/// not a hole the pointer can fall through.
const SLACK: f64 = 4.0;

/// The hot zone of the pill a panel belongs to, as far as it can be derived.
///
/// For [`Side::Centre`] this is exact (the clock is centred); for [`Side::Right`]
/// it is a fallback for a panel that was asked to open without being told where
/// its pill is - a wide strip at the bar's right end, which at least keeps the
/// panel open while the pointer is anywhere near it.
pub fn pill_rect(monitor: Rect, bar: BarGeometry, hot_width: f64, side: Side) -> Rect {
    let top = monitor.y + bar.margin_top;
    let right = monitor.x + monitor.width - bar.margin_x;
    let left = match side {
        Side::Centre => monitor.centre_x() - hot_width / 2.0,
        Side::Right => right - hot_width,
    };
    Rect {
        x: left,
        y: top - SLACK,
        width: hot_width,
        height: bar.height + SLACK * 2.0,
    }
}

/// Where a panel of `size` sits: under the bar, on the same side as its pill,
/// with the same gap and the same shadow inset as the surface around it.
pub fn panel_rect(monitor: Rect, bar: BarGeometry, side: Side, size: (f64, f64)) -> Rect {
    let centre = monitor.centre_x();
    let right = monitor.x + monitor.width - bar.margin_x;
    Rect {
        x: match side {
            Side::Centre => centre - size.0 / 2.0,
            // Right-aligned with the bar, so the popup and the bar's right-hand
            // end read as one object.
            Side::Right => right - size.0,
        },
        y: monitor.y + bar.margin_top + bar.height + bar.gap + bar.shadow_pad,
        width: size.0,
        height: size.1,
    }
}

/// What the island should do about a pointer sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Show,
    Hide,
}

/// How long the pointer has to rest before the panel opens, and how long it may
/// be away before it closes. Both exist to keep the panel from flickering: the
/// first when you are only passing over the clock, the second when you cross the
/// gap between the pill and the panel.
#[derive(Clone, Copy, Debug)]
pub struct Delays {
    pub open: Duration,
    pub close: Duration,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// Nothing to do: the pointer is not around and the panel is not up. No
    /// polling happens in this phase.
    Idle,
    /// The pointer arrived on the pill and is being watched for the dwell.
    Waiting,
    /// The panel is up.
    Open,
    /// The panel is up because somebody asked for it (`show`), and no amount of
    /// pointer movement will take it away.
    Pinned,
}

/// The hover's state machine.
pub struct Machine {
    phase: Phase,
    /// When the pointer settled on the pill - the dwell is measured from here.
    hover_since: Option<Instant>,
    /// When the pointer left both zones, if it has.
    out_since: Option<Instant>,
}

impl Default for Machine {
    fn default() -> Self {
        Machine {
            phase: Phase::Idle,
            hover_since: None,
            out_since: None,
        }
    }
}

impl Machine {
    /// The bar says the pointer has arrived on the pill.
    pub fn arm(&mut self, now: Instant) {
        if self.phase == Phase::Idle {
            self.phase = Phase::Waiting;
            self.hover_since = Some(now);
        }
    }

    /// One pointer sample. `in_pill` and `in_panel` say where it is.
    pub fn sample(
        &mut self,
        in_pill: bool,
        in_panel: bool,
        now: Instant,
        delays: Delays,
    ) -> Action {
        match self.phase {
            Phase::Idle | Phase::Pinned => Action::None,
            Phase::Waiting => {
                if !in_pill {
                    // Passed over, or left again before the dwell was up.
                    self.phase = Phase::Idle;
                    self.hover_since = None;
                    return Action::None;
                }
                let waited = self
                    .hover_since
                    .map(|since| now - since)
                    .unwrap_or_default();
                if waited >= delays.open {
                    self.phase = Phase::Open;
                    self.out_since = None;
                    Action::Show
                } else {
                    Action::None
                }
            }
            Phase::Open => {
                if in_pill || in_panel {
                    self.out_since = None;
                    return Action::None;
                }
                match self.out_since {
                    None => {
                        self.out_since = Some(now);
                        Action::None
                    }
                    Some(since) if now - since >= delays.close => {
                        self.phase = Phase::Idle;
                        self.out_since = None;
                        Action::Hide
                    }
                    Some(_) => Action::None,
                }
            }
        }
    }

    /// Take the panel away now - the `close` verb, or the session locking.
    pub fn dismiss(&mut self) -> Action {
        let was_up = matches!(self.phase, Phase::Open | Phase::Pinned);
        self.phase = Phase::Idle;
        self.hover_since = None;
        self.out_since = None;
        if was_up {
            Action::Hide
        } else {
            Action::None
        }
    }

    /// Put the panel up and keep it there - the `show` verb.
    pub fn pin(&mut self) {
        self.phase = Phase::Pinned;
        self.out_since = None;
    }

    /// Whether the panel is on screen.
    pub fn is_open(&self) -> bool {
        matches!(self.phase, Phase::Open | Phase::Pinned)
    }

    /// Whether the pointer has to keep being sampled. An idle island polls
    /// nothing at all, and a pinned one is not going to be dismissed by the
    /// pointer either.
    pub fn is_watching(&self) -> bool {
        matches!(self.phase, Phase::Waiting | Phase::Open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DELAYS: Delays = Delays {
        open: Duration::from_millis(120),
        close: Duration::from_millis(300),
    };

    fn at(millis: u64) -> Instant {
        // A fixed origin, so the tests read as "120 ms later" rather than as
        // whatever the wall clock says.
        Instant::now() + Duration::from_millis(millis)
    }

    #[test]
    fn nothing_happens_until_the_pill_is_hovered() {
        let mut machine = Machine::default();
        assert_eq!(machine.sample(true, false, at(0), DELAYS), Action::None);
        assert!(!machine.is_watching());
        assert!(!machine.is_open());
    }

    #[test]
    fn the_panel_waits_for_the_dwell_before_opening() {
        let mut machine = Machine::default();
        machine.arm(at(0));
        assert!(machine.is_watching());
        // Still on the pill, but the dwell is not up.
        assert_eq!(machine.sample(true, false, at(60), DELAYS), Action::None);
        assert!(!machine.is_open());
        assert_eq!(machine.sample(true, false, at(130), DELAYS), Action::Show);
        assert!(machine.is_open());
    }

    #[test]
    fn passing_over_the_pill_does_not_open_anything() {
        let mut machine = Machine::default();
        machine.arm(at(0));
        // Gone again after 50 ms: the dwell never completed.
        assert_eq!(machine.sample(false, false, at(50), DELAYS), Action::None);
        assert!(!machine.is_open());
        assert!(!machine.is_watching());
        // And it stays idle even if the pointer is over the zone again, until
        // the bar says the pointer has arrived.
        assert_eq!(machine.sample(true, false, at(400), DELAYS), Action::None);
    }

    #[test]
    fn the_panel_closes_only_after_the_grace_period() {
        let mut machine = Machine::default();
        machine.arm(at(0));
        assert_eq!(machine.sample(true, false, at(200), DELAYS), Action::Show);
        // Away, but not yet for long enough.
        assert_eq!(machine.sample(false, false, at(300), DELAYS), Action::None);
        assert!(machine.is_open());
        assert_eq!(machine.sample(false, false, at(450), DELAYS), Action::None);
        assert_eq!(machine.sample(false, false, at(620), DELAYS), Action::Hide);
        assert!(!machine.is_open());
        assert!(!machine.is_watching());
    }

    #[test]
    fn crossing_from_the_pill_to_the_panel_keeps_it_open() {
        let mut machine = Machine::default();
        machine.arm(at(0));
        assert_eq!(machine.sample(true, false, at(200), DELAYS), Action::Show);
        // The seam between the two: over neither, but only briefly.
        assert_eq!(machine.sample(false, false, at(250), DELAYS), Action::None);
        // On the panel: the grace period is forgotten, not merely paused.
        assert_eq!(machine.sample(false, true, at(300), DELAYS), Action::None);
        assert_eq!(machine.sample(false, false, at(1000), DELAYS), Action::None);
        assert_eq!(machine.sample(false, false, at(1400), DELAYS), Action::Hide);
    }

    #[test]
    fn a_pinned_panel_ignores_the_pointer() {
        let mut machine = Machine::default();
        machine.pin();
        assert!(machine.is_open());
        // Not watching means not polling either - nothing can dismiss it.
        assert!(!machine.is_watching());
        assert_eq!(machine.sample(false, false, at(9999), DELAYS), Action::None);
        assert_eq!(machine.dismiss(), Action::Hide);
        assert!(!machine.is_open());
    }

    #[test]
    fn dismissing_a_panel_that_is_not_up_is_not_a_hide() {
        let mut machine = Machine::default();
        assert_eq!(machine.dismiss(), Action::None);
        // And the bar arming again works after a dismissal.
        machine.arm(at(0));
        assert_eq!(machine.sample(true, false, at(200), DELAYS), Action::Show);
    }

    /// The monitor this desktop has: 1368x912 logical, at an offset, with the
    /// bar floating 12px in from each side.
    fn geometry() -> (Rect, BarGeometry) {
        (
            Rect {
                x: 1903.0,
                y: 1440.0,
                width: 1368.0,
                height: 912.0,
            },
            BarGeometry {
                height: 40.0,
                margin_top: 8.0,
                margin_x: 12.0,
                gap: 6.0,
                shadow_pad: 18.0,
            },
        )
    }

    #[test]
    fn a_centred_pill_and_panel_sit_in_the_middle() {
        let (monitor, bar) = geometry();
        let pill = pill_rect(monitor, bar, 280.0, Side::Centre);
        let panel = panel_rect(monitor, bar, Side::Centre, (440.0, 300.0));

        // The pill is centred on the monitor, and as tall as the bar plus slack.
        assert_eq!(pill.width, 280.0);
        assert!((pill.centre_x() - monitor.centre_x()).abs() < 0.001);
        assert!(pill.y < monitor.y + bar.margin_top);
        assert!(pill.bottom() > monitor.y + bar.margin_top + bar.height);

        // The panel hangs below the bar, centred, at the size it will be - its
        // visible edge, so one shadow-width inside the surface's own margin.
        assert!((panel.centre_x() - monitor.centre_x()).abs() < 0.001);
        assert_eq!(
            panel.y,
            monitor.y + bar.margin_top + bar.height + bar.gap + bar.shadow_pad
        );
        assert_eq!(panel.height, 300.0);
    }

    #[test]
    fn a_right_hand_pill_and_panel_hug_the_bars_right_end() {
        let (monitor, bar) = geometry();
        let bar_right = monitor.x + monitor.width - bar.margin_x;
        let pill = pill_rect(monitor, bar, 260.0, Side::Right);
        let panel = panel_rect(monitor, bar, Side::Right, (420.0, 340.0));

        // The fallback pill zone ends where the bar does, and is as tall as it.
        assert_eq!(pill.x + pill.width, bar_right);
        assert_eq!(pill.height, bar.height + 8.0);
        assert_eq!(pill.y, monitor.y + bar.margin_top - 4.0);

        // The panel's right edge lines up with the bar's, so the two read as one
        // object; its top edge is below the bar by the same gap as the centre
        // panel's.
        assert_eq!(panel.x + panel.width, bar_right);
        assert_eq!(
            panel.y,
            monitor.y + bar.margin_top + bar.height + bar.gap + bar.shadow_pad
        );
    }

    #[test]
    fn a_point_is_inside_a_rectangle_with_grace() {
        let rect = Rect {
            x: 100.0,
            y: 200.0,
            width: 50.0,
            height: 20.0,
        };
        assert!(rect.contains((120.0, 210.0), 0.0));
        assert!(!rect.contains((120.0, 230.0), 0.0));
        // The grace extends the rectangle on every side.
        assert!(rect.contains((120.0, 226.0), 8.0));
        assert!(!rect.contains((120.0, 240.0), 8.0));
        assert!(rect.contains((95.0, 210.0), 8.0));
        assert!(!rect.contains((90.0, 210.0), 8.0));
    }

    #[test]
    fn a_monitor_is_measured_in_layout_pixels() {
        let monitor = serde_json::json!({
            "name": "eDP-1",
            "x": 1903,
            "y": 1440,
            "width": 2736,
            "height": 1824,
            "scale": 2.0,
            "focused": true,
        });
        assert_eq!(
            monitor_rect(&monitor),
            Some(Rect {
                x: 1903.0,
                y: 1440.0,
                width: 1368.0,
                height: 912.0,
            })
        );
        // A monitor without a scale factor is not a division by zero.
        let unscaled = serde_json::json!({ "width": 800, "height": 600, "scale": 0 });
        assert_eq!(
            monitor_rect(&unscaled),
            Some(Rect {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            })
        );
    }

    #[test]
    fn the_cursor_is_read_as_a_pair() {
        assert_eq!(parse_cursor("123,456\n"), Some((123.0, 456.0)));
        assert_eq!(parse_cursor(" 1903 , 1440 "), Some((1903.0, 1440.0)));
        assert_eq!(parse_cursor("nonsense"), None);
        assert_eq!(parse_cursor(""), None);
    }
}

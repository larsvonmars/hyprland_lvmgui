//! A bar pill and its popup drawn as one morphing shape.
//!
//! The bar and its panels are separate processes, so the button cannot literally
//! become a widget in the panel's window. Instead the panel owns a transparent,
//! monitor-sized canvas and draws a rounded shape over the button's exact
//! rectangle. That shape grows into the panel while the bar retracts its other
//! pills; on close the same geometry runs backwards.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;

use crate::hover::{self, BarGeometry, Rect, Side};
use crate::{Hole, CARD_PAD_X, CARD_PAD_Y};

const OPEN_TIME: Duration = Duration::from_millis(240);
const CLOSE_TIME: Duration = Duration::from_millis(190);
const FRAME: Duration = Duration::from_millis(16);

/// Everything needed to locate one panel and the bar button it grows from.
#[derive(Clone, Copy, Debug)]
pub struct Origin {
    pub side: Side,
    pub bar: BarGeometry,
    pub monitor: Rect,
    /// The button inside the bar's card, as measured by the bar.
    pub pill: Option<Rect>,
}

impl Origin {
    fn pill_zone(&self, hot_width: f64) -> Rect {
        self.pill
            .map(|rect| Rect {
                x: self.monitor.x + self.bar.margin_x + rect.x,
                y: self.monitor.y + self.bar.margin_top + rect.y,
                ..rect
            })
            .unwrap_or_else(|| hover::pill_rect(self.monitor, self.bar, hot_width, self.side))
    }

    fn pill_shape(&self, hot_width: f64) -> Rect {
        let mut rect = self.pill_zone(hot_width);
        if self.pill.is_none() {
            rect.y = self.monitor.y + self.bar.margin_top;
            rect.height = self.bar.height;
        }
        rect
    }

    fn card(&self, size: (f64, f64)) -> Rect {
        hover::panel_rect(self.monitor, self.bar, self.side, size)
    }

    /// The visible card's rectangle in the tap catcher's monitor coordinates.
    pub fn hole(&self, size: (f64, f64)) -> Hole {
        let rect = self.card(size);
        Hole {
            x: (rect.x - self.monitor.x).round() as i32,
            y: (rect.y - self.monitor.y).round() as i32,
            width: rect.width.round() as i32,
            height: rect.height.round() as i32,
        }
    }
}

struct Movement {
    from: Rect,
    to: Rect,
    started: Instant,
    opening: bool,
    on_finish: Option<Box<dyn FnOnce()>>,
}

#[derive(Default)]
struct State {
    origin: Option<Origin>,
    current: Option<Rect>,
    movement: Option<Movement>,
    timer: Option<glib::SourceId>,
    min_width: i32,
}

/// The morphing surface shared by the three bar popups.
pub struct Island {
    canvas: gtk::Fixed,
    shape: gtk::Box,
    body: gtk::Box,
    content: gtk::Widget,
    state: RefCell<State>,
}

impl Island {
    pub fn new(content: &impl IsA<gtk::Widget>) -> Rc<Self> {
        let content = content.clone().upcast::<gtk::Widget>();
        content.set_halign(gtk::Align::Fill);
        content.set_valign(gtk::Align::End);
        content.set_margin_start(CARD_PAD_X);
        content.set_margin_end(CARD_PAD_X);
        content.set_margin_bottom(CARD_PAD_Y);

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.set_valign(gtk::Align::End);
        body.append(&content);

        let shape = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        shape.add_css_class("card");
        shape.add_css_class("island-shape");
        shape.set_overflow(gtk::Overflow::Hidden);
        shape.set_visible(false);
        shape.append(&body);

        let canvas = gtk::Fixed::new();
        canvas.add_css_class("island-canvas");
        canvas.put(&shape, 0.0, 0.0);

        let island = Rc::new(Island {
            canvas,
            shape,
            body,
            content,
            state: RefCell::new(State::default()),
        });
        let me = Rc::downgrade(&island);
        island.canvas.connect_map(move |_| {
            let Some(island) = me.upgrade() else {
                return;
            };
            let current = island.state.borrow().current;
            if let Some(current) = current {
                island.place(current);
            }
        });
        island
    }

    pub fn canvas(&self) -> &gtk::Fixed {
        &self.canvas
    }

    /// Set the content's minimum width, excluding the card's horizontal padding.
    pub fn set_min_width(&self, width: i32) {
        let width = width.max(1);
        self.state.borrow_mut().min_width = width;
        self.content.set_size_request(width, -1);
    }

    /// The button's hot zone and the finished, connected island in layout pixels.
    pub fn zones(&self, origin: &Origin, hot_width: f64) -> (Rect, Rect) {
        let pill = origin.pill_zone(hot_width);
        let card = origin.card(self.card_size());
        (pill, union(origin.pill_shape(hot_width), card))
    }

    /// Start at the opening button and grow into the finished panel.
    pub fn unfold(self: &Rc<Self>, origin: &Origin, hot_width: f64) {
        let pill = origin.pill_shape(hot_width);
        let from = if self.canvas.is_mapped() {
            self.state.borrow().current.unwrap_or(pill)
        } else {
            pill
        };
        let to = self.target(origin, hot_width);
        self.move_to(origin, from, to, true, None);
    }

    /// Return the shape to its opening button, then hide the layer surface.
    pub fn fold(self: &Rc<Self>, on_finish: impl FnOnce() + 'static) {
        let Some(origin) = self.state.borrow().origin else {
            on_finish();
            return;
        };
        let from = self.state.borrow().current;
        let Some(from) = from else {
            on_finish();
            return;
        };
        let to = origin.pill_shape(fallback_width(&origin));
        self.move_to(&origin, from, to, false, Some(Box::new(on_finish)));
    }

    /// Follow a content resize or a changed bar without replaying the open morph.
    pub fn refresh(&self, origin: &Origin) {
        let hot_width = fallback_width(origin);
        let target = self.target(origin, hot_width);
        let mut state = self.state.borrow_mut();
        if let Some(movement) = state.movement.as_mut() {
            movement.to = if movement.opening {
                target
            } else {
                origin.pill_shape(hot_width)
            };
        } else {
            state.current = Some(target);
            drop(state);
            self.place(target);
        }
    }

    /// The visible card's rectangle for the tap catcher.
    pub fn hole(&self, origin: &Origin) -> Hole {
        origin.hole(self.card_size())
    }

    fn card_size(&self) -> (f64, f64) {
        let min_width = self.state.borrow().min_width;
        let content_width = self
            .content
            .measure(gtk::Orientation::Horizontal, -1)
            .1
            .max(min_width)
            .max(1);
        let content_height = self
            .content
            .measure(gtk::Orientation::Vertical, content_width)
            .1
            .max(1);
        (
            f64::from(content_width + 2 * CARD_PAD_X),
            f64::from(content_height + 2 * CARD_PAD_Y),
        )
    }

    fn target(&self, origin: &Origin, hot_width: f64) -> Rect {
        union(origin.pill_shape(hot_width), origin.card(self.card_size()))
    }

    fn move_to(
        self: &Rc<Self>,
        origin: &Origin,
        from: Rect,
        to: Rect,
        opening: bool,
        on_finish: Option<Box<dyn FnOnce()>>,
    ) {
        self.cancel();
        self.shape.set_visible(true);
        self.shape.remove_css_class("from-left");
        self.shape.remove_css_class("from-centre");
        self.shape.remove_css_class("from-right");
        self.shape.add_css_class(match origin.side {
            Side::Left => "from-left",
            Side::Centre => "from-centre",
            Side::Right => "from-right",
        });
        self.body.set_halign(match origin.side {
            Side::Left => gtk::Align::Start,
            Side::Centre => gtk::Align::Center,
            Side::Right => gtk::Align::End,
        });

        let card = origin.card(self.card_size());
        let neck = (card.y - origin.pill_shape(1.0).y).max(0.0).round() as i32;
        self.content.set_margin_top(neck + CARD_PAD_Y);
        self.body.set_size_request(
            to.width.round().max(1.0) as i32,
            to.height.round().max(1.0) as i32,
        );

        self.state.borrow_mut().origin = Some(*origin);
        if !animations_enabled() || from == to {
            self.state.borrow_mut().current = Some(to);
            self.place(to);
            self.body.set_opacity(1.0);
            if let Some(on_finish) = on_finish {
                on_finish();
            }
            return;
        }

        self.place(from);
        self.body.set_opacity(if opening { 0.0 } else { 1.0 });
        self.state.borrow_mut().movement = Some(Movement {
            from,
            to,
            started: Instant::now(),
            opening,
            on_finish,
        });
        let me = Rc::downgrade(self);
        let timer = glib::timeout_add_local(FRAME, move || {
            let Some(island) = me.upgrade() else {
                return glib::ControlFlow::Break;
            };
            island.step()
        });
        self.state.borrow_mut().timer = Some(timer);
    }

    fn step(&self) -> glib::ControlFlow {
        let (from, to, elapsed, opening) = {
            let state = self.state.borrow();
            let Some(movement) = state.movement.as_ref() else {
                return glib::ControlFlow::Break;
            };
            (
                movement.from,
                movement.to,
                movement.started.elapsed(),
                movement.opening,
            )
        };
        let duration = if opening { OPEN_TIME } else { CLOSE_TIME };
        let part = (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0);
        let eased = ease_out_cubic(part);
        self.place(between(from, to, eased));
        let opacity = if opening {
            ((part - 0.16) / 0.46).clamp(0.0, 1.0)
        } else {
            (1.0 - part / 0.42).clamp(0.0, 1.0)
        };
        self.body.set_opacity(opacity);
        if part < 1.0 {
            return glib::ControlFlow::Continue;
        }

        let on_finish = {
            let mut state = self.state.borrow_mut();
            state.current = Some(to);
            state.timer = None;
            state
                .movement
                .take()
                .and_then(|mut movement| movement.on_finish.take())
        };
        self.place(to);
        self.body.set_opacity(if opening { 1.0 } else { 0.0 });
        if let Some(on_finish) = on_finish {
            on_finish();
        }
        glib::ControlFlow::Break
    }

    fn place(&self, rect: Rect) {
        self.state.borrow_mut().current = Some(rect);
        let width = rect.width.round().max(1.0) as i32;
        let height = rect.height.round().max(1.0) as i32;
        self.shape.set_size_request(width, height);
        let origin = self.state.borrow().origin;
        let x = (rect.x - origin.map(|origin| origin.monitor.x).unwrap_or(0.0)).round() as i32;
        let y = (rect.y - origin.map(|origin| origin.monitor.y).unwrap_or(0.0)).round() as i32;
        self.canvas.move_(&self.shape, f64::from(x), f64::from(y));

        // The layer surface covers the monitor, but only the morph belongs to it:
        // without an input region the transparent rest would swallow every click.
        if let Some(surface) = self.canvas.native().and_then(|native| native.surface()) {
            let region = gtk::cairo::Region::create_rectangle(&gtk::cairo::RectangleInt::new(
                x, y, width, height,
            ));
            surface.set_input_region(&region);
        }
    }

    fn cancel(&self) {
        let mut state = self.state.borrow_mut();
        if let Some(timer) = state.timer.take() {
            timer.remove();
        }
        state.movement = None;
    }
}

fn fallback_width(origin: &Origin) -> f64 {
    origin
        .pill
        .map(|pill| pill.width)
        .unwrap_or_else(|| match origin.side {
            Side::Left => 60.0,
            Side::Centre => 280.0,
            Side::Right => 240.0,
        })
}

fn union(first: Rect, second: Rect) -> Rect {
    let left = first.x.min(second.x);
    let top = first.y.min(second.y);
    let right = (first.x + first.width).max(second.x + second.width);
    let bottom = first.bottom().max(second.bottom());
    Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

fn between(from: Rect, to: Rect, part: f64) -> Rect {
    let at = |start: f64, end: f64| start + (end - start) * part;
    Rect {
        x: at(from.x, to.x),
        y: at(from.y, to.y),
        width: at(from.width, to.width),
        height: at(from.height, to.height),
    }
}

fn ease_out_cubic(part: f64) -> f64 {
    1.0 - (1.0 - part.clamp(0.0, 1.0)).powi(3)
}

fn animations_enabled() -> bool {
    gtk::Settings::default().is_none_or(|settings| settings.is_gtk_enable_animations())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(side: Side) -> Origin {
        Origin {
            side,
            bar: BarGeometry {
                height: 40.0,
                margin_top: 0.0,
                margin_x: 0.0,
                gap: 6.0,
                shadow_pad: 18.0,
            },
            monitor: Rect {
                x: 100.0,
                y: 200.0,
                width: 1368.0,
                height: 912.0,
            },
            pill: None,
        }
    }

    #[test]
    fn the_finished_shape_connects_the_button_and_card() {
        let origin = origin(Side::Centre);
        let pill = origin.pill_shape(280.0);
        let card = origin.card((476.0, 300.0));
        let shape = union(pill, card);
        assert_eq!(shape.x, card.x);
        assert_eq!(shape.y, pill.y);
        assert_eq!(shape.width, 476.0);
        assert_eq!(shape.height, card.bottom() - pill.y);
    }

    #[test]
    fn the_first_and_last_frames_are_the_two_real_rectangles() {
        let from = Rect {
            x: 10.0,
            y: 20.0,
            width: 40.0,
            height: 40.0,
        };
        let to = Rect {
            x: 0.0,
            y: 20.0,
            width: 500.0,
            height: 400.0,
        };
        assert_eq!(between(from, to, ease_out_cubic(0.0)), from);
        assert_eq!(between(from, to, ease_out_cubic(1.0)), to);
    }

    #[test]
    fn a_side_panel_keeps_its_outer_edge_and_joins_the_button() {
        let origin = origin(Side::Right);
        let pill = origin.pill_shape(240.0);
        let card = origin.card((540.0, 420.0));
        let shape = union(pill, card);
        assert_eq!(shape.x + shape.width, 1468.0);
        assert_eq!(shape.y, 200.0);
    }
}

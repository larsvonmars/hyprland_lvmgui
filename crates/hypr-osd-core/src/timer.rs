//! One-shot timers, and the trick that replaces cancelling them.
//!
//! Two of this collection's elements arm a timer on every keystroke (the
//! debounce before a file walk, the "nobody is here to press Escape" watchdog).
//! Cancelling the previous one is the obvious thing to do, and it is a trap:
//! **`glib::SourceId::remove()` panics once its timer has fired** - the source is
//! destroyed, `g_source_remove` fails, and the binding turns that into an
//! `unwrap`. The id of a fired timer is indistinguishable from the id of a
//! pending one, so a stored `SourceId` is a landmine: it went off for the
//! launcher the second time a query changed after a walk had already started, and
//! took the whole process with it.
//!
//! So nothing here is cancelled. Every timer carries the *generation* it was
//! armed with, and a timer whose generation is no longer the newest does nothing
//! when it fires. The scheduling is the same as ever; the cancellation is a
//! comparison, and it cannot panic.
//!
//! (A repeating timer is a different case and needs none of this: returning
//! `glib::ControlFlow::Break` from inside its callback destroys it cleanly.)

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;

/// Bump a generation counter and answer with the new value.
///
/// The primitive [`Timer`] is built on; public because an element that keys a
/// *result* to the query that asked for it (the launcher's file walk) needs the
/// same idea without a timer attached.
pub fn bump(counter: &Cell<u64>) -> u64 {
    let next = counter.get().wrapping_add(1);
    counter.set(next);
    next
}

/// A one-shot timer that can be armed again, cancelled, or simply left to fire -
/// and that does nothing when it has been overtaken.
///
/// Held as an `Rc` because arming hands the timer to GLib, which outlives the
/// call (see [`Timer::arm`]).
#[derive(Default)]
pub struct Timer {
    generation: Cell<u64>,
}

impl Timer {
    pub fn new() -> Self {
        Timer::default()
    }

    /// Run `action` after `duration`, unless this is armed again or cancelled in
    /// the meantime.
    ///
    /// Arming cancels whatever was pending: the generation is bumped, and the
    /// timer that was waiting for the old one finds it is no longer the newest
    /// and does nothing.
    pub fn arm<F: FnOnce() + 'static>(self: &Rc<Self>, duration: Duration, action: F) {
        let generation = bump(&self.generation);
        let me = Rc::clone(self);
        glib::timeout_add_local_once(duration, move || {
            if me.generation.get() == generation {
                action();
            }
        });
    }

    /// Make sure a timer that is still pending does nothing when it fires. The
    /// timer itself is not touched - it cannot be (see the module comment).
    pub fn cancel(&self) {
        bump(&self.generation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bumping_moves_the_counter_on() {
        let counter = Cell::new(0);
        assert_eq!(bump(&counter), 1);
        assert_eq!(bump(&counter), 2);
        assert_eq!(counter.get(), 2);
    }

    #[test]
    fn cancelling_overtakes_whatever_was_armed() {
        // The comparison `arm` makes at fire time, without a main loop: a timer
        // armed at generation 1 does nothing once the counter has moved on.
        let timer = Rc::new(Timer::new());
        let generation = bump(&timer.generation);
        timer.cancel();
        assert_ne!(timer.generation.get(), generation);
    }
}

//! Run a command once and hand its output to the main loop.
//!
//! The sibling of [`crate::follow`]. Where that watches a tool that prints a
//! line per event for the rest of the session, this runs a tool that answers
//! once - `grim`, capturing a window - and hands back whatever it wrote to
//! stdout. Both keep the *child* busy and the main loop free: the pipe becomes a
//! GIO stream, so the callback arrives when there is something to read, and the
//! card keeps drawing (and keeps hearing keys) in the meantime.
//!
//! An element uses this for work that costs real time. One window capture is a
//! tenth of a second or more, which is far too long to spend with the UI frozen.
//! A whole screen of them is started at once, so the wait is one capture rather
//! than the sum of them.

use std::cell::RefCell;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

// Reads that are still running.
//
// The callback of an asynchronous read only holds a *weak* reference, so
// without a strong one somewhere the read - and its stream, and its child -
// would be dropped the moment the function that started it returned, and the
// callback would never come. An element does not have to hold a handle (the
// overview fires a dozen captures and does not care about any of them), so the
// module holds them instead, until each one answers.
thread_local! {
    static IN_FLIGHT: RefCell<Vec<Rc<RefCell<State>>>> = const { RefCell::new(Vec::new()) };
}

/// Read in bigger chunks than a line-based follower does: this is a picture, and
/// even a thumbnail is a few hundred kilobytes.
const CHUNK: usize = 64 * 1024;

/// Upper bound on what one command may hand back. A capture on a 4K screen is
/// under ten megabytes; the cap is here so that a tool which runs away cannot
/// grow the daemon's memory for the rest of the session.
const MAX_OUTPUT: usize = 32 * 1024 * 1024;

/// Where a command's output goes, once it is all there.
type OutputCallback = Box<dyn FnOnce(Option<Vec<u8>>)>;

/// Run `program args…` and call `on_done` **on the main loop** with its stdout,
/// or with `None` when it wrote nothing, could not be started, or wrote more
/// than [`MAX_OUTPUT`].
///
/// The callback is `FnOnce`: a command that answers once cannot answer twice,
/// and saying so in the type means an element does not have to guard against it.
pub fn read(program: &str, args: &[String], on_done: impl FnOnce(Option<Vec<u8>>) + 'static) {
    let spawned = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // The tool's own complaints are not the card's business, and a command
        // that answers with nothing is reported as such instead.
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            eprintln!("hypr-osd: cannot run {program}: {error}");
            on_done(None);
            return;
        }
    };

    let stdout = child.stdout.take().expect("stdout was piped");
    // The stream takes ownership of the descriptor from here on (`ChildStdout`
    // hands it over without closing it), so nothing else may hold on to it - and
    // closing it is what ends the child's stdout.
    let stream = gio::UnixInputStream::take_fd(stdout.into());
    let state = Rc::new(RefCell::new(State {
        stream: Some(stream),
        child: Some(child),
        buffered: Vec::new(),
        on_done: Some(Box::new(on_done)),
    }));
    // Held until the read answers, so that it is not dropped out from under its
    // own callback - see `IN_FLIGHT`.
    IN_FLIGHT.with(|reads| reads.borrow_mut().push(state.clone()));
    State::read(&state);
}

/// What arrived, and what to do about it.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// More may come: keep reading.
    More,
    /// That was the end of it - and whether what was read is worth handing over.
    Done { deliver: bool },
}

/// Add a chunk to what has been read, and say whether to keep reading.
///
/// Split out of the async plumbing because this is the part with decisions in
/// it: the end of the output, and output that has grown beyond what we are
/// willing to hold. A decision is worth a test that needs no main loop. (The
/// plumbing around it is exercised by running an element that uses it, exactly
/// as [`crate::follow`]'s reads are.)
fn step(buffered: &mut Vec<u8>, chunk: &[u8]) -> Step {
    if chunk.is_empty() {
        return Step::Done { deliver: true };
    }
    if buffered.len() + chunk.len() > MAX_OUTPUT {
        // Truncated output is not an answer: a half-read picture decodes into
        // nothing useful, so this one is dropped rather than handed over.
        return Step::Done { deliver: false };
    }
    buffered.extend_from_slice(chunk);
    Step::More
}

struct State {
    stream: Option<gio::UnixInputStream>,
    /// Kept so the child can be reaped at the end: a daemon that leaves zombies
    /// behind for the rest of the session is a bug, and this process is a daemon.
    child: Option<Child>,
    buffered: Vec<u8>,
    on_done: Option<OutputCallback>,
}

impl State {
    /// Arm one read. GIO completes it when the pipe has data (it is a pollable
    /// stream), so this costs nothing while the command is thinking.
    fn read(me: &Rc<RefCell<Self>>) {
        let stream = {
            let state = me.borrow();
            match state.stream.clone() {
                Some(stream) => stream,
                None => return, // already finished
            }
        };
        let weak = Rc::downgrade(me);
        stream.read_bytes_async(
            CHUNK,
            glib::Priority::DEFAULT,
            gio::Cancellable::NONE,
            move |result| {
                let Some(me) = weak.upgrade() else { return };
                let step = match result {
                    // A read error means the pipe broke. What arrived first is
                    // still the best answer we have; if it is incomplete, the
                    // element's own decoding is what will notice.
                    Err(_) => Step::Done { deliver: true },
                    Ok(bytes) => step(&mut me.borrow_mut().buffered, &bytes),
                };
                match step {
                    Step::More => State::read(&me),
                    Step::Done { deliver } => State::finish(&me, deliver),
                }
            },
        );
    }

    /// Hand back what was read and let go of the child.
    fn finish(me: &Rc<RefCell<Self>>, deliver: bool) {
        // Closing the pipe and reaping the child happen *before* the callback
        // runs: a command that still has more to write than we are willing to
        // hold must not be left writing into a pipe nobody reads.
        let (data, child, on_done) = {
            let mut state = me.borrow_mut();
            state.stream = None;
            let data = if deliver && !state.buffered.is_empty() {
                Some(std::mem::take(&mut state.buffered))
            } else {
                None
            };
            (data, state.child.take(), state.on_done.take())
        };
        if let Some(mut child) = child {
            let _ = child.wait();
        }
        // Done: the registry can let go, and with the callback's own reference
        // gone at the end of this call the state - and the stream it closed -
        // goes with it.
        IN_FLIGHT.with(|reads| reads.borrow_mut().retain(|read| !Rc::ptr_eq(read, me)));
        if let Some(on_done) = on_done {
            on_done(data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn chunks_are_added_up_until_the_end_of_the_output() {
        let mut buffered = Vec::new();
        assert_eq!(step(&mut buffered, b"hypr"), Step::More);
        assert_eq!(step(&mut buffered, b"-osd"), Step::More);
        assert_eq!(buffered, b"hypr-osd");
        // An empty chunk is the end of the output, and what was read is the
        // answer.
        assert_eq!(step(&mut buffered, b""), Step::Done { deliver: true });
        assert_eq!(buffered, b"hypr-osd");
    }

    #[test]
    fn output_beyond_the_cap_is_dropped_rather_than_held() {
        let mut buffered = vec![0u8; MAX_OUTPUT - 2];
        // What still fits is accepted…
        assert_eq!(step(&mut buffered, &[0, 0]), Step::More);
        // …and what does not ends the read with nothing to hand over, because
        // half of a picture is not a picture.
        assert_eq!(step(&mut buffered, &[0]), Step::Done { deliver: false });
    }

    /// The answer to one question per read, shared with the callback.
    type Answer = Rc<RefCell<Option<Option<Vec<u8>>>>>;

    fn ask(program: &str, args: &[&str], answer: &Answer) {
        *answer.borrow_mut() = None;
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        let answer = answer.clone();
        read(program, &args, move |data| {
            *answer.borrow_mut() = Some(data)
        });
    }

    /// Drive the main loop until the read answers, and answer what it said.
    ///
    /// Driven by hand, and bounded by a deadline: a read that never completes
    /// has to fail the test, not hang it. This is the end-to-end half of the
    /// module - it is what catches a read whose state was dropped before its
    /// own callback arrived, which is exactly the bug `IN_FLIGHT` exists for.
    fn wait(context: &glib::MainContext, answer: &Answer) -> Option<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            // The borrow ends with this statement: the callback borrows the
            // same cell, and a live borrow across it would be a panic.
            if answer.borrow().is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "the read never completed");
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
        answer.borrow_mut().take().flatten()
    }

    /// The callback of an asynchronous read lands in the *thread-default* main
    /// context, so the test's own context has to be it while the read runs -
    /// and it has to be iterated for the callback to happen at all, which is why
    /// this test is a main loop wearing a test's clothes.
    #[test]
    fn a_command_answers_on_the_main_loop() {
        let context = glib::MainContext::new();
        let answer: Answer = Rc::new(RefCell::new(None));

        context
            .with_thread_default(|| {
                // What the command wrote is what comes back.
                ask("printf", &["hypr-osd"], &answer);
                assert_eq!(wait(&context, &answer), Some(b"hypr-osd".to_vec()));

                // A command that writes nothing answers nothing…
                ask("true", &[], &answer);
                assert_eq!(wait(&context, &answer), None);

                // …and so does one that cannot be started at all.
                ask("hypr-osd-no-such-command", &[], &answer);
                assert_eq!(wait(&context, &answer), None);
            })
            .expect("the test's own main context");
    }
}

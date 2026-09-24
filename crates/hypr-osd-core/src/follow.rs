//! Follow a long-running command's output, line by line.
//!
//! Some tools print one line per event and then run for the whole session:
//! `playerctl metadata --follow` is the one element #2 needs, and "watch a
//! thing and tell me when it changes" is the shape most of the remaining
//! elements will have. Doing that without blocking the UI *and* without polling
//! is what this module is for: the child's stdout becomes a GIO stream, so the
//! main loop wakes up exactly when the tool has something to say and only then.
//!
//! Two details worth knowing:
//!
//! * If the tool exits - a player can disappear, playerctl can be restarted -
//!   it is started again after a moment, so an element never loses its feed for
//!   the rest of the session. A tool that cannot be started at all (not
//!   installed) is reported once and not retried.
//! * A tool that *waits* for something to watch (playerctl does, when no player
//!   is running yet) is left alone: it is happy to sit on an open pipe.
//!
//! Elements must stop the follower when they go away (`Follow::stop`), because
//! a child process is not killed by its parent dying - see `Osd::on_shutdown`.

use std::cell::RefCell;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::time::Duration;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

/// How long to wait before starting the command again after it exited.
const RESTART_DELAY: Duration = Duration::from_millis(1000);

/// Read in chunks of a pipe's usual size.
const CHUNK: usize = 8192;

/// Upper bound on a line we are willing to buffer: a tool that writes without
/// newlines must not be able to grow our memory for the rest of the session.
const MAX_BUFFER: usize = 64 * 1024;

/// A command whose output is being followed. Dropping this does *not* stop it;
/// call [`Follow::stop`] (or the child outlives the daemon).
pub struct Follow {
    inner: Rc<RefCell<Inner>>,
}

impl Follow {
    /// Run `program args…` and call `on_line` - on the main loop - for every
    /// complete line it writes to stdout. Trailing newlines and empty lines are
    /// dropped; a line is never split across calls, however the tool writes it.
    pub fn start(program: &str, args: &[&str], on_line: impl Fn(&str) + 'static) -> Self {
        let inner = Rc::new(RefCell::new(Inner {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            on_line: Rc::new(on_line),
            child: None,
            stream: None,
            buffered: Vec::new(),
            generation: 0,
            stopped: false,
        }));
        Inner::spawn(&inner);
        Follow { inner }
    }

    /// Kill the child and forget everything about it.
    pub fn stop(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.stopped = true;
        inner.generation += 1;
        inner.stream = None;
        if let Some(mut child) = inner.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct Inner {
    program: String,
    args: Vec<String>,
    on_line: Rc<dyn Fn(&str)>,
    child: Option<Child>,
    /// Owns the read end of the child's stdout.
    stream: Option<gio::UnixInputStream>,
    buffered: Vec<u8>,
    /// Bumped on every (re)start, so a read that completes after a restart is
    /// recognised as belonging to the child that is already gone.
    generation: u64,
    stopped: bool,
}

impl Inner {
    /// Start the command and watch its stdout.
    fn spawn(me: &Rc<RefCell<Inner>>) {
        let (program, args) = {
            let mut inner = me.borrow_mut();
            if inner.stopped {
                return;
            }
            inner.generation += 1;
            inner.buffered.clear();
            (inner.program.clone(), inner.args.clone())
        };

        let spawned = Command::new(&program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            // The tool's own chatter ("No players found") is not the card's
            // business; a failure to start is reported below instead.
            .stderr(Stdio::null())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => {
                // Nothing to retry: a missing binary will not appear later.
                eprintln!("hypr-osd: cannot run {program}: {error}");
                return;
            }
        };
        let stdout = child.stdout.take().expect("stdout was piped");
        // The stream takes ownership of the descriptor from here on
        // (`ChildStdout` hands it over without closing it), so nothing else may
        // hold on to it - and closing it is what ends the child's stdout.
        let stream = gio::UnixInputStream::take_fd(stdout.into());

        {
            let mut inner = me.borrow_mut();
            inner.child = Some(child);
            inner.stream = Some(stream);
        }
        Inner::read(me);
    }

    /// Arm one read. GIO only completes it when the pipe actually has data (it
    /// is a pollable stream), so this costs nothing while the tool is idle.
    fn read(me: &Rc<RefCell<Inner>>) {
        let (stream, generation) = {
            let inner = me.borrow();
            match inner.stream.clone() {
                Some(stream) => (stream, inner.generation),
                None => return,
            }
        };
        let weak = Rc::downgrade(me);
        stream.read_bytes_async(
            CHUNK,
            glib::Priority::DEFAULT,
            gio::Cancellable::NONE,
            move |result| {
                let Some(me) = weak.upgrade() else { return };
                if me.borrow().generation != generation {
                    return; // a restart happened while this read was pending
                }
                match result {
                    Ok(bytes) if !bytes.is_empty() => {
                        me.borrow_mut().buffered.extend_from_slice(&bytes);
                        Inner::emit(&me);
                        Inner::read(&me);
                    }
                    // EOF or a read error: the child is gone (or its pipe broke).
                    _ => Inner::restart(&me),
                }
            },
        )
    }

    /// Hand every complete line to the callback, without holding the borrow
    /// while the callback runs - it belongs to an element and will touch widgets.
    fn emit(me: &Rc<RefCell<Inner>>) {
        // The borrow ends with the statement, so the callback below - which
        // belongs to an element and will touch widgets - runs without it.
        let lines = me.borrow_mut().take_lines();
        for line in lines {
            let on_line = me.borrow().on_line.clone();
            on_line(&line);
        }
    }

    fn restart(me: &Rc<RefCell<Inner>>) {
        {
            let mut inner = me.borrow_mut();
            inner.generation += 1;
            inner.stream = None;
            // Dropping the child does not kill it; it has already exited.
            inner.child = None;
            inner.buffered.clear();
        }

        let weak = Rc::downgrade(me);
        glib::timeout_add_local_once(RESTART_DELAY, move || {
            if let Some(me) = weak.upgrade() {
                Inner::spawn(&me);
            }
        });
    }

    /// Everything up to the last newline, as owned lines.
    fn take_lines(&mut self) -> Vec<String> {
        if self.buffered.len() > MAX_BUFFER {
            self.buffered.clear();
        }
        let mut lines = Vec::new();
        while let Some(end) = self.buffered.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffered.drain(..=end).collect();
            let text = String::from_utf8_lossy(&line[..line.len() - 1]);
            let text = text.trim_end_matches('\r').trim();
            if !text.is_empty() {
                lines.push(text.to_owned());
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Inner {
        Inner {
            program: String::new(),
            args: Vec::new(),
            on_line: Rc::new(|_: &str| {}),
            child: None,
            stream: None,
            buffered: Vec::new(),
            generation: 0,
            stopped: false,
        }
    }

    fn lines(input: &[u8]) -> Vec<String> {
        let mut inner = empty();
        inner.buffered = input.to_vec();
        inner.take_lines()
    }

    #[test]
    fn only_complete_lines_are_handed_out() {
        assert_eq!(lines(b"one\ntwo\nthree"), vec!["one", "two"]);
        // Nothing after the last newline is a line yet.
        assert_eq!(lines(b"one"), Vec::<String>::new());
        assert_eq!(lines(b"\n\n"), Vec::<String>::new());
        // A tool writing through a pty adds \r; it is not part of the line.
        assert_eq!(lines(b"noisy\r\n"), vec!["noisy"]);
    }

    #[test]
    fn a_line_split_across_reads_is_rejoined() {
        let mut inner = empty();
        inner.buffered.extend_from_slice(b"hel");
        assert!(inner.take_lines().is_empty());
        inner.buffered.extend_from_slice(b"lo\nsecond\npar");
        assert_eq!(inner.take_lines(), vec!["hello", "second"]);
    }
}

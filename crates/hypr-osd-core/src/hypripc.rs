//! Hyprland's own sockets.
//!
//! `hyprctl` is the friendly way in, and it stays the way every element that
//! asks *one* question works ([`crate::hyprctl`]). The bar is different: it has
//! to keep up with the compositor, redrawing the workspace row while a swipe is
//! still in progress, and spawning a process per update would be both slow and
//! impolite. So it talks to the two sockets Hyprland already publishes under
//! `$XDG_RUNTIME_DIR/hypr/<signature>` (with the documented `/tmp/hypr/<sig>`
//! fallback, exactly like every other Hyprland client):
//!
//! * `.socket.sock` - request/answer, the protocol `hyprctl` speaks too, with
//!   `j/`-prefixed commands answering JSON. See [`request`] and [`json`].
//! * `.socket2.sock` - a stream of `event>>data` lines, one per thing that
//!   happens, pushed to us. See [`Events`].
//!
//! Neither socket is required. Without `HYPRLAND_INSTANCE_SIGNATURE` (a session
//! that is not Hyprland at all) everything here answers `None` and a caller can
//! carry on with whatever it was going to draw.

use std::cell::RefCell;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;

/// How long the compositor may take to answer one request. It is a local
/// socket to a process that is already running, so anything beyond this is a
/// compositor that is wedged - and a bar that blocks on it would be too.
const REQUEST_TIMEOUT: Duration = Duration::from_millis(500);

/// How long to wait before reconnecting the event socket. Hyprland can be
/// restarted under a running bar (and a config reload re-creates the socket),
/// so a closed stream is a "see you in a moment", not an error.
const RECONNECT_DELAY: Duration = Duration::from_millis(1000);

/// Read in chunks of a pipe's usual size.
const CHUNK: usize = 8192;

/// Upper bound on what we buffer from the event socket: a stream that never
/// sends a newline must not be able to grow the bar's memory for the session.
const MAX_BUFFER: usize = 64 * 1024;

/// The directory Hyprland's sockets live in, or `None` when this is not a
/// Hyprland session.
fn directory() -> Option<PathBuf> {
    let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let runtime_home = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    // The documented location is `$XDG_RUNTIME_DIR/hypr/<sig>`; Hyprland falls
    // back to `/tmp/hypr/<sig>` when the runtime dir is not writable, and so do
    // we.
    for root in [runtime_home, PathBuf::from("/tmp")] {
        let dir = root.join("hypr").join(&signature);
        if dir.is_dir() {
            return Some(dir);
        }
    }
    None
}

/// The path of one of the two sockets (`".socket.sock"` or `".socket2.sock"`).
pub fn socket(name: &str) -> Option<PathBuf> {
    Some(directory()?.join(name))
}

/// Ask Hyprland one question on the command socket and read the whole answer.
///
/// The protocol is deliberately dumb: write the command, read until the
/// compositor closes its end. `None` covers every way that can go wrong (not a
/// Hyprland session, compositor gone, timeout), because every caller treats them
/// the same way - keep the last picture and try again later.
pub fn request(command: &str) -> Option<String> {
    let mut stream = UnixStream::connect(socket(".socket.sock")?).ok()?;
    stream.set_read_timeout(Some(REQUEST_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(REQUEST_TIMEOUT)).ok()?;
    stream.write_all(command.as_bytes()).ok()?;
    // Half-close: the answer is defined as "everything until EOF", and a
    // compositor that also waits for our end before answering would otherwise
    // deadlock until the timeout.
    stream.shutdown(std::net::Shutdown::Write).ok()?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer).ok()?;
    Some(answer)
}

/// A `j/…` request: the same question, with the answer parsed.
pub fn json(command: &str) -> Option<Value> {
    serde_json::from_str(&request(command)?).ok()
}

/// A subscription to the event socket.
///
/// Hold on to this: dropping it stops the feed (the in-flight read is the only
/// thing keeping the connection alive, so it goes with it).
pub struct Events {
    inner: Rc<RefCell<Inner>>,
}

impl Events {
    /// Call `on_event` - on the main loop - for every event line, e.g.
    /// `workspace>>3`, `activewindow>>kitty,README.md`. The line is passed as
    /// it arrived, `>>` and all: which events matter is the caller's business.
    pub fn start(on_event: impl Fn(&str) + 'static) -> Self {
        let inner = Rc::new(RefCell::new(Inner {
            on_event: Rc::new(on_event),
            stream: None,
            buffered: Vec::new(),
            generation: 0,
            stopped: false,
        }));
        Inner::connect(&inner);
        Events { inner }
    }

    /// Close the connection and stop hearing about events.
    pub fn stop(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.stopped = true;
        inner.generation += 1;
        // Dropping the stream closes the socket: the read (if any) is abandoned
        // with it, which is what we want.
        inner.stream = None;
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Inner {
    on_event: Rc<dyn Fn(&str)>,
    stream: Option<gio::UnixInputStream>,
    buffered: Vec<u8>,
    /// Bumped on every (re)connect, so a read that completes after a reconnect
    /// is recognised as belonging to the connection that is already gone.
    generation: u64,
    stopped: bool,
}

impl Inner {
    fn connect(me: &Rc<RefCell<Inner>>) {
        let path = {
            let inner = me.borrow();
            if inner.stopped {
                return;
            }
            match socket(".socket2.sock") {
                Some(path) => path,
                None => {
                    eprintln!(
                        "hypr-osd: no Hyprland event socket \
                         (HYPRLAND_INSTANCE_SIGNATURE unset?) - not watching for events"
                    );
                    return;
                }
            }
        };

        match UnixStream::connect(&path) {
            Ok(stream) => {
                // Non-blocking is what makes the GIO read below a *poll* rather
                // than a thread parked on a socket.
                if stream.set_nonblocking(true).is_err() {
                    Inner::retry(me);
                    return;
                }
                let stream = gio::UnixInputStream::take_fd(stream.into());
                let mut inner = me.borrow_mut();
                inner.generation += 1;
                inner.buffered.clear();
                inner.stream = Some(stream);
                drop(inner);
                Inner::read(me);
            }
            Err(error) => {
                // Hyprland not up yet, or just restarted. Not an error worth
                // shouting about; try again shortly.
                eprintln!("hypr-osd: cannot connect to {}: {error}", path.display());
                Inner::retry(me);
            }
        }
    }

    /// Arm one read. GIO only completes it when the socket actually has data
    /// (it is a pollable stream), so this costs nothing while nothing happens.
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
                // A reconnect happened while this read was pending: whatever it
                // produced belongs to the old connection.
                if me.borrow().generation != generation {
                    return;
                }
                match result {
                    Ok(bytes) if !bytes.is_empty() => {
                        me.borrow_mut().buffered.extend_from_slice(&bytes);
                        Inner::emit(&me);
                        Inner::read(&me);
                    }
                    // EOF or an error: the compositor went away.
                    _ => Inner::retry(&me),
                }
            },
        );
    }

    /// Hand every complete line to the callback, without holding the borrow
    /// while the callback runs - it belongs to an element and will touch widgets.
    fn emit(me: &Rc<RefCell<Inner>>) {
        let lines = me.borrow_mut().take_lines();
        for line in lines {
            let on_event = me.borrow().on_event.clone();
            on_event(&line);
        }
    }

    fn retry(me: &Rc<RefCell<Inner>>) {
        {
            let mut inner = me.borrow_mut();
            inner.generation += 1;
            inner.stream = None;
            inner.buffered.clear();
        }
        let weak = Rc::downgrade(me);
        glib::timeout_add_local_once(RECONNECT_DELAY, move || {
            if let Some(me) = weak.upgrade() {
                Inner::connect(&me);
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
            on_event: Rc::new(|_: &str| {}),
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
    fn events_arrive_one_line_at_a_time() {
        assert_eq!(
            lines(b"workspace>>3\nactivewindow>>kitty,README.md\n"),
            vec!["workspace>>3", "activewindow>>kitty,README.md"]
        );
        // A line split across two reads is put back together.
        let mut inner = empty();
        inner.buffered.extend_from_slice(b"movewindow>>0x1,");
        assert!(inner.take_lines().is_empty());
        inner.buffered.extend_from_slice(b"5\n");
        assert_eq!(inner.take_lines(), vec!["movewindow>>0x1,5"]);
    }
}

//! The MPRIS side of the desktop: what is playing, and controlling it.
//!
//! MPRIS is a D-Bus interface, but the tool that speaks it well is `playerctl`,
//! and going through it (rather than over D-Bus by hand) is the same choice the
//! rest of this collection makes for audio (`wpctl`) and the compositor
//! (`hyprctl`): no bindings to keep in step with a moving target. It also picks
//! the player *for* us - whatever is playing, which is the same choice the media
//! keys make.
//!
//! Three elements need this and none of them should own a second copy of the
//! format string: the media card ([`follow`]s and shows), the bar's media pill
//! (shows, and passes clicks through), and the island popup (shows everything,
//! including how far into the track it is - [`playback`]).

use std::process::Command;

use crate::follow::Follow;

/// How a track is reported.
///
/// The fields are joined with an ASCII unit separator (0x1f), a byte that cannot
/// occur in a title - a song called "A|B" would otherwise be mistaken for two
/// fields. `--follow` prints the current state as soon as it attaches, which is
/// what makes a player that only appears once playback starts (a browser, say)
/// work: its first line *is* the track that just started.
const FORMAT: &str = "{{status}}\u{1f}{{title}}\u{1f}{{artist}}\u{1f}{{album}}\u{1f}{{mpris:artUrl}}\u{1f}{{playerName}}";

/// The same, plus the two fields only a progress bar wants. Kept separate from
/// [`FORMAT`] on purpose: the followed feed must stay exactly what it was, and
/// position is not metadata - it changes continuously, so it has no business in
/// a stream that is supposed to only speak when something *happens*.
const PLAYBACK_FORMAT: &str = "{{title}}\u{1f}{{artist}}\u{1f}{{album}}\u{1f}{{mpris:artUrl}}\u{1f}{{playerName}}\u{1f}{{status}}\u{1f}{{position}}\u{1f}{{mpris:length}}";

const SEPARATOR: char = '\u{1f}';

/// One track, as far as a card or a pill is concerned.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// `mpris:artUrl` exactly as the player reported it - a URL, not a path.
    pub art: String,
    /// The player's own name ("vlc", "firefox"), used when there is nothing
    /// else for the second line.
    pub player: String,
}

impl Track {
    /// The second line: artist and album, joined the way the bar joins its
    /// dynamic fields (" · "), falling back to the player's name - a browser
    /// playing a video usually reports neither.
    pub fn subtitle(&self) -> String {
        let parts: Vec<&str> = [self.artist.as_str(), self.album.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect();
        if parts.is_empty() {
            self.player.clone()
        } else {
            parts.join(" · ")
        }
    }

    /// Whether there is anything to show at all: players publish an empty
    /// metadata dict shortly before the real one arrives.
    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.artist.is_empty() && self.album.is_empty()
    }

    /// What the media pill (and the island's one-line summary) puts on screen:
    /// the title, with the artist behind it when there is room for both.
    pub fn summary(&self) -> String {
        match (self.title.is_empty(), self.artist.is_empty()) {
            (false, false) => format!("{} · {}", self.title, self.artist),
            (false, true) => self.title.clone(),
            (true, false) => self.artist.clone(),
            (true, true) => self.player.clone(),
        }
    }
}

/// What the player reports: the track, and whether it is playing right now.
pub struct Event {
    pub track: Track,
    pub playing: bool,
}

/// One line of the followed output.
pub fn parse(line: &str) -> Option<Event> {
    let mut fields = line.split(SEPARATOR);
    let status = fields.next()?;
    Some(Event {
        track: Track {
            title: fields.next().unwrap_or_default().trim().to_owned(),
            artist: fields.next().unwrap_or_default().trim().to_owned(),
            album: fields.next().unwrap_or_default().trim().to_owned(),
            art: fields.next().unwrap_or_default().trim().to_owned(),
            player: fields.next().unwrap_or_default().trim().to_owned(),
        },
        playing: status.trim().eq_ignore_ascii_case("playing"),
    })
}

/// Follow the active player: `on_event` runs on the main loop for every metadata
/// change, including the state the follower sees when it attaches.
///
/// The player is picked by playerctl itself, which prefers whatever is playing -
/// the same choice the media keys and the bar make.
pub fn follow(on_event: impl Fn(Event) + 'static) -> Follow {
    Follow::start(
        "playerctl",
        &["metadata", "--follow", "--format", FORMAT],
        move |line| match parse(line) {
            Some(event) => on_event(event),
            None => eprintln!("hypr-osd: unreadable playerctl line: {line}"),
        },
    )
}

/// What the player reports right now, for a `show`/`status` verb or a pill that
/// is refreshed on a timer.
pub fn current() -> Result<Event, String> {
    let output = run(&["metadata", "--format", FORMAT])?;
    parse(output.trim())
        .ok_or_else(|| format!("could not read the player's metadata: {}", output.trim()))
}

/// The same snapshot, plus how far into the track we are - what the island
/// popup's progress bar is drawn from.
///
/// `None` covers every quiet case (no player running, nothing loaded, a player
/// that does not report a length), which is not an error worth printing: a
/// progress bar simply does not appear.
pub fn playback() -> Option<Playback> {
    let output = run(&["metadata", "--format", PLAYBACK_FORMAT]).ok()?;
    let line = output.trim();
    let mut fields = line.split(SEPARATOR);
    let track = Track {
        title: fields.next().unwrap_or_default().trim().to_owned(),
        artist: fields.next().unwrap_or_default().trim().to_owned(),
        album: fields.next().unwrap_or_default().trim().to_owned(),
        art: fields.next().unwrap_or_default().trim().to_owned(),
        player: fields.next().unwrap_or_default().trim().to_owned(),
    };
    let status = fields.next().unwrap_or_default().trim();
    let position = micros(fields.next().unwrap_or_default());
    let length = micros(fields.next().unwrap_or_default());
    if track.is_empty() {
        return None;
    }
    Some(Playback {
        playing: status.eq_ignore_ascii_case("playing"),
        track,
        position,
        length,
    })
}

/// Seconds into the track, and how long it is - both zero when the player does
/// not say. playerctl reports microseconds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playback {
    pub track: Track,
    pub playing: bool,
    pub position: u64,
    pub length: u64,
}

impl Playback {
    /// How full the progress bar is, `0.0..=1.0`. Zero for a stream with no
    /// length (a radio station), which is the signal to leave the bar away.
    pub fn fraction(&self) -> f64 {
        if self.length == 0 {
            return 0.0;
        }
        (self.position as f64 / self.length as f64).clamp(0.0, 1.0)
    }

    /// `m:ss` for the elapsed time and the total.
    pub fn elapsed(&self) -> String {
        clock(self.position)
    }

    pub fn total(&self) -> String {
        clock(self.length)
    }
}

/// Microseconds as reported in a format field, or 0. Anything that is not a
/// number at all (a player answering `{{mpris:length}}` with an empty string)
/// is simply unknown.
fn micros(field: &str) -> u64 {
    field.trim().parse().unwrap_or(0)
}

fn clock(microseconds: u64) -> String {
    let seconds = microseconds / 1_000_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Skip to the next or previous track.
///
/// The caller does not show the result itself: the metadata change that follows
/// arrives through [`follow`], so a button click and a track change from
/// anywhere else take the same path.
pub fn skip(direction: Direction) -> Result<(), String> {
    run(&[direction.verb()]).map(|_| ())
}

/// Play if paused, pause if playing.
pub fn play_pause() -> Result<(), String> {
    run(&["play-pause"]).map(|_| ())
}

#[derive(Clone, Copy)]
pub enum Direction {
    Next,
    Previous,
}

impl Direction {
    fn verb(self) -> &'static str {
        match self {
            Direction::Next => "next",
            Direction::Previous => "previous",
        }
    }
}

fn run(args: &[&str]) -> Result<String, String> {
    let output = Command::new("playerctl")
        .args(args)
        .output()
        .map_err(|error| format!("could not run playerctl ({error}) - is it installed?"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        return Err(if stderr.is_empty() {
            format!("playerctl {} failed", args.join(" "))
        } else {
            stderr.to_owned()
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_follow_line() {
        let line = "Playing\u{1f}Test Song One\u{1f}Test Artist One\u{1f}Test Album\u{1f}file:///tmp/art.png\u{1f}vlc";
        let event = parse(line).expect("a well-formed line");
        assert!(event.playing);
        assert_eq!(event.track.title, "Test Song One");
        assert_eq!(event.track.artist, "Test Artist One");
        assert_eq!(event.track.album, "Test Album");
        assert_eq!(event.track.art, "file:///tmp/art.png");
        assert_eq!(event.track.player, "vlc");
    }

    #[test]
    fn a_paused_player_is_not_playing() {
        let line = "Paused\u{1f}Song\u{1f}\u{1f}\u{1f}\u{1f}vlc";
        let event = parse(line).expect("a well-formed line");
        assert!(!event.playing);
        assert!(event.track.album.is_empty());
        // Nothing but the player's name: the card falls back to it.
        assert_eq!(event.track.subtitle(), "vlc");
    }

    #[test]
    fn missing_fields_do_not_break_the_parse() {
        // A player that reports only a status (nothing loaded).
        let event = parse("Stopped").expect("a well-formed line");
        assert!(!event.playing);
        assert!(event.track.is_empty());
        assert!(event.track.subtitle().is_empty());
    }

    #[test]
    fn the_subtitle_joins_artist_and_album() {
        let track = Track {
            artist: "Artist".to_owned(),
            album: "Album".to_owned(),
            player: "vlc".to_owned(),
            ..Track::default()
        };
        assert_eq!(track.subtitle(), "Artist · Album");
        let only_artist = Track {
            artist: "Artist".to_owned(),
            ..Track::default()
        };
        assert_eq!(only_artist.subtitle(), "Artist");
    }

    #[test]
    fn the_summary_is_the_title_and_the_artist() {
        let track = Track {
            title: "Title".to_owned(),
            artist: "Artist".to_owned(),
            player: "vlc".to_owned(),
            ..Track::default()
        };
        assert_eq!(track.summary(), "Title · Artist");
        assert_eq!(
            Track {
                title: "Title".to_owned(),
                ..Track::default()
            }
            .summary(),
            "Title"
        );
        // Nothing loaded: the player's name at least says *something*.
        assert_eq!(
            Track {
                player: "vlc".to_owned(),
                ..Track::default()
            }
            .summary(),
            "vlc"
        );
    }

    #[test]
    fn a_progress_fraction_needs_a_length() {
        let playback = |position: u64, length: u64| Playback {
            position,
            length,
            ..Playback::default()
        };
        assert!((playback(30_000_000, 60_000_000).fraction() - 0.5).abs() < 1e-9);
        // A stream that reports no length has no progress to draw.
        assert_eq!(playback(30_000_000, 0).fraction(), 0.0);
        // Overshooting (a player that keeps counting past the end) is clamped.
        assert_eq!(playback(90_000_000, 60_000_000).fraction(), 1.0);
    }

    #[test]
    fn times_are_read_as_minutes_and_seconds() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(9_000_000), "0:09");
        assert_eq!(clock(75_000_000), "1:15");
        assert_eq!(clock(3_600_000_000), "60:00");
    }
}

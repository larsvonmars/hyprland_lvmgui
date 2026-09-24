//! The MPRIS side of the element: what is playing, and skipping tracks.

use std::process::Command;

use hypr_osd_core::follow::Follow;

/// How a track is reported.
///
/// The fields are joined with an ASCII unit separator (0x1f), a byte that cannot
/// occur in a title - a song called "A|B" would otherwise be mistaken for two
/// fields. `--follow` prints the current state as soon as it attaches, which is
/// what makes a player that only appears once playback starts (a browser, say)
/// work: its first line *is* the track that just started.
const FORMAT: &str = "{{status}}\u{1f}{{title}}\u{1f}{{artist}}\u{1f}{{album}}\u{1f}{{mpris:artUrl}}\u{1f}{{playerName}}";

const SEPARATOR: char = '\u{1f}';

/// One track, as far as the card is concerned.
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
    /// The card's second line: artist and album, joined the way the bar joins
    /// its dynamic fields (" · "), falling back to the player's name - a browser
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
            None => eprintln!("hypr-osd-media: unreadable playerctl line: {line}"),
        },
    )
}

/// What the player reports right now, for the `show` and `status` verbs.
pub fn current() -> Result<Event, String> {
    let output = run(&["metadata", "--format", FORMAT])?;
    parse(output.trim())
        .ok_or_else(|| format!("could not read the player's metadata: {}", output.trim()))
}

/// Skip to the next or previous track. The card does not show the result itself:
/// the metadata change that follows arrives through `follow`, so a button click
/// and a track change from anywhere else take the same path.
pub fn skip(direction: Direction) -> Result<(), String> {
    run(&[direction.verb()]).map(|_| ())
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
}

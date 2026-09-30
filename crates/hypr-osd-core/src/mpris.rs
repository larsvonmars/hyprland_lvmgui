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
//!
//! The island's tile is the one that drives everything MPRIS offers, so the rest
//! of the commands live here too: [`set_shuffle`], [`set_repeat`], [`set_volume`]
//! and [`seek`], each read back by the next [`playback`] so a button and the
//! player cannot disagree. It is also the only caller that has to *choose* a
//! player - for which there is [`Player`] - because a desktop can have a browser
//! and a music player going at once; [`players`] is what lists them.

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

/// The same, plus the fields a progress bar and a row of controls want. Kept
/// separate from [`FORMAT`] on purpose: the followed feed must stay exactly what
/// it was, and position is not metadata - it changes continuously, so it has no
/// business in a stream that is supposed to only speak when something *happens*.
///
/// One call covers the whole island tile - the track, how far into it we are, and
/// the three properties its buttons wear - because asking for them in separate
/// calls would fork `playerctl` four times per heartbeat.
const PLAYBACK_FORMAT: &str = "{{title}}\u{1f}{{artist}}\u{1f}{{album}}\u{1f}{{mpris:artUrl}}\u{1f}{{playerName}}\u{1f}{{status}}\u{1f}{{position}}\u{1f}{{mpris:length}}\u{1f}{{shuffle}}\u{1f}{{loop}}\u{1f}{{volume}}";

const SEPARATOR: char = '\u{1f}';

/// Which player a command is aimed at.
///
/// `Active` is playerctl's own choice - whatever is playing, which is the same
/// choice the media keys and the bar make - and `Named` is a player the island's
/// tile has been switched to, by one of the names [`players`] answers with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Player {
    #[default]
    Active,
    Named(String),
}

impl Player {
    /// A player by name, as `playerctl --list-all` prints it.
    pub fn named(name: impl Into<String>) -> Self {
        Player::Named(name.into())
    }

    /// The name this player answers to, or `None` for "whichever is playing".
    pub fn name(&self) -> Option<&str> {
        match self {
            Player::Active => None,
            Player::Named(name) => Some(name),
        }
    }
}

/// What the player does when the track ends - `playerctl loop`'s three answers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Repeat {
    #[default]
    Off,
    /// Play this track again.
    Track,
    /// Play the queue again.
    All,
}

impl Repeat {
    /// The next one in the cycle a single button walks: off → this track →
    /// everything → off.
    pub fn next(self) -> Repeat {
        match self {
            Repeat::Off => Repeat::Track,
            Repeat::Track => Repeat::All,
            Repeat::All => Repeat::Off,
        }
    }

    /// What MPRIS calls it, which is also what goes back to `playerctl loop`.
    pub fn verb(self) -> &'static str {
        match self {
            Repeat::Off => "None",
            Repeat::Track => "Track",
            Repeat::All => "Playlist",
        }
    }

    /// One of the three answers, in whatever case a player wrote it - and `Off`
    /// for the empty string, which is what a player that does not implement the
    /// property reports.
    fn parse(text: &str) -> Repeat {
        match text.trim().to_ascii_lowercase().as_str() {
            "track" => Repeat::Track,
            "playlist" => Repeat::All,
            _ => Repeat::Off,
        }
    }
}

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
    let output = run(&Player::Active, &["metadata", "--format", FORMAT])?;
    parse(output.trim())
        .ok_or_else(|| format!("could not read the player's metadata: {}", output.trim()))
}

/// The same snapshot, plus the fields the island popup draws: how far into the
/// track we are, and the three properties its buttons wear.
///
/// `None` covers every quiet case (no player running, nothing loaded), which is
/// not an error worth printing: the tile simply says "Nothing playing".
pub fn playback(player: &Player) -> Option<Playback> {
    let output = run(player, &["metadata", "--format", PLAYBACK_FORMAT]).ok()?;
    parse_playback(output.trim())
}

/// One line of [`PLAYBACK_FORMAT`], split into the fields the island's tile
/// draws. Separate from [`playback`] so the parsing can be tested without a
/// player running, exactly like [`parse`] for the followed feed.
pub fn parse_playback(line: &str) -> Option<Playback> {
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
        shuffle: yes(fields.next().unwrap_or_default()),
        repeat: Repeat::parse(fields.next().unwrap_or_default()),
        volume: number(fields.next().unwrap_or_default()),
    })
}

/// Seconds into the track, how long it is, and what the controls are set to.
/// Position and length are zero when the player does not say; playerctl reports
/// microseconds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playback {
    pub track: Track,
    pub playing: bool,
    pub position: u64,
    pub length: u64,
    /// Shuffle is on.
    pub shuffle: bool,
    /// What the player does at the end of the track.
    pub repeat: Repeat,
    /// The player's *own* volume, `0.0..=1.0`, or `None` when it does not report
    /// one. This is not the sink's volume: a player at 40% of a sink at 80% is an
    /// ordinary state, and the sink's step belongs to the volume element.
    pub volume: Option<f64>,
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

    /// Where the pointer let go of the progress bar, as a number of seconds -
    /// what [`seek`] wants.
    pub fn second_at(&self, fraction: f64) -> u64 {
        (fraction.clamp(0.0, 1.0) * self.length as f64 / 1_000_000.0).round() as u64
    }

    /// The track's position in whole seconds.
    pub fn position_seconds(&self) -> u64 {
        self.position / 1_000_000
    }
}

/// Microseconds as reported in a format field, or 0. Anything that is not a
/// number at all (a player answering `{{mpris:length}}` with an empty string)
/// is simply unknown.
fn micros(field: &str) -> u64 {
    field.trim().parse().unwrap_or(0)
}

/// A boolean field. playerctl prints the property (`true`/`false`); a player that
/// only answers the verb prints `On`/`Off`.
fn yes(field: &str) -> bool {
    matches!(
        field.trim().to_ascii_lowercase().as_str(),
        "true" | "on" | "yes"
    )
}

/// A number field, or `None` when the player has nothing to say about it - a
/// player with no volume of its own reports an empty string.
fn number(field: &str) -> Option<f64> {
    field.trim().parse().ok()
}

/// The players that can be controlled, by the names `playerctl --list-all`
/// prints, with one player's duplicate names collapsed (see [`merge`]).
///
/// An empty list is not an error: it means nothing is running, and the island's
/// tile then offers no choice to make.
pub fn players() -> Vec<String> {
    let Ok(output) = run(&Player::Active, &["--list-all"]) else {
        return Vec::new();
    };
    merge(
        output
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty()),
    )
}

/// Collapse the duplicate names a single player can register.
///
/// VLC registers `vlc` **and** `vlc.instance10492`: the second is the name a
/// *second* VLC would answer to, so both are true names of one process, and two
/// chips for it would be a lie. A player that registers only instance names (two
/// VLCs, neither holding the plain name) keeps them all - those really are two
/// players. The plain name is the one kept, because it is the one a command
/// aimed at "vlc" reaches.
fn merge<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let names: Vec<&str> = names.collect();
    names
        .iter()
        .filter(|name| {
            // A plain name is always kept; an instance name only while its player
            // has not registered the plain name as well.
            let base = base_of(name);
            base == **name || !names.contains(&base)
        })
        .map(|name| (*name).to_owned())
        .collect()
}

/// `vlc.instance10492` → `vlc`; anything else is its own base.
fn base_of(name: &str) -> &str {
    name.split(".instance").next().unwrap_or(name)
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
pub fn skip(direction: Direction, player: &Player) -> Result<(), String> {
    run(player, &[direction.verb()]).map(|_| ())
}

/// Play if paused, pause if playing.
pub fn play_pause(player: &Player) -> Result<(), String> {
    run(player, &["play-pause"]).map(|_| ())
}

/// Jump to `seconds` into the track - what a click on the progress bar means.
///
/// Nothing is painted from the answer: the next [`playback`] reads the position
/// back, so the bar cannot disagree with the player about where the track is.
pub fn seek(player: &Player, seconds: u64) -> Result<(), String> {
    run(player, &["position", &seconds.to_string()]).map(|_| ())
}

/// Turn shuffle on or off.
pub fn set_shuffle(player: &Player, on: bool) -> Result<(), String> {
    run(player, &["shuffle", if on { "On" } else { "Off" }]).map(|_| ())
}

/// Walk the repeat cycle: none → this track → the whole queue.
pub fn set_repeat(player: &Player, repeat: Repeat) -> Result<(), String> {
    run(player, &["loop", repeat.verb()]).map(|_| ())
}

/// Set the player's *own* volume, `0.0..=1.0`.
pub fn set_volume(player: &Player, level: f64) -> Result<(), String> {
    run(
        player,
        &["volume", &format!("{:.2}", level.clamp(0.0, 1.0))],
    )
    .map(|_| ())
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

fn run(player: &Player, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("playerctl");
    // `--player` is what aims a command at one player. Without it playerctl picks
    // whatever is playing, which is exactly what `Player::Active` means.
    if let Some(name) = player.name() {
        command.args(["--player", name]);
    }
    let output = command
        .args(args)
        .output()
        .map_err(|error| format!("could not run playerctl ({error}) - is it installed?"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        return Err(if stderr.is_empty() {
            match player.name() {
                Some(name) => format!("playerctl --player {name} {} failed", args.join(" ")),
                None => format!("playerctl {} failed", args.join(" ")),
            }
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

    #[test]
    fn a_playback_line_carries_the_track_and_the_controls() {
        // The shape one `playerctl metadata --format …` really answers with:
        // captured from VLC playing a five-minute track at 40% volume.
        let line = "First Track\u{1f}Beyoncé\u{1f}Test Album\u{1f}file:///tmp/art.png\u{1f}vlc\u{1f}Playing\u{1f}10785903\u{1f}300386331\u{1f}true\u{1f}Track\u{1f}0.4";
        let playback = parse_playback(line).expect("a well-formed line");
        assert!(playback.playing);
        assert_eq!(playback.track.title, "First Track");
        assert_eq!(playback.track.artist, "Beyoncé");
        assert_eq!(playback.elapsed(), "0:10");
        assert_eq!(playback.total(), "5:00");
        assert_eq!(playback.position_seconds(), 10);
        assert!(playback.shuffle);
        assert_eq!(playback.repeat, Repeat::Track);
        assert_eq!(playback.volume, Some(0.4));
        // Where a click three quarters along the bar lands.
        assert_eq!(playback.second_at(0.75), 225);
        // A fraction from somewhere other than a pointer is clamped, not trusted.
        assert_eq!(playback.second_at(2.0), 300);
    }

    #[test]
    fn a_player_that_says_nothing_about_its_controls_is_off() {
        // A browser playing a video: shuffle off, no repeat, no volume of its
        // own, and a stream with no length.
        let line = "Video\u{1f}\u{1f}\u{1f}\u{1f}firefox\u{1f}Playing\u{1f}1000000\u{1f}0\u{1f}false\u{1f}\u{1f}";
        let playback = parse_playback(line).expect("a well-formed line");
        assert_eq!(playback.repeat, Repeat::Off);
        assert_eq!(playback.volume, None);
        assert!(!playback.shuffle);
        // Nothing to seek in a stream that reports no length.
        assert_eq!(playback.second_at(0.5), 0);
    }

    #[test]
    fn an_empty_player_is_not_a_playback() {
        assert_eq!(parse_playback(""), None);
        // A player that answers with a status but nothing loaded.
        assert_eq!(
            parse_playback("\u{1f}\u{1f}\u{1f}\u{1f}vlc\u{1f}Stopped"),
            None
        );
    }

    #[test]
    fn repeat_walks_its_cycle_and_names_itself() {
        assert_eq!(Repeat::Off.next(), Repeat::Track);
        assert_eq!(Repeat::Track.next(), Repeat::All);
        assert_eq!(Repeat::All.next(), Repeat::Off);
        // What goes back to `playerctl loop` is MPRIS's own spelling.
        assert_eq!(Repeat::All.verb(), "Playlist");
        assert_eq!(Repeat::Track.verb(), "Track");
        assert_eq!(Repeat::parse("playlist"), Repeat::All);
        assert_eq!(Repeat::parse(" TRACK "), Repeat::Track);
        // A player that does not implement the property, or answers oddly.
        assert_eq!(Repeat::parse(""), Repeat::Off);
        assert_eq!(Repeat::parse("something else"), Repeat::Off);
    }

    #[test]
    fn the_two_names_of_one_player_are_collapsed() {
        // VLC's pair, exactly as this machine reports it.
        assert_eq!(merge(["vlc", "vlc.instance10492"].into_iter()), vec!["vlc"]);
        // Two VLCs, neither holding the plain name: those are two players.
        assert_eq!(
            merge(["vlc.instance1", "vlc.instance2"].into_iter()),
            vec!["vlc.instance1", "vlc.instance2"]
        );
        // Unrelated names are just names.
        assert_eq!(
            merge(["firefox", "vlc"].into_iter()),
            vec!["firefox", "vlc"]
        );
    }

    #[test]
    fn a_player_is_either_named_or_whichever_is_playing() {
        assert_eq!(Player::Active.name(), None);
        assert_eq!(Player::named("vlc").name(), Some("vlc"));
        assert_eq!(Player::default(), Player::Active);
    }
}

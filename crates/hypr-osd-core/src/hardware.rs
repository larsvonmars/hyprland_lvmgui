//! The small facts about the machine that the bar's pills and the system popup
//! are drawn from: the sink's volume, the battery, the wireless link.
//!
//! Three very different sources, one shape: read what the system already
//! publishes, parse it, and hand back a value that is either there or not.
//! `None` means "this machine has nothing to say about that" - no battery in a
//! desktop, no wireless card - and the caller takes the pill (or the row) away
//! rather than showing a zero that would look like a reading.
//!
//! Nothing here talks to the UI. The parsing is split out from the reading, so
//! what a pill does with an answer can be tested without the hardware. And it
//! lives in the shared half rather than in the bar because the *rest* of the
//! desktop reads the same values - the system popup shows the wireless link the
//! bar's pill does, and neither of them should have its own opinion about what
//! `iw` prints.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use gtk::glib;

// ---------------------------------------------------------------------------
// Volume (PipeWire/WirePlumber, through wpctl)
// ---------------------------------------------------------------------------

/// The default sink's level and mute state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    /// Percent of full scale. wpctl allows over-amplification, so this can be
    /// above 100.
    pub percent: i32,
    pub muted: bool,
}

/// `@DEFAULT_AUDIO_SINK@` - the sink the volume keys act on, so the pill
/// follows whatever output the user is on rather than a device name.
pub const SINK: &str = "@DEFAULT_AUDIO_SINK@";

/// Read the default sink, or `None` when WirePlumber is not there to ask.
pub fn volume() -> Option<Volume> {
    let output = Command::new("wpctl")
        .args(["get-volume", SINK])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_volume(&String::from_utf8_lossy(&output.stdout))
}

/// `Volume: 0.55` or `Volume: 0.55 [MUTED]` - the same two shapes the volume
/// element parses, from the same tool.
fn parse_volume(text: &str) -> Option<Volume> {
    let rest = text.trim().strip_prefix("Volume:")?.trim();
    let volume: f64 = rest.split_whitespace().next()?.parse().ok()?;
    Some(Volume {
        percent: (volume * 100.0).round() as i32,
        muted: rest.contains("[MUTED]"),
    })
}

/// Move the volume by `delta` percent, the way the pill's scroll wheel does.
///
/// This deliberately goes through the volume *element* (`hypr-osd-volume
/// up|down|toggle`) rather than calling `wpctl` here: that element owns the
/// volume step, and one desktop should not have two opinions about what one
/// notch is. It also means scrolling the pill shows the card - the same feedback
/// a key press gets. `fallback` is what to run instead when the element is not
/// installed, so the pill still works on a machine with only the bar.
///
/// Whether the element is there is answered by *looking* for it (see
/// [`installed`]) rather than by running it and waiting for the exit: the first
/// invocation of a single-instance element does not exit, it *becomes* the card
/// (see [`launch`]).
pub fn nudge_volume(program: &str, verb: &str, fallback: &[&str]) {
    if installed(program) {
        launch(program, &[verb]);
        return;
    }
    if let Some((program, args)) = fallback.split_first() {
        launch(program, args);
    }
}

/// Run a command that answers once, and answer whether it succeeded.
///
/// **This waits for the command to exit**, which is only ever right for a tool
/// that answers and leaves (`wpctl`, `playerctl`, `hyprctl`). For anything that
/// might still be on screen when it finishes - an element, a terminal - use
/// [`launch`]: a command that does not exit would block the caller's main loop
/// for as long as its window is up, which is a frozen bar rather than a slow one.
///
/// Its output is its own business, and its stdin is nulled and its stdout closed
/// because this is called from a bar: a command that decides to print or to read
/// would otherwise get a say in how the bar looks.
pub fn run(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Start a command and walk away.
///
/// This is what a bar's *handles* need: clicking the status pill runs the system
/// popup, the power button runs the session card, the clock's hover runs the
/// island, and the wheel runs the volume element. None of those programs exits
/// while it is on screen - each one **is** the daemon behind its surface, and the
/// first invocation of a binary nobody has started yet becomes that daemon
/// instead of a client of it. A caller that waited would therefore be waiting for
/// the *user* to close a popup, with its own main loop parked: the bar's pill
/// click froze the whole bar until the panel was dismissed.
///
/// So: spawn, hand the exit to GLib (which reaps it - a daemon that left zombies
/// behind for the session would be a bug of its own), and return. Nothing here
/// can block, whatever the command decides to do.
pub fn launch(program: &str, args: &[&str]) {
    let spawned = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        // The `Child` is dropped on the spot: dropping one neither kills nor
        // waits, and the wait belongs to GLib's child watch from here on. A
        // non-zero exit is not reported - by the time it happens the card has
        // been and gone, and there is nowhere honest to put the message.
        Ok(child) => {
            glib::child_watch_add_local(glib::Pid(child.id() as i32), |_, _| {});
        }
        Err(error) => eprintln!("hypr-osd: cannot run {program}: {error}"),
    }
}

/// Whether `program` can be run at all, without running it.
///
/// A `$PATH` lookup, for the one decision that has to be made *before* the
/// command starts: whether the volume element is installed, or whether the
/// pill's wheel has to move the sink through `wpctl` itself.
pub fn installed(program: &str) -> bool {
    if program.contains('/') {
        return executable(Path::new(program));
    }
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| executable(&directory.join(program)))
    })
}

/// A file that exists and has some execute bit set - the same test a shell's
/// `command -v` makes.
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

// ---------------------------------------------------------------------------
// Battery (the kernel's own power_supply class)
// ---------------------------------------------------------------------------

/// What the battery is doing, as far as the pill's word and colour go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Charging {
    /// Current is going in.
    Yes,
    /// On the battery.
    No,
    /// Charged and holding.
    Full,
    /// On the mains without charging - the kernel's "Not charging", which is
    /// what a charge limit looks like. On the battery is the more alarming of
    /// the two, so the adapter is what tells them apart.
    Plugged,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Battery {
    pub percent: i32,
    pub charging: Charging,
}

impl Battery {
    /// Plugged in means an amber pill would be crying wolf.
    fn on_mains(&self) -> bool {
        matches!(
            self.charging,
            Charging::Yes | Charging::Full | Charging::Plugged
        )
    }

    /// Below 30 % the pill turns amber, below 15 % red - the two thresholds the
    /// bar this replaces used.
    pub fn warning(&self) -> bool {
        !self.on_mains() && self.percent < 30
    }

    pub fn critical(&self) -> bool {
        !self.on_mains() && self.percent < 15
    }
}

/// Read the battery called `name` (`BAT1`), or - when there is no such device -
/// the first battery the kernel does report, so a rename between machines does
/// not silently remove the pill. `adapter` is the mains supply (`ADP1`), which
/// is what distinguishes "not charging" from "on the battery".
pub fn battery(name: &str, adapter: &str) -> Option<Battery> {
    let dir = supply_dir(name).or_else(|| first_supply("BAT"))?;
    let percent = read_number(&dir.join("capacity"))?;
    let status = fs::read_to_string(dir.join("status")).unwrap_or_default();
    let charging = match parse_charging(&status) {
        Charging::No if plugged_in(adapter) => Charging::Plugged,
        charging => charging,
    };
    Some(Battery { percent, charging })
}

/// Whether the machine is on mains power. With no such supply the answer is
/// "no", which reads as "on battery" - the state that makes a low charge worth
/// warning about.
fn plugged_in(adapter: &str) -> bool {
    supply_dir(adapter)
        .map(|dir| read_number(&dir.join("online")).unwrap_or(0) != 0)
        .unwrap_or(false)
}

/// `Charging` / `Discharging` / `Full` / anything else (the kernel also reports
/// `Not charging`, which is not the same as charging).
fn parse_charging(status: &str) -> Charging {
    let status = status.trim();
    if status.eq_ignore_ascii_case("charging") {
        Charging::Yes
    } else if status.eq_ignore_ascii_case("full") {
        Charging::Full
    } else {
        Charging::No
    }
}

/// `/sys/class/power_supply/<name>`, if the kernel has such a device.
fn supply_dir(name: &str) -> Option<std::path::PathBuf> {
    let dir = Path::new("/sys/class/power_supply").join(name);
    dir.is_dir().then_some(dir)
}

/// The first supply whose directory starts with `prefix`, for a machine whose
/// battery is not called what the config says.
fn first_supply(prefix: &str) -> Option<std::path::PathBuf> {
    let mut entries: Vec<std::path::PathBuf> = fs::read_dir("/sys/class/power_supply")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
        })
        .collect();
    // Stable order, so two batteries resolve the same way on every run.
    entries.sort();
    entries.into_iter().next()
}

fn read_number(path: &Path) -> Option<i32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

// ---------------------------------------------------------------------------
// Network (the kernel's wireless link, through iw)
// ---------------------------------------------------------------------------

/// The wireless link that is up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Network {
    /// The access point's name.
    pub ssid: String,
    /// Signal strength as a percentage, for the pill.
    pub signal: i32,
    /// The same reading in dBm, for the tooltip - the number a human can act on.
    pub dbm: i32,
}

/// Read the active wireless link.
///
/// Two answers, and they are different things:
/// * `Ok(Some(link))` - connected, this is the link;
/// * `Ok(None)` - the interface is there and nothing is connected;
/// * `Err(())` - the caller cannot tell (no wireless interface at all, or no
///   `iw` to ask), so it draws no pill (and no row) for the link rather than a
///   permanent "off" that would misreport hardware which is not there.
///
/// The `Err(())` is deliberate, and clippy is told so: this is a two-way answer,
/// not a failure to report. "There is no wireless card" is not an error, and
/// inventing an error type for it would only move the two-way distinction into a
/// type nobody reads.
#[allow(clippy::result_unit_err)]
pub fn network(interface: &str) -> Result<Option<Network>, ()> {
    if !Path::new("/sys/class/net").join(interface).is_dir() {
        return Err(());
    }
    // `iw` reports the one link that is up, in the kernel's own terms, in one
    // call. (This asked `nmcli` at first, and should not: nmcli's *values* are
    // translated - a field that reads `yes` in English reads `nein` in German -
    // and its scan list does not reliably mark the access point actually in use,
    // so the pill flickered between a percentage and "off".)
    let Ok(output) = Command::new("iw").args(["dev", interface, "link"]).output() else {
        return Err(());
    };
    Ok(parse_iw(&String::from_utf8_lossy(&output.stdout)))
}

/// `iw dev <if> link` prints a paragraph while connected, and `Not connected.`
/// (which is not a link) or an empty, failing answer when it is not.
fn parse_iw(text: &str) -> Option<Network> {
    if !text.contains("Connected to") {
        return None;
    }
    let ssid = field(text, "SSID:")?;
    let dbm: i32 = field(text, "signal:")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some(Network {
        ssid,
        signal: dbm_to_percent(dbm),
        dbm,
    })
}

/// A dBm reading as a percentage: -100 dBm is 0 %, -50 dBm is 100 %. It is the
/// approximation every bar uses - a display convenience, not physics - which is
/// why the tooltip carries the real number.
fn dbm_to_percent(dbm: i32) -> i32 {
    (2 * (dbm + 100)).clamp(0, 100)
}

/// The value behind a `Key:` line of one of `iw`'s paragraphs.
fn field(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(key))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wpctl_volume() {
        assert_eq!(
            parse_volume("Volume: 0.55\n"),
            Some(Volume {
                percent: 55,
                muted: false
            })
        );
        assert_eq!(
            parse_volume("Volume: 1.00 [MUTED]"),
            Some(Volume {
                percent: 100,
                muted: true
            })
        );
        // Over-amplification is a real value, not an error.
        assert_eq!(
            parse_volume("Volume: 1.40"),
            Some(Volume {
                percent: 140,
                muted: false
            })
        );
        assert_eq!(parse_volume("no volume here"), None);
    }

    #[test]
    fn parses_the_battery_state() {
        assert_eq!(parse_charging("Charging\n"), Charging::Yes);
        assert_eq!(parse_charging("Full"), Charging::Full);
        assert_eq!(parse_charging("Discharging"), Charging::No);
        // "Not charging" (plugged in, capped) is *not* charging: the glyph must
        // not claim power is going in.
        assert_eq!(parse_charging("Not charging"), Charging::No);
    }

    #[test]
    fn a_flat_battery_warns_and_a_charged_one_does_not() {
        let battery = |percent, charging| Battery { percent, charging };
        assert!(battery(20, Charging::No).warning());
        assert!(battery(10, Charging::No).critical());
        assert!(!battery(20, Charging::No).critical());
        // On the charger - or holding at a charge limit - a low number is not
        // bad news.
        assert!(!battery(20, Charging::Yes).warning());
        assert!(!battery(20, Charging::Plugged).warning());
        assert!(!battery(100, Charging::Full).warning());
    }

    #[test]
    fn reads_a_connected_link_out_of_iw() {
        let text = "Connected to 6c:d6:e3:77:14:4f (on wlan0)\n\
                    \tSSID: PHSG\n\
                    \tfreq: 5220.0\n\
                    \tsignal: -67 dBm\n\
                    \trx bitrate: 309.7 MBit/s\n";
        let link = parse_iw(text).expect("a connected link");
        assert_eq!(link.ssid, "PHSG");
        assert_eq!(link.dbm, -67);
        assert_eq!(link.signal, 66);
    }

    #[test]
    fn a_disconnected_interface_is_not_a_link() {
        // What `iw` prints with the radio on but nothing joined.
        assert_eq!(parse_iw("Not connected.\n"), None);
        // An interface that is down answers with nothing at all.
        assert_eq!(parse_iw(""), None);
        // A connected line with no signal reading is not a usable link either.
        assert_eq!(parse_iw("Connected to aa:bb (on wlan0)\n\tSSID: x\n"), None);
    }

    #[test]
    fn a_dbm_reading_becomes_a_percentage() {
        assert_eq!(dbm_to_percent(-50), 100);
        assert_eq!(dbm_to_percent(-75), 50);
        assert_eq!(dbm_to_percent(-100), 0);
        // Beyond the ends of the scale it is clamped rather than negative.
        assert_eq!(dbm_to_percent(-30), 100);
        assert_eq!(dbm_to_percent(-120), 0);
    }

    #[test]
    fn launching_a_command_does_not_wait_for_it() {
        // The bug this guards against: the bar's pills used to `run` their
        // handles, and `.status()` waits for the child to exit. Half of those
        // handles *are* the daemon behind the surface they open - the status
        // pill's `hypr-osd-stats toggle` becomes the popup and does not exit
        // while it is up - so clicking one parked the bar's main loop until the
        // user dismissed the panel. `launch` has to return immediately whatever
        // the command decides to do; three seconds of `sleep` would show up here
        // as three seconds of waiting.
        let started = std::time::Instant::now();
        launch("sleep", &["3"]);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "launch waited for its child to exit"
        );
    }

    #[test]
    fn installed_is_a_path_lookup() {
        // `sh` is on any machine this runs on; a name that cannot exist is not.
        assert!(installed("sh"));
        assert!(!installed("hypr-osd-no-such-program"));
    }
}

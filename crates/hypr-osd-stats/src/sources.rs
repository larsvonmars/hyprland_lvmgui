//! The readings the system popup is drawn from.
//!
//! The popup is a *report on the machine*, so almost everything it shows comes
//! from somewhere that already publishes it: `/proc` and `/sys` for the load and
//! the backlight, `bluetoothctl` and `powerprofilesctl` for the radios and the
//! power profile, `rfkill` for whether the wireless radio is on at all, and
//! Hyprland itself for the keyboard layout. None of that is invented here - this
//! module is the parsing, split from the reading, so what the rows say can be
//! tested without the hardware.
//!
//! Two of these overlap with what the bar already reads, and they are shared
//! rather than copied: the load and the update count are
//! [`hypr_osd_core::system`], and the wireless link is
//! [`hypr_osd_core::hardware`]. One desktop, one opinion about what "hot" means.
//! What is here is what *only* the popup needs.
//!
//! Everything slow is asynchronous. The one genuinely slow read is
//! `bluetoothctl`, which has to talk to the bluetooth daemon over D-Bus and can
//! take a few hundred milliseconds; a popup that freezes its own pointer
//! sampling for that long would close late and feel broken. It goes through
//! [`output::read`], which hands the answer back on the main loop.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use hypr_osd_core::hypripc;
use hypr_osd_core::output;

// ---------------------------------------------------------------------------
// Bluetooth (through bluetoothctl)
// ---------------------------------------------------------------------------

/// What the controller is doing, as far as the two rows of the popup go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bluetooth {
    pub powered: bool,
    /// How many devices are connected right now - the number that makes "on"
    /// mean something.
    pub connected: usize,
}

impl Bluetooth {
    /// The state as the BLUETOOTH row reads it.
    pub fn state(&self) -> String {
        match (self.powered, self.connected) {
            (false, _) => "Off".to_string(),
            (true, 0) => "On".to_string(),
            (true, count) => format!("On · {count} connected"),
        }
    }

    /// What the toggle button would do, as the button's own word.
    pub fn action(&self) -> &'static str {
        if self.powered {
            "Turn off"
        } else {
            "Turn on"
        }
    }
}

/// Read the controller state, and hand it to `on_done` on the main loop.
///
/// Two commands, one after the other (`bluetoothctl` answers one question at a
/// time), and both of them slow enough that waiting for them here would stall
/// the pointer sampling that decides whether the popup stays open.
pub fn refresh_bluetooth(on_done: impl FnOnce(Bluetooth) + 'static) {
    output::read("bluetoothctl", &["show".to_string()], move |show| {
        let show = text(show);
        output::read(
            "bluetoothctl",
            &["devices".to_string(), "Connected".to_string()],
            move |connected| on_done(parse_bluetooth(&show, &text(connected))),
        );
    });
}

/// The two answers, parsed. `Powered: yes` in `show`, one line per device in
/// `devices Connected` - and an empty answer (bluetooth off, no daemon) is "off
/// with nothing connected" rather than an error the popup would have to render.
fn parse_bluetooth(show: &str, connected: &str) -> Bluetooth {
    Bluetooth {
        powered: show.lines().any(|line| line.trim() == "Powered: yes"),
        connected: connected
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count(),
    }
}

/// Switch the controller on or off.
///
/// `bluetoothctl` has no toggle subcommand, so the state has to be known first -
/// which the popup has, from the last refresh. Fire and forget: the answer is
/// read back the same way.
pub fn set_bluetooth(powered: bool) {
    spawn(
        "bluetoothctl",
        &["power", if powered { "on" } else { "off" }],
    );
}

// ---------------------------------------------------------------------------
// Power profile (power-profiles-daemon)
// ---------------------------------------------------------------------------

/// The profiles in the order the button cycles them, quietest first.
pub const PROFILES: [&str; 3] = ["power-saver", "balanced", "performance"];

/// The current profile, or `None` when the daemon does not answer (not
/// installed, or the machine has no platform profile).
pub fn power_profile() -> Option<String> {
    capture("powerprofilesctl", &["get"]).filter(|profile| !profile.is_empty())
}

/// What a click on the profile button would set next.
pub fn next_profile(current: Option<&str>) -> &'static str {
    match current.and_then(|current| PROFILES.iter().position(|p| *p == current)) {
        Some(index) => PROFILES[(index + 1) % PROFILES.len()],
        // Anything unknown (or nothing at all) starts from the middle: it is the
        // one profile every machine with the daemon has.
        None => "balanced",
    }
}

pub fn set_power_profile(profile: &str) {
    spawn("powerprofilesctl", &["set", profile]);
}

// ---------------------------------------------------------------------------
// Presentation mode (a systemd inhibitor)
// ---------------------------------------------------------------------------

/// The "presentation mode" toggle: a `systemd-inhibit` child holding an
/// idle/sleep block for as long as it lives.
///
/// This is the old bar's `idle_inhibitor` module moved into the card, with one
/// deliberate difference: the child is *ours*, so leaving the popup or quitting
/// the element releases the block. The script this replaces started an orphan
/// `sleep infinity` that outlived its window - which meant an inhibitor nobody
/// could see or stop, still blocking sleep an hour later.
#[derive(Default)]
pub struct Inhibitor {
    child: Option<Child>,
}

impl Inhibitor {
    pub fn active(&self) -> bool {
        self.child.is_some()
    }

    pub fn toggle(&mut self) {
        if self.active() {
            self.release();
        } else {
            self.acquire();
        }
    }

    /// Let go of the block, if there is one.
    pub fn release(&mut self) {
        if let Some(mut child) = self.child.take() {
            // Killing `sleep` is what ends the inhibitor; systemd takes the lock
            // away with the process.
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn acquire(&mut self) {
        let spawned = Command::new("systemd-inhibit")
            .args([
                "--what=idle:sleep",
                // The name systemd shows in `systemd-inhibit --list`, so it is
                // obvious which button put the lock there.
                "--who=hypr-osd-stats",
                "--why=Presentation",
                "--mode=block",
                "sleep",
                "infinity",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(child) => self.child = Some(child),
            Err(error) => eprintln!("hypr-osd-stats: cannot inhibit: {error}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Brightness (the kernel's backlight class, through brightnessctl)
// ---------------------------------------------------------------------------

/// Where the kernel keeps the backlights.
const BACKLIGHT_DIR: &str = "/sys/class/backlight";

/// The panel brightness in percent, or `None` on a machine whose backlight
/// cannot be read (a desktop, a VM).
pub fn brightness() -> Option<i32> {
    let dir = backlight_dir()?;
    let current: f64 = read_number(&dir.join("brightness"))?;
    let maximum: f64 = read_number(&dir.join("max_brightness"))?;
    backlight_percent(current, maximum)
}

/// The panel's own directory: `intel_backlight` where there is one (this
/// machine), otherwise whatever the kernel did register - the name is a driver
/// detail, and hard-coding it would silently turn the row off on another laptop.
fn backlight_dir() -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(BACKLIGHT_DIR)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    entries.sort();
    if let Some(preferred) = entries.iter().find(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains("intel_backlight"))
    }) {
        return Some(preferred.clone());
    }
    entries.into_iter().next()
}

/// Current over maximum, rounded to a percentage.
fn backlight_percent(current: f64, maximum: f64) -> Option<i32> {
    if maximum <= 0.0 {
        return None;
    }
    Some((current * 100.0 / maximum).round().clamp(0.0, 100.0) as i32)
}

/// Set the backlight. `-e4 -n2` is the same invocation the bar this replaces
/// used: exponent 4 (the kernel's own curve is not perceptually even), and
/// `-n2` so that a value that reads as the same percentage is not written
/// again.
pub fn set_brightness(percent: i32) {
    spawn(
        "brightnessctl",
        &["-e4", "-n2", "set", &format!("{}%", percent.clamp(1, 100))],
    );
}

// ---------------------------------------------------------------------------
// The wireless radio (through rfkill)
// ---------------------------------------------------------------------------

/// Whether the wireless radio is on at all.
///
/// `rfkill` rather than `nmcli radio wifi`: nmcli *translates its values*, so a
/// bar that greps its output for "enabled" reads "deaktiviert" on a German
/// machine and gets the answer wrong. rfkill's `Soft blocked: yes/no` is
/// kernel-defined and never localised.
pub fn radio_on() -> bool {
    parse_radio(&capture("rfkill", &["list", "wifi"]).unwrap_or_default())
}

fn parse_radio(text: &str) -> bool {
    // Anything unreadable is "on": a machine with no wireless card has nothing
    // to switch, and claiming the radio is off would be a lie with a button
    // attached to it.
    !text.lines().any(|line| {
        line.trim()
            .to_ascii_lowercase()
            .starts_with("soft blocked: yes")
            || line
                .trim()
                .to_ascii_lowercase()
                .starts_with("hard blocked: yes")
    })
}

/// Switch the wireless radio. `nmcli` is the tool that can do it; only its
/// *output* is untrustworthy, never its arguments.
pub fn set_radio(on: bool) {
    spawn("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]);
}

// ---------------------------------------------------------------------------
// Keyboard layout (through Hyprland)
// ---------------------------------------------------------------------------

/// The active layout as a short code, e.g. `DE`.
///
/// `j/devices` reports the first keyboard's `active_keymap`, which is the full
/// XKB name - `German (Switzerland)`, not `ch`. That is what the old
/// `hyprland/language` module read too, and its `de` came from XKB's
/// short description, which hyprctl does not expose at all. So the language word
/// is mapped instead, and [`LAYOUT_CODES`] grows as layouts are added to the
/// compositor's config.
pub fn layout() -> String {
    let Some(devices) = hypripc::json("j/devices") else {
        return "?".to_string();
    };
    let keymap = devices
        .get("keyboards")
        .and_then(|keyboards| keyboards.get(0))
        .and_then(|keyboard| keyboard.get("active_keymap"))
        .and_then(|keymap| keymap.as_str())
        .unwrap_or_default();
    layout_code(keymap)
}

/// The language word of an XKB keymap name, as the two-letter code the bar shows.
fn layout_code(keymap: &str) -> String {
    if keymap.is_empty() {
        return "?".to_string();
    }
    let language = keymap.split('(').next().unwrap_or_default().trim();
    match LAYOUT_CODES.iter().find(|(name, _)| *name == language) {
        Some((_, code)) => (*code).to_string(),
        // An unmapped layout shows its full name rather than a wrong code.
        None => keymap.to_string(),
    }
}

const LAYOUT_CODES: [(&str, &str); 8] = [
    ("English", "EN"),
    ("German", "DE"),
    ("French", "FR"),
    ("Italian", "IT"),
    ("Spanish", "ES"),
    ("Portuguese", "PT"),
    ("Dutch", "NL"),
    ("Swiss", "CH"),
];

// ---------------------------------------------------------------------------
// Running things
// ---------------------------------------------------------------------------

/// Start a command and walk away, the way the popup's buttons do: what changes
/// comes back through the next refresh.
///
/// Detached on purpose, and *not* [`hypr_osd_core::hardware::run`] (which waits):
/// half of these open a terminal (`btop`, `bluetoothctl`, `nmtui`, `pacman`) and
/// waiting for one would block the popup for as long as its window is open -
/// including the pointer sampling that decides whether it should still be up.
pub fn spawn(program: &str, args: &[&str]) {
    if let Err(error) = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        eprintln!("hypr-osd-stats: cannot run {program}: {error}");
    }
}

/// Run a command that answers once, and hand back its trimmed stdout when it
/// succeeded. Used for the short reads (`powerprofilesctl get`, `rfkill`) where
/// waiting a few milliseconds is exactly what a `status` read should do.
fn capture(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn text(bytes: Option<Vec<u8>>) -> String {
    bytes
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

fn read_number(path: &Path) -> Option<f64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_controller_that_answers_reads_as_powered() {
        let show = "Controller 00:1A:7D:DA:71:13 (public)\n\tName: lars\n\tPowered: yes\n";
        let devices = "Device AA:BB:CC:DD:EE:FF Keyboard\n\nDevice 11:22:33:44:55:66 Buds\n";
        let bluetooth = parse_bluetooth(show, devices);
        assert!(bluetooth.powered);
        assert_eq!(bluetooth.connected, 2);
        assert_eq!(bluetooth.state(), "On · 2 connected");
        assert_eq!(bluetooth.action(), "Turn off");
    }

    #[test]
    fn a_controller_that_says_nothing_is_off() {
        let bluetooth = parse_bluetooth("", "");
        assert!(!bluetooth.powered);
        assert_eq!(bluetooth.connected, 0);
        assert_eq!(bluetooth.state(), "Off");
        assert_eq!(bluetooth.action(), "Turn on");
    }

    #[test]
    fn a_powered_controller_with_nothing_attached_says_so() {
        assert_eq!(
            parse_bluetooth("Powered: yes\n", "").state(),
            "On",
            "a zero would read as a reading"
        );
    }

    #[test]
    fn the_profile_button_cycles_through_all_three_and_wraps() {
        assert_eq!(next_profile(Some("power-saver")), "balanced");
        assert_eq!(next_profile(Some("balanced")), "performance");
        assert_eq!(next_profile(Some("performance")), "power-saver");
    }

    #[test]
    fn an_unknown_profile_starts_from_the_middle() {
        assert_eq!(next_profile(None), "balanced");
        assert_eq!(next_profile(Some("something-else")), "balanced");
    }

    #[test]
    fn a_backlight_is_a_percentage_of_what_it_can_do() {
        assert_eq!(backlight_percent(96000.0, 96000.0), Some(100));
        assert_eq!(backlight_percent(0.0, 96000.0), Some(0));
        assert_eq!(backlight_percent(24000.0, 96000.0), Some(25));
    }

    #[test]
    fn a_backlight_that_claims_no_maximum_is_no_reading() {
        assert_eq!(backlight_percent(100.0, 0.0), None);
    }

    #[test]
    fn a_blocked_radio_reads_as_off() {
        let blocked = "1: phy0: Wireless LAN\n\tSoft blocked: yes\n\tHard blocked: no\n";
        assert!(!parse_radio(blocked));
        let on = "1: phy0: Wireless LAN\n\tSoft blocked: no\n\tHard blocked: no\n";
        assert!(parse_radio(on));
    }

    #[test]
    fn a_machine_without_a_radio_is_not_reported_as_switched_off() {
        // No rfkill answer at all: nothing to switch, so the button must not
        // claim the radio is off.
        assert!(parse_radio(""));
    }

    #[test]
    fn a_layout_is_the_language_it_is_named_after() {
        assert_eq!(layout_code("German (Switzerland)"), "DE");
        assert_eq!(layout_code("English (US)"), "EN");
        // A bare name, an unmapped name and nothing at all.
        assert_eq!(layout_code("German"), "DE");
        assert_eq!(layout_code("Klingon (Qo'noS)"), "Klingon (Qo'noS)");
        assert_eq!(layout_code(""), "?");
    }
}

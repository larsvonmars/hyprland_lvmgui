//! The machine's own readings: CPU load, memory in use, package temperature, and
//! how many updates are waiting.
//!
//! Two elements show them and neither should invent them twice: the bar's system
//! pill (three numbers and a count, at a glance) and the system popup (the same
//! numbers with bars, the package list, and the controls that change things). So
//! the readings live here and the elements decide what to do with them.
//!
//! All four are read the way the rest of this collection reads things - from the
//! kernel directly, or from the tool that owns the answer - with the parsing split
//! out so it can be tested without the hardware:
//!
//! * **CPU** is two readings of `/proc/stat` apart: the counter is monotonic
//!   since boot, so the *delta* between two ticks is the answer, and a single
//!   reading only ever says "the average since you turned the machine on".
//! * **Memory** is `MemTotal - MemAvailable` from `/proc/meminfo`, which is what
//!   "in use" means to the kernel (it already accounts for reclaimable caches).
//! * **Temperature** is the package sensor under `coretemp`, in millidegrees.
//!   It is the one reading that can simply be absent (no such driver, a VM), and
//!   an absent sensor shows as no number rather than as 0°.
//! * **Updates** come from `checkupdates`, which syncs a throwaway pacman
//!   database and takes a second or two - far too slow to run on a bar's
//!   heartbeat. So its answer is *cached in a file* for half an hour, and the
//!   cache is the same one the old waybar module used, so the two agree and a
//!   cache written either way is usable by both.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::output;

/// How long an update count is believed. Past this it is not a reading any more,
/// it is history - and `checkupdates` runs again.
pub const UPDATES_MAX_AGE: Duration = Duration::from_secs(1800);

// The thresholds the bar this replaces used, unchanged: amber at the first, red
// at the second, whichever of the three readings crosses it first. They are here
// rather than in an element because two elements colour themselves by them, and
// the desktop should not have two opinions about what "hot" means.
pub const CPU_WARN: f32 = 60.0;
pub const CPU_CRIT: f32 = 85.0;
pub const MEM_WARN: f32 = 70.0;
pub const MEM_CRIT: f32 = 90.0;
pub const TEMP_WARN: f32 = 75.0;
pub const TEMP_CRIT: f32 = 90.0;

/// One reading of the three fast sensors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// Percent of the CPU, averaged over the interval between two ticks.
    pub cpu: f32,
    /// Percent of memory in use.
    pub memory: f32,
    /// Package temperature in °C. `None` when there is no sensor to read.
    pub temperature: Option<f32>,
}

impl Reading {
    pub fn warning(&self) -> bool {
        self.cpu >= CPU_WARN
            || self.memory >= MEM_WARN
            || self.temperature.is_some_and(|t| t >= TEMP_WARN)
    }

    pub fn critical(&self) -> bool {
        self.cpu >= CPU_CRIT
            || self.memory >= MEM_CRIT
            || self.temperature.is_some_and(|t| t >= TEMP_CRIT)
    }
}

/// The CPU counters from one reading of `/proc/stat`.
///
/// `busy` is everything except idle and iowait - a machine waiting on its disk
/// is not busy, which is the same accounting `top` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuTimes {
    busy: u64,
    total: u64,
}

/// Reads the sensors, remembering the last CPU sample.
///
/// The first reading established in [`Sampler::new`] is what makes the first
/// *answer* possible one tick later - otherwise the pill would show nothing for
/// a second (or, worse, a bogus 0 %, which is what "no baseline yet" looks like
/// when it is printed as a number).
pub struct Sampler {
    previous: Option<CpuTimes>,
}

impl Sampler {
    pub fn new() -> Self {
        Sampler {
            previous: cpu_times(),
        }
    }

    /// Read everything. `cpu` is the load since the previous call.
    pub fn read(&mut self) -> Reading {
        let current = cpu_times();
        let cpu = match (self.previous, current) {
            (Some(previous), Some(current)) => {
                let percent = cpu_percent(previous, current).unwrap_or(0.0);
                self.previous = Some(current);
                percent
            }
            // Either the counter was unreadable (not Linux, or /proc missing) or
            // this is the very first call: report nothing rather than a number
            // that was not measured.
            _ => {
                self.previous = current;
                0.0
            }
        };
        Reading {
            cpu,
            memory: memory_used().unwrap_or(0.0),
            temperature: temperature(),
        }
    }
}

impl Default for Sampler {
    fn default() -> Self {
        Sampler::new()
    }
}

/// How far apart the two samples of a one-off reading are. Long enough that the
/// counters have moved, short enough that a `status` verb still feels instant.
const SAMPLE_GAP: Duration = Duration::from_millis(200);

/// A reading with a real CPU number in it.
///
/// The heartbeat does not need this - it has the previous tick as its baseline,
/// so its first answer is one tick away and free. This is for the one-off reads:
/// a `status` verb whose answer should be true when it is printed, even though
/// it costs a fifth of a second.
///
pub fn read_now() -> Reading {
    let mut sampler = Sampler::new();
    std::thread::sleep(SAMPLE_GAP);
    sampler.read()
}

/// The aggregate CPU counters, or `None` when `/proc/stat` cannot be read.
fn cpu_times() -> Option<CpuTimes> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    parse_cpu_stat(&text)
}

/// The first line of `/proc/stat`: `cpu  user nice system idle iowait …`.
fn parse_cpu_stat(text: &str) -> Option<CpuTimes> {
    let line = text.lines().next()?;
    let fields = line.split_whitespace();
    let mut values = fields.skip_while(|field| !field.chars().all(|c| c.is_ascii_digit()));
    let mut numbers = Vec::new();
    for value in values.by_ref() {
        match value.parse::<u64>() {
            Ok(number) => numbers.push(number),
            Err(_) => break,
        }
    }
    // user nice system idle iowait - iowait was added later, and a kernel
    // without it would otherwise look permanently busy.
    if numbers.len() < 4 {
        return None;
    }
    let idle = numbers[3] + numbers.get(4).copied().unwrap_or(0);
    let total: u64 = numbers.iter().sum();
    Some(CpuTimes {
        busy: total.saturating_sub(idle),
        total,
    })
}

/// The load between two readings, as a percentage. `None` when no time passed
/// (two calls within the same jiffy), which would otherwise be a divide by zero.
fn cpu_percent(previous: CpuTimes, current: CpuTimes) -> Option<f32> {
    let busy = current.busy.checked_sub(previous.busy)?;
    let total = current.total.checked_sub(previous.total)?;
    if total == 0 {
        return None;
    }
    Some((busy as f32 * 100.0 / total as f32).clamp(0.0, 100.0))
}

/// Memory in use, as a percentage.
fn memory_used() -> Option<f32> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_meminfo(&text)
}

/// `MemTotal` and `MemAvailable` out of `/proc/meminfo`.
fn parse_meminfo(text: &str) -> Option<f32> {
    let value = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|number| number.parse::<u64>().ok())
    };
    let total = value("MemTotal:")?;
    // `MemAvailable` is the kernel's own estimate of what a new workload could
    // get without swapping, so "used" is what is left over. (A kernel from
    // before 3.14 has no such field; there is nothing honest to report then.)
    let available = value("MemAvailable:")?;
    if total == 0 {
        return None;
    }
    Some(((total - available.min(total)) as f32 * 100.0 / total as f32).clamp(0.0, 100.0))
}

/// The package temperature, or `None` when this machine has no such sensor.
///
/// The paths are sorted so that the same sensor is read on every run: `glob`
/// order is directory order, and a pill whose number jumps between two sensors
/// every time it starts is worse than one that picks the first and stays there.
fn temperature() -> Option<f32> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir("/sys/devices/platform/coretemp.0/hwmon")
        .ok()?
        .flatten()
        .map(|entry| entry.path().join("temp1_input"))
        .collect();
    paths.sort();
    for path in paths {
        if let Some(millidegrees) = read_number(&path) {
            return Some(millidegrees / 1000.0);
        }
    }
    None
}

fn read_number(path: &Path) -> Option<f32> {
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<f32>()
        .ok()
}

// ---------------------------------------------------------------------------
// Pending updates
// ---------------------------------------------------------------------------

/// The count cache: the same file the waybar module wrote, so that going back to
/// it finds a warm cache, and so an `updates.sh` run by hand is picked up here.
pub fn cache_path() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    runtime.join("waybar-checkupdates.cache")
}

/// The cached count, or `None` when there is no cache and when the one there is
/// has gone stale (`max_age`). `None` means "ask `checkupdates` again", not
/// "no updates" - the two are different answers and the pill shows neither.
pub fn cached_updates() -> Option<i32> {
    cached_lines().map(|lines| lines.len() as i32)
}

/// The cached package list - `checkupdates` prints one package per line, so the
/// count is its length. The bar's pill only needs the number; the system popup
/// lists a few names, which is what makes the number mean something.
///
/// An empty list means exactly what a `None` count means: there is nothing
/// *believable* cached. A caller that has to tell "no cache" from "no updates"
/// apart - the bar's pill does, to know when to ask again - uses
/// [`cached_updates`] and gets the `Option`.
pub fn cached_lines() -> Option<Vec<String>> {
    lines_in(&cache_path(), UPDATES_MAX_AGE)
}

fn lines_in(path: &Path, max_age: Duration) -> Option<Vec<String>> {
    let age = std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .elapsed()
        .ok()?;
    if age >= max_age {
        return None;
    }
    Some(non_empty_lines(&std::fs::read_to_string(path).ok()?))
}

/// Run `checkupdates`, count what it printed, and leave the answer in the cache.
///
/// Asynchronous like everything else that runs a command here: `checkupdates`
/// syncs a pacman database over the network, and a bar that blocked on that
/// would freeze for as long as the mirror takes.
pub fn refresh_updates(command: &str, on_done: impl FnOnce(i32) + 'static) {
    let path = cache_path();
    output::read(command, &[], move |output| {
        let text = output
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        let count = non_empty_lines(&text).len() as i32;
        // A failed run writes an empty cache rather than leaving a stale count
        // behind for the next half hour - an empty file is the honest answer to
        // "how many updates are there" when the question could not be asked.
        let _ = std::fs::write(&path, &text);
        on_done(count);
    });
}

/// Non-empty lines, which is what `checkupdates` prints one package per line.
fn non_empty_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape `/proc/stat` really has, on a four-CPU machine.
    const STAT: &str = "\
cpu  4705 356 1105 220235 382 0 148 0 0 0
cpu0 1198 92 287 54989 98 0 45 0 0 0
intr 1234567
";

    #[test]
    fn reads_the_aggregate_cpu_line() {
        let times = parse_cpu_stat(STAT).expect("a cpu line");
        // user+nice+system+idle+iowait+irq+softirq+steal = 226931
        assert_eq!(times.total, 226931);
        // Everything but idle and iowait.
        assert_eq!(times.busy, 226931 - 220235 - 382);
        // And the per-CPU lines below it are not mistaken for the total.
        assert!(parse_cpu_stat("intr 5\n").is_none());
    }

    #[test]
    fn cpu_load_is_the_delta_between_two_readings() {
        let before = parse_cpu_stat("cpu  100 0 100 800 0 0 0 0 0 0").unwrap();
        // 100 more busy jiffies out of 200 that passed: half the time.
        let after = parse_cpu_stat("cpu  200 0 100 900 0 0 0 0 0 0").unwrap();
        let percent = cpu_percent(before, after).expect("a percentage");
        assert!((percent - 50.0).abs() < 0.001, "{percent}");
    }

    #[test]
    fn a_counter_that_did_not_move_is_not_a_division_by_zero() {
        let times = parse_cpu_stat(STAT).unwrap();
        assert_eq!(cpu_percent(times, times), None);
    }

    #[test]
    fn an_idle_machine_reads_zero() {
        let before = parse_cpu_stat("cpu  1 0 1 100 0 0 0 0 0 0").unwrap();
        let after = parse_cpu_stat("cpu  1 0 1 200 0 0 0 0 0 0").unwrap();
        assert_eq!(cpu_percent(before, after), Some(0.0));
    }

    #[test]
    fn reads_memory_in_use() {
        // 16 GiB total, 12 GiB of it available: a quarter in use.
        let meminfo = "MemTotal:       16384000 kB\nMemFree:         2000000 kB\nMemAvailable:   12288000 kB\n";
        let used = parse_meminfo(meminfo).expect("a percentage");
        assert!((used - 25.0).abs() < 0.001, "{used}");
    }

    #[test]
    fn a_meminfo_without_an_available_field_reports_nothing() {
        // Kernels before 3.14 have no `MemAvailable`, and guessing from
        // `MemFree` would report every cache page as used.
        assert_eq!(parse_meminfo("MemTotal: 100 kB\nMemFree: 10 kB\n"), None);
        assert_eq!(parse_meminfo(""), None);
    }

    #[test]
    fn counts_the_lines_checkupdates_printed() {
        let count = |text: &str| non_empty_lines(text).len();
        assert_eq!(count(""), 0);
        assert_eq!(count("\n\n"), 0);
        assert_eq!(count("linux 6.1\nmesa 24.0\n"), 2);
        // A trailing newline is not a package.
        assert_eq!(count("linux 6.1\n"), 1);
        // And the lines are kept whole, which is what the popup's tile lists.
        assert_eq!(non_empty_lines("linux 6.1\nmesa 24.0\n")[1], "mesa 24.0");
    }

    #[test]
    fn a_stale_cache_is_not_an_answer() {
        let path = std::env::temp_dir().join("hypr-osd-updates-test.cache");
        std::fs::write(&path, "linux 6.1\nmesa 24.0\n").unwrap();
        let count = |path: &Path, max_age| lines_in(path, max_age).map(|lines| lines.len());
        // Fresh: believed, and the names are there for the tile to list.
        assert_eq!(count(&path, Duration::from_secs(1800)), Some(2));
        // Anything older than the limit is history, not a reading.
        assert_eq!(count(&path, Duration::ZERO), None);
        // And a cache that is not there at all is the same "ask again".
        assert_eq!(
            count(
                &std::env::temp_dir().join("hypr-osd-no-such.cache"),
                UPDATES_MAX_AGE
            ),
            None
        );
    }

    #[test]
    fn the_cached_names_are_the_packages() {
        let path = std::env::temp_dir().join("hypr-osd-names-test.cache");
        std::fs::write(&path, "linux 6.1\n\nmesa 24.0\n").unwrap();
        let lines = lines_in(&path, UPDATES_MAX_AGE).expect("a fresh cache");
        // The blank line is not a package, and every line is kept whole: it is
        // the popup's tile that takes the first word off each one.
        assert_eq!(
            lines,
            vec!["linux 6.1".to_string(), "mesa 24.0".to_string()]
        );
    }

    #[test]
    fn the_thresholds_are_the_ones_the_old_pill_used() {
        let reading = |cpu, memory, temperature| Reading {
            cpu,
            memory,
            temperature,
        };
        assert!(!reading(10.0, 20.0, Some(40.0)).warning());
        assert!(reading(60.0, 20.0, Some(40.0)).warning());
        assert!(reading(10.0, 70.0, Some(40.0)).warning());
        assert!(reading(10.0, 20.0, Some(75.0)).warning());
        assert!(reading(85.0, 20.0, Some(40.0)).critical());
        assert!(reading(10.0, 90.0, Some(40.0)).critical());
        assert!(reading(10.0, 20.0, Some(90.0)).critical());
        // No sensor is not a hot sensor.
        assert!(!reading(10.0, 20.0, None).warning());
    }
}

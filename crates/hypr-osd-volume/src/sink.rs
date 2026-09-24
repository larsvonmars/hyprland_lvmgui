//! The default audio sink, read and written through `wpctl`.
//!
//! PipeWire/WirePlumber's own CLI - the same tool the bar's volume pill and the
//! previous `XF86Audio*` keybindings used, so the OSD cannot disagree with them
//! about what the volume is. Reading is one cheap subprocess (single-digit
//! milliseconds); a native PipeWire client would mean linking libpipewire to
//! fetch two numbers.

/// A snapshot of the default sink.
pub struct Sink {
    /// 0.0 is silence, 1.0 is full scale. Can exceed 1.0: wpctl allows
    /// over-amplification on purpose, which is why the element has a
    /// `max_percent` ceiling instead of a hard-coded 100 %.
    pub volume: f64,
    pub muted: bool,
}

/// `@DEFAULT_AUDIO_SINK@` - the sink the desktop's volume keys act on, so the
/// OSD follows the user's current output rather than a device name.
const TARGET: &str = "@DEFAULT_AUDIO_SINK@";

/// The current state of the default sink.
pub fn read() -> Result<Sink, String> {
    let output = wpctl(&["get-volume", TARGET])?;
    parse(&output).ok_or_else(|| format!("could not read the volume from wpctl: {}", output.trim()))
}

/// Raise or lower by `delta` (a fraction of full scale, e.g. 0.05), clamped to
/// `max`.
///
/// With `unmute`, a raise also unmutes: a volume key that changes nothing
/// audible is the stock keybindings' best-known annoyance, and hitting "up" on
/// a muted sink is an unambiguous request for sound.
pub fn nudge(delta: f64, max: f64, unmute: bool) -> Result<Sink, String> {
    let before = read()?;
    let target = (before.volume + delta).clamp(0.0, max.max(0.0));
    // Already at the end of the range (or muted at 0 % and going down): don't
    // spawn wpctl just to write the value it already has.
    if (target - before.volume).abs() < f64::EPSILON {
        return Ok(before);
    }
    set_volume(target)?;
    let muted = unmute && before.muted && target > before.volume;
    if muted {
        set_mute(false)?;
    }
    Ok(confirm(Sink {
        volume: target,
        muted: !muted && before.muted,
    }))
}

/// Set an absolute level, given in percent of full scale.
pub fn set_percent(percent: f64, max: f64, unmute: bool) -> Result<Sink, String> {
    let target = (percent / 100.0).clamp(0.0, max.max(0.0));
    let before = read()?;
    set_volume(target)?;
    let muted = unmute && before.muted && target > 0.0;
    if muted {
        set_mute(false)?;
    }
    Ok(confirm(Sink {
        volume: target,
        muted: !muted && before.muted,
    }))
}

/// Toggle mute, keeping the level.
pub fn toggle_mute() -> Result<Sink, String> {
    let before = read()?;
    set_mute(!before.muted)?;
    Ok(confirm(Sink {
        volume: before.volume,
        muted: !before.muted,
    }))
}

/// The percentage shown in the card: absolute, like the bar's `{volume}%`.
pub fn percent(sink: &Sink) -> i32 {
    (sink.volume * 100.0).round() as i32
}

fn set_volume(volume: f64) -> Result<(), String> {
    // Three decimals is all wpctl keeps anyway.
    wpctl(&["set-volume", TARGET, &format!("{volume:.3}")]).map(|_| ())
}

fn set_mute(muted: bool) -> Result<(), String> {
    wpctl(&["set-mute", TARGET, if muted { "1" } else { "0" }]).map(|_| ())
}

/// wpctl is the authority - it clamps and rounds - so the state shown in the
/// card is read back rather than assumed.
fn confirm(guess: Sink) -> Sink {
    read().unwrap_or(guess)
}

/// `Volume: 0.55` or `Volume: 0.55 [MUTED]`.
fn parse(output: &str) -> Option<Sink> {
    let rest = output.trim().strip_prefix("Volume:")?.trim();
    let volume = rest.split_whitespace().next()?.parse().ok()?;
    Some(Sink {
        volume,
        muted: rest.contains("[MUTED]"),
    })
}

fn wpctl(args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("wpctl")
        .args(args)
        .output()
        .map_err(|error| {
            format!("could not run wpctl ({error}) - is PipeWire/WirePlumber installed?")
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "wpctl {} failed: {}",
            args.join(" "),
            stderr.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wpctl_output() {
        assert_eq!(parse("Volume: 0.55\n").map(|s| s.muted), Some(false));
        assert_eq!(parse("Volume: 0.55\n").map(|s| s.volume), Some(0.55));
        assert_eq!(parse("Volume: 0.55 [MUTED]\n").map(|s| s.muted), Some(true));
        assert_eq!(parse("Volume: 1.00 [MUTED]").map(|s| s.volume), Some(1.0));
        assert!(parse("no volume here").is_none());
    }
}

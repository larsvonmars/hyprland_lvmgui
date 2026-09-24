//! Talking to Hyprland.
//!
//! Everything in this collection that needs to know what the compositor knows -
//! where the focused output is, which windows exist, whether a dispatch went
//! through - asks `hyprctl`, the same way audio goes through `wpctl` and media
//! through `playerctl`: no bindings to keep in step with a moving target, and
//! `hyprctl -j` is a stable interface with a documented shape.

use std::process::Command;

use serde_json::Value;

/// Run `hyprctl -j <args>` and parse the answer, or `None` when hyprctl is
/// missing, failed, or answered something that is not JSON.
pub fn json(args: &[&str]) -> Option<Value> {
    let mut full = Vec::with_capacity(args.len() + 1);
    full.push("-j");
    full.extend_from_slice(args);
    let output = Command::new("hyprctl").args(&full).output().ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

/// Run one dispatcher, and answer whether Hyprland did it.
///
/// The argument is passed through as given, because the right *dialect* depends
/// on the Hyprland in front of it: since 0.55 a Hyprland is configured in Lua
/// and `hyprctl dispatch` parses its argument as a Lua expression, while an
/// older one wants the legacy `name args` string. Callers that have to work on
/// both try the Lua form first, like [`focus_window`] does.
///
/// Only an `ok` counts: a dispatch that Hyprland refused prints its complaint
/// instead, and may still exit 0 - which is exactly why the answer is matched
/// rather than the exit status.
pub fn dispatch(argument: &str) -> bool {
    let Ok(output) = Command::new("hyprctl")
        .args(["dispatch", argument])
        .output()
    else {
        return false;
    };
    output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "ok"
}

/// Focus a window by its address (`0x…`), in whichever dialect this Hyprland
/// speaks.
pub fn focus_window(address: &str) -> bool {
    dispatch(&format!(
        "hl.dsp.focus({{ window = \"address:{address}\" }})"
    )) || dispatch(&format!("focuswindow address:{address}"))
}

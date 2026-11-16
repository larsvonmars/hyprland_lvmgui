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

/// Switch to a workspace by id or name (`3`, `special:magic`).
///
/// Numbers are passed as numbers, because that is how a Hyprland configured in
/// Lua spells them (`hl.dsp.focus({ workspace = 3 })`, exactly what the example
/// config's workspace binds use); a name goes through as a string
/// (`{ workspace = "special:magic" }`).
pub fn focus_workspace(name: &str) -> bool {
    dispatch(&format!("hl.dsp.focus({{ workspace = {} }})", target(name)))
        || dispatch(&format!("workspace {name}"))
}

/// Move one window to one workspace and **stay on the one you are on**.
///
/// The `follow` flag is the whole difference between what a key press means and
/// what a drop means: `window.move` on its own also switches the focused monitor
/// to the target, which is right for `SUPER + SHIFT + 3` and wrong for a window
/// dragged onto a workspace chip - that is a statement about where the window
/// belongs, not about where the person does.
///
/// The window is named by address, because the dispatcher would otherwise move
/// *the focused* window, which is a different window from the one that was
/// dragged.
pub fn move_window(address: &str, workspace: &str) -> bool {
    dispatch(&format!(
        "hl.dsp.window.move({{ workspace = {}, window = \"address:{address}\", follow = false }})",
        target(workspace)
    )) || dispatch(&format!(
        "movetoworkspacesilent {workspace},address:{address}"
    ))
}

/// A workspace as a dispatcher wants it: a number for a numbered workspace, a
/// quoted string for a name (`special:magic`, `code`).
fn target(workspace: &str) -> String {
    match workspace.parse::<i32>() {
        Ok(id) => id.to_string(),
        Err(_) => format!("\"{workspace}\""),
    }
}

/// One of Hyprland's own options, as it stands *now*: the value it is using, and
/// whether the config states it or it is the built-in default.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reading {
    pub value: String,
    pub set: bool,
}

/// Ask Hyprland what one option is worth right now (`general:gaps_in`).
///
/// `None` means "this compositor has no such option" - either the path is wrong
/// or it is from another Hyprland version. The settings window shows the answer
/// next to a key it manages, so a value that is set elsewhere (in the user's own
/// `hyprland.lua`, say) is visible instead of being silently overridden.
pub fn option(path: &str) -> Option<Reading> {
    parse_option(&json(&["getoption", path])?)
}

/// Run a Lua expression in the compositor (`hyprctl eval '…'`).
///
/// `Ok` carries whatever Hyprland printed, which for a successful `hl.config` is
/// usually nothing at all; `Err` carries its complaint, and for an unknown option
/// that complaint *names the option* - which is the whole reason the answer is
/// read rather than the exit status alone. Used by the settings window (whose
/// Hyprland tab hands over a generated document) and by the energy daemon (whose
/// document carries the dropped effects).
pub fn eval(expression: &str) -> Result<String, String> {
    let answer = Command::new("hyprctl")
        .args(["eval", expression])
        .output()
        .map_err(|error| format!("cannot run hyprctl: {error}"))?;
    let said = String::from_utf8_lossy(&answer.stdout).trim().to_string();
    if answer.status.success() {
        return Ok(said);
    }
    let refused = String::from_utf8_lossy(&answer.stderr).trim().to_string();
    Err(if refused.is_empty() { said } else { refused })
}

/// Pull the value out of `hyprctl -j getoption`'s answer.
///
/// The shape depends on the option's type: `int`, `float` and `str` for the
/// scalars, `bool` for the switches, and `css` for the box-ish ones - `gaps_in`
/// answers `"5 5 5 5"`, because Hyprland keeps it as four numbers. So the value
/// is whichever of those fields is there, and `set` travels with it.
fn parse_option(answer: &Value) -> Option<Reading> {
    let set = answer.get("set").and_then(Value::as_bool).unwrap_or(false);
    for key in ["int", "float", "bool", "str", "css"] {
        let Some(field) = answer.get(key) else {
            continue;
        };
        if field.is_null() {
            continue;
        }
        // A string option that is empty answers with a placeholder rather than
        // with nothing, and that placeholder is not a value.
        let value = match field {
            Value::String(text) => text.trim_matches('"').to_string(),
            Value::Bool(flag) => flag.to_string(),
            Value::Number(number) => number.to_string(),
            _ => continue,
        };
        if value.is_empty() || value == "[[EMPTY]]" {
            return Some(Reading {
                value: String::new(),
                set,
            });
        }
        return Some(Reading { value, set });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn a_value_is_read_out_of_whichever_field_it_arrived_in() {
        // The three scalar shapes, exactly as Hyprland spells them...
        let int = parse_option(&answer(
            r#"{"option":"general:border_size","int":2,"set":true}"#,
        ));
        assert_eq!(
            int,
            Some(Reading {
                value: "2".into(),
                set: true
            })
        );

        let float = parse_option(&answer(
            r#"{"option":"input:sensitivity","float":0.5,"set":false}"#,
        ));
        assert_eq!(
            float,
            Some(Reading {
                value: "0.5".into(),
                set: false
            })
        );

        let flag = parse_option(&answer(
            r#"{"option":"decoration:blur:enabled","bool":true,"set":true}"#,
        ));
        assert_eq!(
            flag,
            Some(Reading {
                value: "true".into(),
                set: true
            })
        );

        // ...and the box-ish ones, which answer as four numbers in a string.
        let css = parse_option(&answer(
            r#"{"option":"general:gaps_in","css":"5 5 5 5","set":true}"#,
        ));
        assert_eq!(
            css,
            Some(Reading {
                value: "5 5 5 5".into(),
                set: true
            })
        );
    }

    #[test]
    fn an_option_with_no_value_reads_as_empty() {
        // An empty string option answers with a placeholder; it is still a
        // value - "the config sets this to nothing" - so it counts as set.
        let empty = parse_option(&answer(
            r#"{"option":"input:kb_options","str":"[[EMPTY]]","set":true}"#,
        ));
        assert_eq!(
            empty,
            Some(Reading {
                value: String::new(),
                set: true
            })
        );

        // An option Hyprland does not know has no value field at all.
        assert_eq!(
            parse_option(&answer(r#"{"option":"nonsense","set":false}"#)),
            None
        );
    }

    #[test]
    fn a_workspace_is_spelled_the_way_the_dispatcher_wants_it() {
        // A numbered workspace is a number - a Hyprland configured in Lua reads
        // `"3"` as an addressable name and `3` as an id, and would create a
        // workspace called `3` in the wrong spelling.
        assert_eq!(target("3"), "3");
        assert_eq!(target("12"), "12");
        // Everything else is a quoted string, special workspaces included.
        assert_eq!(target("special:magic"), "\"special:magic\"");
        assert_eq!(target("code"), "\"code\"");
    }
}

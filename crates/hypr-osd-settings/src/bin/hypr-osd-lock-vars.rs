//! `hypr-osd-lock-vars` — rewrite the lock screen's generated variables.
//!
//! hyprlock cannot read CSS, so the palette and the lock screen's knobs are
//! written into `~/.config/hypr/hyprlock-vars.conf`, and the hand-written
//! `~/.config/hypr/hyprlock.conf` sources it (see [`hypr_osd_core::lock`]). This
//! is that write without a window:
//!
//! * the installer runs it right after installing the structure, so a fresh
//!   machine has a lock screen that reads theme.css;
//! * it is worth running by hand after retinting `~/.config/hypr-osd/theme.css`,
//!   since the generated file is cut from the palette - the settings window also
//!   writes it when it opens, and on every change on its Lock screen page.
//!
//! ```text
//! hypr-osd-lock-vars            # write it
//! hypr-osd-lock-vars --print    # print it instead
//! ```
//!
//! Exit status is 1 when the file could not be written, with the reason on
//! stderr: the installer is the caller that cares about that.

use hypr_osd_core::lock;

fn main() -> std::process::ExitCode {
    if std::env::args().skip(1).any(|arg| arg == "--print") {
        print!("{}", lock::document());
        return std::process::ExitCode::SUCCESS;
    }

    match lock::write_vars() {
        Ok(path) => {
            println!("wrote {}", path.display());
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("hypr-osd-lock-vars: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

# Copilot Instructions — hypr-osd (hyprland_lvmgui)

Workspace-specific guidance for Copilot working in this repository.

## Project overview

**hypr-osd** is a collection of small Hyprland UI elements: cards that appear on
screen for a moment (volume, media, session), styled as bar popups. **One binary
per element**, sharing a small core crate. See the README for the full tour.

Elements are *not* windows: each one owns a borderless layer-shell surface
(`gtk4-layer-shell`) with one card in it. They never take keyboard focus, with two
deliberate exceptions (the switcher and the overview, which are driven by keys).
Hyprland drives them through `scripts/hyprland/osd.lua`.

The **bar** is the exception to "one surface": it belongs on every screen, so it
owns one surface per output, reconciled against GDK's monitor list. The element
hands the shell a `Content::PerOutput` factory and the shell calls it once per
output (`Osd::sync_outputs`); the bar keys its views by connector and paints them
all from one set of readings. One of them carries the session's tray.

See also `crates/hypr-osd-island` (the panel under the clock), `-stats` (the
system popup), `-switcher` and `-overview` (the two keyboard-driven cards).

## Tech stack

- **Frontend/UI:** GTK4 via `gtk4-rs` 0.10 (`gtk4-layer-shell` 0.6 pins that
  version — do not bump one alone), plain GTK CSS (no `var()`, no `transform`,
  no CSS animations).
- **Backend:** Rust, `std::process::Command` for CLI tools (`wpctl`, `hyprctl`)
  rather than native bindings.
- **Single instance:** `GtkApplication` + `GApplicationFlags::HANDLES_COMMAND_LINE`.
  A second invocation of a binary forwards its argv to the running instance over
  D-Bus and returns its exit status/output; widgets are built lazily on the first
  `command-line`/`activate` (GTK does not exist before the app starts).

## Layout

- `crates/hypr-osd-core/` — shared: `css.rs` (the runtime palette from
  `~/.config/hypr-osd/theme.css`, plus the `base.css` recipes), `osd.rs` (the
  layer surfaces, cards, auto-hide, `on_shutdown`, `run()` — and the bar's one
  surface per output), `hardware.rs` (the volume/battery/network reads, `run` for
  a command that answers once, `launch` for one that must not be waited for),
  `follow.rs` (run a long-lived command and deliver its lines to the main loop),
  `output.rs` (run a command that answers once **with bytes**), `hypripc.rs`
  (Hyprland's request socket and event stream), `mpris.rs`, `system.rs`,
  `hover.rs` (a popup's hot zone and dwell), `monitors.rs` (the focused output,
  and every output), `config.rs` (`~/.config/hypr-osd/<element>.conf`).
- `crates/hypr-osd-bar/` — the bar: `main.rs` (verbs, settings, and one
  `BarView` per monitor painted from one set of readings), `view.rs` (the pills),
  `hypr.rs` (the workspace row and title, from the event socket), `tray.rs` (the
  StatusNotifier host), `bar.css`.
- `crates/hypr-osd-volume/` — element #1: `main.rs` (verbs, settings),
  `sink.rs` (wpctl), `view.rs` (widgets), `volume.css`. Owns the volume step.
- `crates/hypr-osd-media/` — element #2: `main.rs` (verbs, settings, trigger
  rules), `player.rs` (playerctl follow + parsing), `art.rs` (`mpris:artUrl` →
  texture), `view.rs` (widgets), `media.css`. Watch-only: no keybinding.
- `crates/hypr-osd-session/` — element #3: `main.rs` (verbs, settings),
  `actions.rs` (the five actions, their commands and availability),
  `view.rs` (rows + the click-again rule), `session.css`. Keyed by
  `SUPER + SHIFT + L` and `XF86PowerOff`.
- `scripts/install.sh` + `scripts/hyprland/osd.lua` — install and the compositor
  side (keys, autostart, layer rule; the installer also reports the logind
  `HandlePowerKey` setting, which gates the hardware power key).

## Conventions

- **Styling rule:** the palette lives in this repo as `configs/theme.css`,
  installed to `~/.config/hypr-osd/theme.css` and read at runtime by `core::css` —
  the one source of truth for every element (a waybar `style.css` used to play
  that role). `configs/hyprlock.conf` mirrors it by hand, because hyprlock cannot
  read CSS. Never hard-code a colour in an element's CSS — use the `@tokens`
  (`@accent`, `@card_top`, `@fg_dim`, …).
- **Namespace:** every element uses the `hypr-osd` layer-shell namespace prefix,
  so the single layer rule (`ignore_alpha`, which makes the transparent frame
  click-through) covers all of them.
- **App ids:** `com.schells2.osd.<element>` (the repo owner's convention, cf.
  `com.schells2.launchpad`, `com.schells2.hyprland-settings`).
- **One source of truth for state:** an element that owns a step (volume) reads
  the value back after writing it, so the card can never show a stale number.
- **Never wait for a command that might outlive the click.** Use
  `hypr_osd_core::hardware::launch` (spawn, hand the exit to GLib) rather than
  `run` when the command is another *element* or a terminal: the first invocation
  of an element nobody has started yet becomes that element's daemon and stays up
  as long as its card does, so a `.status()` wait parks the caller's main loop for
  good (the bar's status pill froze it that way). `run` is only for tools that
  answer once and leave — `wpctl`, `playerctl`, `hyprctl`.
- **Comments explain *why*, at the place it matters** — this repo is documentation
  as much as code; keep that density when editing.
- Run before considering a change complete: `cargo fmt`, `cargo clippy
  --all-targets`, `cargo test`.

## Build & run

```sh
cargo build --release          # or cargo build
cargo test                     # parsing unit tests
cargo clippy --all-targets
./target/release/hypr-osd-bar   # the bar, on every monitor
./target/release/hypr-osd-bar status    # what every pill reads, and the screens
./target/release/hypr-osd-volume up     # first invocation becomes the daemon
./target/release/hypr-osd-media         # watch for tracks (needs an MPRIS player)
./target/release/hypr-osd-session toggle  # the session card
pkill -x hypr-osd-volume                # stop a daemon (also stops its followers)
pkill -f hypr-osd-session               # '-x' only matches names up to 15 chars
pkill -f 'hypr-osd-ba[r]'               # the bar ('-f'; the brackets keep pkill
                                        #   from matching the shell running it)
./scripts/install.sh --no-hyprland      # install binaries + config only
./scripts/install.sh                    # also write/refresh ~/.config/hypr/osd.lua
```

## Verification

- CSS errors are printed to stderr with line numbers (do not swallow them).
- `hyprctl -j layers` shows the surface, its namespace and its geometry — the
  fastest way to distinguish "not mapped" from "mapped off-screen". A bar should
  appear as one `hypr-osd` surface on the *top* layer per monitor.
- `hypr-osd-bar status` prints what every pill reads **and** which screens the bars
  are on (and which one carries the tray) — the first thing to run when the bar
  looks wrong or is missing from a monitor.
- A bar that has stopped answering the D-Bus verbs is a main loop parked in a
  blocking wait: `ps -L -p <pid> -o tid,stat,wchan,comm` (a `do_wait` on the first
  thread is `Command::status`/`.wait()`). Use `hardware::launch` instead.
- `hypr-osd-volume status` prints the sink state without a card;
  `hypr-osd-media status` prints the current track without a card;
  `hypr-osd-session status` prints what each session action would run
  (and, on this machine, that Lock is unavailable: no hyprlock/swaylock/gtklock).
- Never test `reboot`/`poweroff`/`suspend`/`logout`/`lock` for real - they are
  destructive or lock the user out. `actions.rs`' unit tests cover the policy
  (which actions ask twice, suspend locking first) instead.
- A live MPRIS player is needed to test the media card. `vlc` (with
  `--extraintf dbus`, or `cvlc -I dummy`) exposes MPRIS and extracts cover art to
  `~/.cache/vlc/art`; `playerctl -l` lists what is available. Paused players do
  not trigger the card (`only_when_playing`).
- A live check needs the compositor: run the element, then screenshot with
  `grim`. Remember `grim -g` takes **global layout** coordinates, and the monitor
  here sits at an offset (check `hyprctl monitors`).

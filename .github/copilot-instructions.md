# Copilot Instructions — hypr-osd (hyprland_lvmgui)

Workspace-specific guidance for Copilot working in this repository.

## Project overview

**hypr-osd** is a collection of small Hyprland UI elements: cards that appear on
screen for a moment (volume, media, session), styled as bar popups. **One binary
per element**, sharing a small core crate. See the README for the full tour.

Elements are *not* windows: each one owns a single borderless layer-shell
surface (`gtk4-layer-shell`, overlay layer) with one card in it. They never take
keyboard focus. Hyprland drives them through `scripts/hyprland/osd.lua`.

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

- `crates/hypr-osd-core/` — shared: `css.rs` (runtime waybar palette + `base.css`
  recipes), `osd.rs` (layer surface, card, auto-hide, `on_shutdown`, `run()`),
  `follow.rs` (run a long-lived command and deliver its lines to the main loop,
  for elements that watch instead of waiting for a key press), `monitors.rs`
  (focused output via `hyprctl -j activeworkspace`), `config.rs`
  (`~/.config/hypr-osd/<element>.conf`).
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

- **Styling rule:** the palette is *not* in this repo. It is read at runtime from
  the `@define-color` block of `~/.config/waybar/style.css`, the same way
  `~/.config/waybar/scripts/theme.py` does it for the bar's popups; the recipes
  in `base.css` mirror that module's `base_css()`. Never hard-code a colour in an
  element's CSS — use the `@tokens` (`@accent`, `@card_top`, `@fg_dim`, …).
- **Namespace:** every element uses the `hypr-osd` layer-shell namespace prefix,
  so the single layer rule (`ignore_alpha`, which makes the transparent frame
  click-through) covers all of them.
- **App ids:** `com.schells2.osd.<element>` (the repo owner's convention, cf.
  `com.schells2.launchpad`, `com.schells2.hyprland-settings`).
- **One source of truth for state:** an element that owns a step (volume) reads
  the value back after writing it, so the card can never show a stale number.
- **Comments explain *why*, at the place it matters** — this repo is documentation
  as much as code; keep that density when editing.
- Run before considering a change complete: `cargo fmt`, `cargo clippy
  --all-targets`, `cargo test`.

## Build & run

```sh
cargo build --release          # or cargo build
cargo test                     # parsing unit tests
cargo clippy --all-targets
./target/release/hypr-osd-volume up     # first invocation becomes the daemon
./target/release/hypr-osd-media         # watch for tracks (needs an MPRIS player)
./target/release/hypr-osd-session toggle  # the session card
pkill -x hypr-osd-volume                # stop a daemon (also stops its followers)
pkill -f hypr-osd-session               # '-x' only matches names up to 15 chars
./scripts/install.sh --no-hyprland      # install binaries + config only
./scripts/install.sh                    # also write/refresh ~/.config/hypr/osd.lua
```

## Verification

- CSS errors are printed to stderr with line numbers (do not swallow them).
- `hyprctl -j layers` shows the surface, its namespace and its geometry — the
  fastest way to distinguish "not mapped" from "mapped off-screen".
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

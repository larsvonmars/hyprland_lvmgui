# Copilot Instructions — hypr-osd (hyprland_lvmgui)

Workspace-specific guidance for Copilot working in this repository.

## Project overview

**hypr-osd** is a collection of small Hyprland UI elements: cards that appear on
screen for a moment (volume, media, session), styled as bar popups. **One binary
per element**, sharing a small core crate. See the README for the full tour.

Elements are *not* windows: each one owns a borderless layer-shell surface
(`gtk4-layer-shell`) with one card in it. They never take keyboard focus, with
four deliberate exceptions (the switcher, the overview, the launcher and the
applications panel, which are driven by keys - or, for the last two, typed into).
Hyprland drives them through `scripts/hyprland/osd.lua`.

The **bar** is the exception to "one surface": it belongs on every screen, so it
owns one surface per output, reconciled against GDK's monitor list. The element
hands the shell a `Content::PerOutput` factory and the shell calls it once per
output (`Osd::sync_outputs`); the bar keys its views by connector and paints them
all from one set of readings. One of them carries the session's tray.

See also `crates/hypr-osd-island` (the panel under the clock, whose notification
hub is read in two halves: the *state* from `swaync-client -swb`, and the
notifications themselves - application, summary, icon - off the bus with
`busctl --user --json=short monitor org.freedesktop.Notifications`, because
swaync publishes no list of them), `-stats` (the system popup), `-switcher` and
`-overview` (the two window cards), `-launcher` (the search card on SUPER + SPACE,
which took over from the separate `linux-launchpad` Tauri app) and `-apps` (that
app's *main window* - the grid of everything installed - rebuilt as a card the
bar's left-end button opens).

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
  and every output), `apps.rs` (installed applications: the `.desktop` parser,
  the category map, the scoring ladder and `gio launch` - shared by the launcher
  and the panel), `timer.rs` (one-shot timers that cannot fire after a cancel),
  `config.rs` (`~/.config/hypr-osd/<element>.conf`).
- `crates/hypr-osd-bar/` — the bar: `main.rs` (verbs, settings, and one
  `BarView` per monitor painted from one set of readings), `view.rs` (the pills),
  `hypr.rs` (the workspace row and title, from the event socket), `tray.rs` (the
  StatusNotifier host), `bar.css`. Its far-left pill is the applications button:
  a click runs `apps_command` with `toggle` (`hardware::launch`, never a wait),
  and the pill wears `panel-open` while the flag in `state::apps_panel` says the
  panel is up.
- `crates/hypr-osd-volume/` — element #1: `main.rs` (verbs, settings),
  `sink.rs` (wpctl), `view.rs` (widgets), `volume.css`. Owns the volume step.
- `crates/hypr-osd-media/` — element #2: `main.rs` (verbs, settings, trigger
  rules), `player.rs` (playerctl follow + parsing), `art.rs` (`mpris:artUrl` →
  texture), `view.rs` (widgets), `media.css`. Watch-only: no keybinding.
- `crates/hypr-osd-session/` — element #3: `main.rs` (verbs, settings),
  `actions.rs` (the five actions, their commands and availability),
  `view.rs` (rows + the click-again rule), `session.css`. Keyed by
  `SUPER + SHIFT + L` and `XF86PowerOff`.
- `crates/hypr-osd-launcher/` — element #6, the search card on `SUPER + SPACE`
  (it replaced the separate Tauri/Next.js `linux-launchpad`): `main.rs` (verbs,
  the ranked result list, the file-search worker and its channel),
  `files.rs` (the bounded walk of `~/Documents`…, *no* SQLite index and no
  watcher), `view.rs` (the padded, fixed-height list), `launcher.css`. One of the
  four keyboard cards; the `.desktop` parsing, the category map and the scoring
  ladder it shares with the panel live in `core::apps`.
- `crates/hypr-osd-apps/` — element #7, the applications panel: `main.rs` (verbs,
  the drawer set, the pinned list, the keys, and the geometry it works out from
  the bar's margins), `view.rs` (the drawer sidebar, the tile grid, the widget
  cache keyed by application index), `pins.rs`
  (`~/.config/hypr-osd/apps-pinned`), `apps.css`. The fourth keyboard card, and
  the only element with no keybinding at all: the bar's left-end button is its
  trigger.
- `scripts/install.sh` + `scripts/hyprland/osd.lua` — install and the compositor
  side (keys, autostart, layer rule; the installer also reports the logind
  `HandlePowerKey` setting, which gates the hardware power key, and comments out
  waybar's and launchpad's own wiring).

## Conventions

- **Styling rule:** the palette lives in this repo as `configs/theme.css`,
  installed to `~/.config/hypr-osd/theme.css` and read at runtime by `core::css` —
  the one source of truth for every element (a waybar `style.css` used to play
  that role). `configs/hyprlock.conf` mirrors it by hand, because hyprlock cannot
  read CSS. Never hard-code a colour in an element's CSS — use the `@tokens`
  (`@accent`, `@card_top`, `@fg_dim`, …).
- **Icons are Lucide SVGs, bundled — never a font glyph.** `core::icons::lucide`
  draws one from `crates/hypr-osd-core/icons/`, which `core/build.rs` bakes into
  the binary as a GResource; `core::icons::names` is the vocabulary every element
  draws from. An icon's size is the `size` argument (there is no font-size for a
  drawing), and its colour is the CSS `color` it inherits. The files in
  `icons/lucide/` are Lucide's drawings *outlined* rather than stroked, because
  GTK's own SVG engine fills paths and does not stroke them — see
  `icons/README.md` before touching them. `cargo run -p hypr-osd-core --example
  icon-sheet` is the contact sheet that shows whether a drawing came out right.
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
./target/release/hypr-osd-apps status   # applications, drawers, pins, size, open?
./target/release/hypr-osd-apps show     # the applications panel, without the bar
pkill -x hypr-osd-volume                # stop a daemon (also stops its followers)
pkill -f hypr-osd-session               # '-x' only matches names up to 15 chars
pkill -f 'hypr-osd-ba[r]'               # the bar ('-f'; the brackets keep pkill
                                        #   from matching the shell running it)
pkill -f 'hypr-osd-app[s]'              # a bracket: the pattern matches the
                                        #   daemon, not this command
./scripts/install.sh --no-hyprland      # install binaries + config only
./scripts/install.sh                    # also write/refresh ~/.config/hypr/osd.lua
```

## Verification

- CSS errors are printed to stderr with line numbers (do not swallow them).
- `cargo test` holds the icon vocabulary against the bundle, so a name with no
  drawing behind it is a test failure instead of a card that draws GTK's "image
  missing" placeholder. `cargo run -p hypr-osd-core --example icon-sheet` shows
  every icon at the sizes and colours the elements use — the fastest way to see
  that a drawing arrived as a silhouette rather than an outline.
- `hyprctl -j layers` shows the surface, its namespace and its geometry — the
  fastest way to distinguish "not mapped" from "mapped off-screen". A bar should
  appear as one `hypr-osd` surface on the *top* layer per monitor.
- `hypr-osd-bar status` prints what every pill reads **and** which screens the bars
  are on (and which one carries the tray) — the first thing to run when the bar
  looks wrong or is missing from a monitor.
- `hypr-osd-island status` prints the panel's geometry, whether it is up, and the
  notification rows it would draw (application, summary, age) - the quickest way
  to tell "the bus feed is empty" from "the tile is misdrawn". The rows come from
  `busctl --user --json=short monitor org.freedesktop.Notifications`; the same
  command by hand is how a parsing question is answered, and `notify-send -a App
  -i icon "summary" "body"` is how a row is produced on demand.
- A bar that has stopped answering the D-Bus verbs is a main loop parked in a
  blocking wait: `ps -L -p <pid> -o tid,stat,wchan,comm` (a `do_wait` on the first
  thread is `Command::status`/`.wait()`). Use `hardware::launch` instead.
- `hypr-osd-volume status` prints the sink state without a card;
  `hypr-osd-media status` prints the current track without a card;
  `hypr-osd-session status` prints what each session action would run
  (and, on this machine, that Lock is unavailable: no hyprlock/swaylock/gtklock).
- `hypr-osd-launcher status` prints the application count, the file roots it
  found and whether the card is open; `hypr-osd-launcher search <query>` prints
  the ranking *without* a card, which is how "why is that row first" is answered.
  `show <query>` pre-fills the field, and the whole keyboard path can be driven
  from a script: `hyprctl dispatch 'hl.dsp.send_shortcut({ mods = "", key = "v" })'`
  sends a key to the keyboard-exclusive surface (no `window` needed), so typing,
  Enter and Escape are all testable without a pointer.
- `hypr-osd-apps status` prints the application count, the drawers, the pins, the
  size the card came out at and the **measured** content/sidebar/pane widths -
  the numbers that catch a widget widening the card. `apps` prints one line per
  application with the drawer it landed in (the answer to "why is that one under
  Utilities?"), `drawer <name>` opens a drawer without clicking, and `pin`/`unpin`
  drive the pins file. A throwaway `.desktop` in `~/.local/share/applications` plus
  `refresh` is how the launch path is tested: type to it with `send_shortcut`, press
  `Return`, and check the marker file its `Exec=` touches.
- **A click cannot be scripted.** This Hyprland's Lua API moves the pointer
  (`hyprctl dispatch 'hl.dsp.cursor.move({ x = 100, y = 20 })'`) but has no
  press/click dispatcher, and `send_shortcut` with a mouse key (`mouse:272`) goes
  to the *focused window*, which a layer surface never is. So a pill's `on_click`
  can only be checked by hand; verify the rest of the path instead (the command
  it runs - `apps_command` in `bar.conf` - and the state it writes, e.g.
  `hypr-osd-apps show` lighting the bar's button while the panel is up).
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

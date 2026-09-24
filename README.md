# hyprland_lvmgui

A collection of small Hyprland UI elements — one binary per element.

Each element is a GTK4 app that owns exactly one **card**: a borderless
layer-shell surface that appears when something happens (a volume key, later a
brightness key, a media key), shows the state, and gets out of the way again. It
never takes keyboard focus, it draws above fullscreen windows, and it is styled
from the same palette as the rest of the desktop — see
[Look and feel](#look-and-feel).

| Element | Binary | What it does |
| --- | --- | --- |
| Volume card | `hypr-osd-volume` | Speaker glyph, draggable slider and percentage for the default sink. Owns the volume step for `XF86Audio{RaiseVolume,LowerVolume,Mute}`. |
| Media card | `hypr-osd-media` | Cover art, title and artist, with previous/next buttons. Appears when a new medium starts playing — no keybinding involved. |
| Session card | `hypr-osd-session` | Lock, suspend, log out, reboot and shut down — the same menu the bar's popup shows. `SUPER + SHIFT + L`, or the hardware power key. |
| Window switcher | `hypr-osd-switcher` | Every window you have open, most recently used first, with its application icon and title. `ALT + TAB`, and `ALT + SHIFT + TAB` to walk backwards — the one element that takes the keyboard. |

Siblings of this repo, which share the design language:
[`linux-launchpad`](../linux-launchpad) and
[`hyprland-settings-gui`](../hyprland-settings-gui).

```
crates/
  hypr-osd-core/       the shared half: palette, card recipe, layer surface,
                       single-instance command line, config reader, and the
                       line-follower for elements that watch instead of
                       waiting for a key press
  hypr-osd-volume/     element #1: the volume card
  hypr-osd-media/      element #2: the media card
  hypr-osd-session/    element #3: the session card
  hypr-osd-switcher/   element #4: the Alt-Tab window switcher
scripts/
  install.sh           build + install + wire up Hyprland
  hyprland/osd.lua     the Hyprland side (keys, autostart, layer rule)
configs/
  hyprpaper.conf       the desktop wallpaper (the lock screen's, too)
  hyprlock.conf        the lock screen, in the bar's palette
  hypridle.conf        idle timers: lock, blank the panel, sleep
```

## Look and feel

The rule is the same one the bar and the two apps already follow, so there is
still exactly one place to change a colour:

1. **The palette is not in this repo.** At start-up an element reads the
   `@define-color` block out of `~/.config/waybar/style.css` and pastes it into
   its own stylesheet — exactly what `~/.config/waybar/scripts/theme.py` does for
   the bar's popups. GTK stylesheets are per process, so a copy is the only way;
   reading it at runtime is what keeps the copy honest. A mirror of that block
   (`crates/hypr-osd-core/src/css.rs`) is used only when waybar's file is
   missing, so a token can never come out undefined.
2. **The recipes are theme.py's.** `crates/hypr-osd-core/src/base.css` carries
   the typography (`MesloLGS Nerd Font Mono` 13px/500), the transparent surface
   and the card (theme.py's `.card`: the solid pill fill, `@border`, the 16px
   card radius, the bar's shadow), plus a couple of pieces elements share (tiles,
   chips, the progress recipe).
3. **Element-specific rules live with the element**, in
   `crates/hypr-osd-volume/src/volume.css` — and they borrow from the two apps:
   the slider is the bar's progress recipe (`@surface_h` trough, `@accent` →
   `@accent2` fill, 999px) wearing the settings app's 14px round accent thumb,
   the glyphs are the same nerd-font symbols waybar's `#wireplumber` module
   shows, and a muted sink is painted in the same `@crit` red the bar uses.

GTK CSS is a subset: no `var()` (colour tokens are `@define-color`/`@name`, which
is what waybar's stylesheet uses anyway), no `transform`, and no CSS animations —
the card's entrance is Hyprland's own layer animation (`layersIn`/`layersOut`
in your config), which also keeps it consistent with every other surface on the
desktop.

## Install

```sh
./scripts/install.sh                  # build, install, wire up Hyprland
./scripts/install.sh --no-hyprland    # binary + config only, no config edits
./scripts/install.sh --take-over-keys # …and comment out the old wpctl volume binds
```

What it does:

| | |
| --- | --- |
| `~/.local/bin/hypr-osd-volume` | the binary |
| `~/.config/hypr-osd/volume.conf` | a commented config template (only created if missing) |
| `~/.config/hypr/osd.lua` | volume keys, autostart and the layer rule, generated from `scripts/hyprland/osd.lua` |
| `~/.config/hypr/hyprland.lua` | one appended line: `require("osd")` (backup: `hyprland.lua.bak`), then `hyprctl reload` |
| `~/.config/hypr/hyprpaper.conf` | the desktop wallpaper — only when hyprpaper is installed, and only if the file is missing |
| `~/.config/hypr/hyprlock.conf` | the lock screen — only when `hyprlock` is installed, and only if the file is missing |
| `~/.config/hypr/hypridle.conf` | idle timers — only when `hypridle` is installed, and only if the file is missing |

One thing to know: the volume element owns the volume **step**, so the three
`XF86Audio*` binds in `osd.lua` have to be the only ones on those keys. If
`hyprland.lua` still runs `wpctl set-volume …` on them, every press moves the
volume twice — the installer reports those lines, and `--take-over-keys`
comments them out for you (marking them, with a backup). The microphone key is
left alone.

The session card also binds `SUPER + SHIFT + L` and `XF86PowerOff`; if you
already use either, the installer says so rather than quietly replacing it. The
power key has a second gate in front of it — systemd-logind — which the
installer also reports; see [How it works](#how-it-works).

Requirements: Hyprland ≥ 0.55 (Lua config), GTK4, `gtk4-layer-shell`, a Rust
toolchain; at runtime `wpctl` (PipeWire/WirePlumber) and `hyprctl`.

```sh
sudo pacman -S --needed base-devel pkgconf gtk4 gtk4-layer-shell rust
```

## Use

Every element answers to verbs on its own binary, and they work the same whether
its daemon is already running or not: the binary is single-instance, so a second
invocation forwards its arguments to the running instance over D-Bus (and gets
the exit status and any output back). That is also why a keybinding can just be
`exec, hypr-osd-volume up`.

**`hypr-osd-volume` — the volume card**

```sh
hypr-osd-volume up        # +5 %, unmuting on the way up
hypr-osd-volume down      # −5 %
hypr-osd-volume toggle    # mute/unmute, keeping the level
hypr-osd-volume set 40    # an absolute level
hypr-osd-volume show      # reveal the card without changing anything
hypr-osd-volume status    # print the level, no card (for scripts/waybar)
hypr-osd-volume           # start the daemon and wait for the first key press
```

Dragging the card's slider sets the volume through the same path, and the card
stays up for as long as you hold it.

**`hypr-osd-media` — the media card**

```sh
hypr-osd-media next       # skip forward, then show what is playing
hypr-osd-media previous   # skip back
hypr-osd-media show       # reveal the card for the current track
hypr-osd-media status     # print "Title — Artist", no card
hypr-osd-media            # start the daemon and wait for a track to start
```

It needs no keybinding: the daemon watches the player and shows the card by
itself (see [How it works](#how-it-works)). The buttons on the card run exactly
the `next`/`previous` verbs, so a click and a command take the same path.

**`hypr-osd-session` — the session card**

```sh
hypr-osd-session toggle   # show it, or take it away again (what the key does)
hypr-osd-session show     # reveal it without toggling
hypr-osd-session status   # what each action would run, and what is unavailable
hypr-osd-session reboot   # run one action now: lock | suspend | logout | reboot | poweroff
```

The card's rows are the same five actions the bar's own session menu offers
(`~/.config/waybar/scripts/popup.py`), with the same glyphs and the same
"destructive actions ask twice" rule: `Reboot` and `Shut down` arm on the first
click and fire on the second, so a card that appeared under the pointer cannot
end your session by accident. A verb is never confirmed — typing or binding it is
already deliberate. **Lock** uses whatever screen locker is installed (and
**Suspend** locks first, then sleeps) — see [Lock screen](#lock-screen).

**`hypr-osd-switcher` — the window switcher**

```sh
hypr-osd-switcher next     # open the card and step forward (what Alt+Tab runs)
hypr-osd-switcher prev     # the same, backwards (Alt+Shift+Tab)
hypr-osd-switcher show     # open it without moving the selection
hypr-osd-switcher commit   # switch to the selected window, and close
hypr-osd-switcher cancel   # close without switching
hypr-osd-switcher status   # the list, in switcher order, and no card
```

It is the one element that takes the keyboard, and it has to be: a switch ends
when you let go of Alt, and the only way to see that release is to own the
keyboard for as long as the card is up. So `ALT + TAB` is a *held* gesture. A
tap and release swaps to the window you were in before this one; keep tapping Tab
(or the arrow keys, which walk the grid properly) to move further; Enter settles
on the tile you are on, a click on a tile does the same, `1`–`9` jump straight to
one, and Escape walks away without switching. Holding Shift goes backwards.

The list is `hyprctl clients` in `focusHistoryID` order — the order you last used
the windows in — and it includes windows on other workspaces, because switching
to one should bring its workspace along. If no key arrives for five seconds the
card commits on its own (see `idle_commit_ms`), which is the safety net for the
one case the keyboard cannot cover: an Alt release that happened before the card
was listening.

## Configure

One file per element under `~/.config/hypr-osd/`, every key optional (the
defaults are in the comments of the installed templates).

**`volume.conf`**

| Key | Default | |
| --- | --- | --- |
| `step_percent` | `5` | volume step per key press |
| `max_percent` | `100` | the slider's ceiling; raise it to allow over-amplification |
| `duration_ms` | `1400` | how long the card stays after the last change |
| `unmute_on_raise` | `true` | `up` (and dragging) also unmutes a muted sink |
| `width` | `340` | card width in pixels |
| `bottom_margin` | `0.12` | distance from the bottom edge, as a fraction of the monitor height |

**`media.conf`**

| Key | Default | |
| --- | --- | --- |
| `duration_ms` | `4000` | how long the card stays after the last change |
| `only_when_playing` | `true` | only show the card for something that is actually playing |
| `width` | `400` | card width in pixels (cover, text, buttons) |
| `bottom_margin` | `0.12` | distance from the bottom edge, as a fraction of the monitor height |

**`session.conf`**

| Key | Default | |
| --- | --- | --- |
| `duration_ms` | `8000` | how long the card stays; `0` keeps it up until dismissed |
| `width` | `0` | minimum card width; `0` lets the rows decide |
| `bottom_margin` | `0.12` | distance from the bottom edge, as a fraction of the monitor height |

A daemon reads its file once, at start-up:

```sh
pkill -x hypr-osd-media && hypr-osd-media &
pkill -f hypr-osd-session && hypr-osd-session &   # '-f': see note below
```

`pkill -x` matches process names of up to 15 characters, and `hypr-osd-session`
is one longer — use `pkill -f hypr-osd-session` for that element.

**`switcher.conf`**

| Key | Default | |
| --- | --- | --- |
| `columns` | `5` | tiles per row; the card's width follows from this and `tile_width` |
| `tile_width` | `132` | one tile's width in pixels, and how much of a title fits in it |
| `icon_size` | `72` | application icon size in pixels |
| `idle_commit_ms` | `5000` | commit if no key arrives for this long; `0` turns the safety net off |

Application icons come from the window's class: the freedesktop entry that claims
it (by file name, or by `StartupWMClass` for the Electron apps that need it),
then GTK's own icon theme. An application nothing can be found for gets its first
letter in a tile rather than a generic glyph, so the grid still says which window
is which.

## Lock screen

**hyprlock** draws it (`sudo pacman -S hyprlock`), from
`~/.config/hypr/hyprlock.conf`, which this repo carries as
[`configs/hyprlock.conf`](configs/hyprlock.conf) and the installer writes when
the file is missing — it is meant to be edited, so re-running `install.sh` leaves
your version alone.

hyprlock cannot read the bar's stylesheet, so the palette is mirrored at the top
of that file as `$variables` — the same `@define-color` values theme.py reads for
the bar's popups and the OSDs' `base.css` mirrors, written as `#RRGGBBAA`. Same
deal as everywhere else in this theme: change a colour in
`~/.config/waybar/style.css`, change it here.

What it draws, over **your desktop wallpaper** — the lock screen points straight at
the image the desktop uses, rather than at a screenshot of the desktop, so it
shows the background you chose, never the shape of what you had open, and the
backdrop holds still while everything else sleeps:

- a sheet of dark glass — one `shape`, 800×670, hairline accent border, deep
  shadow — so the type sits on a known contrast instead of on the wallpaper;
- the **clock at 170px**, twice the bar's own type, with an accent colon and a
  soft shadow; the date in caps underneath, then a short accent rule;
- the password field **only while there is something to type into**: a tall
  440×78 box, glass tinted towards the accent, a gradient border that carries
  the state (accent while PAM checks, red when the password is wrong, amber while
  capslock is on).

The `path` in that block (`~/Bilder/Wallpapers/1022522.jpg` here) names the same
image [`configs/hyprpaper.conf`](configs/hyprpaper.conf) puts on the desktop —
the wallpaper and the lock screen's backdrop are one picture, and those two files
are where it is decided. hyprlock draws a still image, so a *video* wallpaper
needs a frame of it instead — the comment above the line has the one-liner — and
pointing it back at `screenshot` re-takes the live desktop on every lock instead.
Blur, dim and grain are the four numbers just below it: at `blur_size = 1` the
picture is essentially untouched, and 8–16 is frosted glass.

Switching wallpapers at runtime is one command —
`hyprctl hyprpaper wallpaper ",/path/to/image.jpg"` — but do change the lock
screen's `path` too, or the two will disagree until you do.

The field is what makes the resting screen quiet: it fades out while the buffer
is empty and back in with the first keystroke, so what you walk up to says
"locked" instead of "type here". A padlock glyph and one faint line carry that.
**Enter on its own does not reveal it** — hyprlock follows the buffer, not the
key, and an empty Enter is deliberately a no-op (`ignore_empty_input`, so a stray
Enter cannot spend one of PAM's three attempts). Once there is something to
submit, Enter submits it, as always.

One thing to know before moving anything: `position` is a `layoutxy` offset from
the alignment anchor, and **y points up** — `position = 0, 259` is 259px *above*
the middle. hyprlock's documentation does not say so, and guessing the other way
puts the whole arrangement upside down, which is what this file did until it was
first rendered.

Three things lock the screen, and all three end in the same place:

| | |
| --- | --- |
| `SUPER + L` | `pidof hyprlock >/dev/null \|\| hyprlock` — a second request must not start a second locker, which would refuse the lock and log an error |
| the session card's **Lock** row | the same program; that row is enabled exactly when hyprlock (or swaylock/gtklock) exists |
| **hypridle**, on idle or before sleep | `loginctl lock-session` → hypridle's `lock_cmd` |

**hypridle is a separate program** (`sudo pacman -S hypridle`): hyprlock knows how
to draw a lock screen, hypridle is what decides to show it. Without it, nothing
locks by itself — the key and the card's row still work. Its config is
[`configs/hypridle.conf`](configs/hypridle.conf): lock after 5 minutes, blank the
panel after 10, sleep after 30. `install.sh` installs it when hypridle is
present, and `osd.lua` starts hypridle at login only if it exists — so the fix is
the install line, not a config edit.

Three habits worth having with a lock screen:

- **Check a config edit without locking yourself out.** hyprlock *ignores* bad
  entries instead of refusing to run, so a typo is silent until it matters. Point
  it at a display that does not exist and it reads the config, prints every error
  with a line number, and only then fails to connect:

  ```sh
  WAYLAND_DISPLAY=none hyprlock --config ~/.config/hypr/hyprlock.conf -v
  ```

  `install.sh` runs exactly this after writing the config.
- **Look at a config edit without locking anything either.** A lock screen is the
  one thing you cannot try on yourself, so try it on a throwaway compositor: start
  a nested Hyprland (a window in your session, class `aquamarine`), point hyprlock
  at *that* wayland display, and photograph the window from the outside.

  ```sh
  Hyprland -c /dev/null &                 # nested, on its own display + socket
  hyprlock --display wayland-2 --config /tmp/preview.conf &
  grim -g "<the window's rect>" preview.png
  ```

  Three things to know: `grim` **cannot capture a locked output** (the lock
  protocol forbids it) — the capture has to come from the outer session, so the
  window needs to be visible and focused; `path = screenshot` would photograph the
  nested desktop, so preview a copy of the config that points at an image; and a
  nested output's logical size follows its window, so set the window to your
  monitor's geometry first, otherwise every size in the file is off.
- **If a lock screen ever misbehaves**, you are not stuck: `Ctrl+Alt+F2` to a
  text console, log in, and `pkill hyprlock`. hyprlock runs as you, not as root.

`loginctl lock-session` works from anywhere — a keybinding, a script, a timer —
and is what the pieces above use, so any future locker that listens for the
session lock signal slots in without touching them.

## How it works

- **One process per element**, started at login by `osd.lua`'s `exec-once`
  (nothing is shown), or lazily by the first key press. A card, not a window:
  `gtk4-layer-shell` puts the surface on the *overlay* layer, so it appears above
  fullscreen windows and is not touched by the tiling layout.
- **It never steals focus.** The layer surface is created with
  `KeyboardMode::None`, so the window you were typing in keeps the keyboard while
  the card is up. Pointer input still works — the slider needs it.
- **The transparent frame is click-through.** The surface is the card plus an
  18px ring that lets the card's shadow breathe; `osd.lua`'s `ignore_alpha` layer
  rule makes those transparent pixels pass clicks to whatever is behind them, so
  only the card itself is hit-testable.
- **It follows the focused monitor.** Each command asks `hyprctl -j
  activeworkspace` which output is focused, and the surface is (re)built for that
  monitor, so the card appears where you are looking. One monitor means this is a
  no-op.
- **`wpctl` is the single source of truth.** The volume element reads the sink, applies
  the change and reads it back, so the card can never show a number the sink does
  not have — and it can never disagree with the bar's volume module.
- **The media element watches instead of polling.** One `playerctl metadata
  --follow` runs for the session and prints a line per change; the daemon reads
  it as a single async stream (no timer, no repeated process spawns, nothing
  while the player is idle). `playerctl` waits for a player to appear, and the
  follower restarts it if it ever exits, so playback that starts later is picked
  up too.
- **What counts as “a new medium”.** A followed event shows the card when it is
  *playing*, has something to show, and is not the track already on the card.
  Players announce one track with several events (metadata, then the status
  flipping, sometimes the artwork later); that rule is what tells them apart and
  keeps a pause/resume from re-opening the card.
- **Cover art** is read from `mpris:artUrl`: a `file://` URL is percent-decoded
  and loaded directly (VLC, and most local players, extract covers there), an
  `http(s)://` URL is fetched with `curl` in the background. Anything else — a
  `blob:` URL, a `data:` URI, which some browser players use — has no file behind
  it, so the card keeps the bar's music glyph instead. While a new cover is being
  fetched the glyph stays up: showing the previous track's cover would be a lie.
- **Auto-hide.** `duration_ms` after the last change the card hides — unless the
  volume slider is being dragged, or (for the cards with buttons) the pointer is
  resting on it, so they stay usable.
- **The session card is a menu, not a notification.** A layer surface never gets
  the keyboard, so Escape cannot reach it: the key that opened it toggles it away
  again, and the card says so in its own hint line. Its `duration_ms` is longer
  than the other cards', and `0` keeps it up until dismissed. Arming a
  destructive row gives the card a fresh grace period, and showing the card again
  disarms everything — an armed row that outlived a hide would be a loaded gun
  nobody remembers loading.
- **The power key belongs to systemd-logind by default.** With
  `HandlePowerKey=` set to anything but `ignore` (this machine suspends), logind
  acts on the key itself and may consume the event before Hyprland sees it, so the
  card would never appear. `install.sh` reports the current setting and prints the
  two lines that hand the key over (a `logind.conf.d` drop-in and a logind
  restart); `SUPER + SHIFT + L` works either way.
- **Stopping a daemon stops its helpers.** Follower children are not killed by
  their parent dying, so `run` turns SIGTERM/SIGINT into a clean shutdown and
  `Osd::on_shutdown` gives each element the chance to stop what it started.
  `pkill -x hypr-osd-media` therefore leaves no orphaned `playerctl` behind.

## Adding an element

1. `crates/hypr-osd-<name>/`, a `Cargo.toml` with a `[[bin]]` whose name matches
   the crate, and `src/main.rs` + `src/<name>.css`.
2. Use `hypr_osd_core::run(…)`: give it `Opts` (`app_id` `com.schells2.osd.<name>`,
   namespace `hypr-osd`, width, bottom margin), a `Build` closure that returns the
   card's content widget, and a `Handle` closure that implements the verbs and
   calls `osd.reveal(duration)` when the card should appear.
3. An element that reacts to something happening rather than to a key press starts
   a follower in `Build` (`hypr_osd_core::follow`) and stops it in
   `osd.on_shutdown(…)` — that is the whole of the media card's wiring.
3. Keep the `hypr-osd` namespace prefix: the layer rule in `osd.lua` matches it,
   and that rule is what makes the transparent frame click-through.
4. Style it from the tokens (`@accent`, `@card_top`, …) and the shared classes
   (`box.card`, `box.tile`, `label.chip`, `progressbar`), and add only what is
   special about that element.
5. Add the binary to `ELEMENTS` in `scripts/install.sh` and its keybinding/
   autostart to `scripts/hyprland/osd.lua`.
6. `cargo fmt && cargo clippy --all-targets && cargo test`.

## Development

```sh
cargo build --release            # or: cargo build
cargo test                       # config/css/argv/sink parsing
cargo clippy --all-targets
cargo fmt
```

Running an element straight out of `target/release/` works exactly like the
installed one — the *first* invocation becomes the daemon, so run it in the
background when you want to keep using the shell:

```sh
./target/release/hypr-osd-volume up     # daemon starts, volume steps, card shows
pkill -x hypr-osd-volume                # stop it again
```

Debugging:

- CSS problems are **printed, not swallowed**: run the daemon in the foreground
  (`./target/release/hypr-osd-volume show`) and GTK's parse errors appear on
  stderr with their line numbers.
- `hyprctl -j layers` lists the surface with its namespace and geometry — the
  quickest way to tell "not mapped" from "mapped but off-screen".
- `hypr-osd-volume status` shows what the sink reports, without showing a card.
- `GTK_DEBUG=interactive hypr-osd-volume show` opens the GTK inspector (hover the
  card and press `Ctrl+Shift+D`).

## Troubleshooting

| Symptom | Likely cause |
| --- | --- |
| No card, but the volume changes | The daemon died — check its stderr, and `pgrep -x hypr-osd-volume`. On a session without `gtk4-layer-shell` it cannot create the surface at all. |
| The volume moves two steps per press | `hyprland.lua` still binds the volume keys to `wpctl`; re-run with `--take-over-keys`. |
| `wpctl … failed` on stderr | PipeWire/WirePlumber trouble; `wpctl status` should list a default sink. |
| Card in the wrong place | `bottom_margin` / `width` in the element's config. |
| Card on the wrong monitor | It follows the focused output through `hyprctl`; if `hyprctl` is unavailable it stays on the monitor it was built for. |
| No media card appears | Is the daemon running (`pgrep -x hypr-osd-media`)? Does the player speak MPRIS (`playerctl -l`)? A track that is merely loaded, never played, does not trigger a card unless `only_when_playing = false`. |
| Media card shows no cover | The player reported none, or reported it as a `blob:`/`data:` URL — only `file://` and `http(s)://` artwork can be read. `playerctl metadata | grep -i arturl` shows what it sends. |
| “Lock” is greyed out | No screen locker is installed — `hypr-osd-session status` names the candidates (`hyprlock`, `swaylock`, `gtklock`). The bar's own session menu greys it out the same way. The row re-checks every time the card is shown, so installing one is enough. |
| Nothing locks when you walk away | That is hypridle's job, and it is not installed: `sudo pacman -S hypridle`, then re-run `install.sh`. `pgrep -af hypridle` should show it running. |
| The power button doesn't show the session card | systemd-logind is handling the key (`HandlePowerKey=`); see the note in [How it works](#how-it-works) and re-run `install.sh` to see the current value. |
| The session card won't go away | Press `SUPER + SHIFT + L` again, or set `duration_ms` to something shorter in `session.conf` (`0` means “stay until dismissed”, which is also what a card with no way out looks like). |

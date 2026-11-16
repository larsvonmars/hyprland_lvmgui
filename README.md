# hyprland_lvmgui

The desktop's own UI, in Rust: a top bar, and a collection of small Hyprland
elements — one binary per element.

The **bar** is the piece of furniture: a GTK4 layer-shell surface across the top
of **every** screen — one surface per output, kept in step with the compositor's
monitor list, so plugging a screen in grows a bar onto it and unplugging one
takes that bar away (`output = eDP-1` in `bar.conf` narrows it back down to a
single screen). It holds the workspaces, the focused window's title, what is
playing, the tray, the network, the volume, the battery, the clock and the power
button. It reads the system itself — Hyprland's own sockets, MPRIS, the session
bus, `wpctl`, sysfs — with no helper scripts and no module configuration, which
is the point: the data, the layout and the colours belong to one program, so they
cannot disagree with each other. It replaced waybar on this desktop (see
[Waybar](#waybar)).

Every other element owns exactly one **card**: a borderless layer-shell surface
that appears when something happens (a volume key, a media key, the power key),
shows the state, and gets out of the way again. A card draws above fullscreen
windows and is styled from the same palette as the bar — see
[Look and feel](#look-and-feel). It does not take the keyboard, with a
deliberate few exceptions: cards that are *driven* by keys or typed into have to
own it while they are up (see [How it works](#how-it-works)).

| Element | Binary | What it does |
| --- | --- | --- |
| The bar | `hypr-osd-bar` | Workspaces, window title, media, tray (the application indicators **and** the windows you have put away), the combined status pill (link, sound, battery), clock and the power button, across the top of every screen - in **three isolated pills**: one in the left corner, the player in the middle, one in the right corner. **Every part can be switched off** (the settings window's Bar page, or the `show_*` keys in `bar.conf`), so the bar can be anything from all of it to a clock and a status pill. Hovering the player unfolds the island popup below it, hovering (or clicking) the status pill at the right end unfolds the system popup, and the same on the grid button at the left end unfolds the applications panel. `SUPER + H` puts the focused window away, `SUPER + SHIFT + H` brings the last one back. |
| Island popup | `hypr-osd-island` | What is playing (cover, title, artist, progress with click-to-seek, shuffle/previous/play/next/repeat, the player's own volume and a chip per player), the notification hub, and a calendar — the panel that comes out of the bar's clock. |
| System popup | `hypr-osd-stats` | CPU, memory and temperature as gauges, the pending updates, the bluetooth radios **with the devices they know** (power, visibility, connect, disconnect, pair, forget, scan, battery and signal readouts) and the switches: the wireless link, the sound, the power profile, presentation mode, brightness and the keyboard layout — the panel that comes out of the bar's status pill. Its header's **gear** opens the settings window, and the bluetooth tile's **Manage** chip opens the bluetooth manager. |
| Bluetooth manager | `hypr-osd-bluetooth` | Every device BlueZ knows — paired and merely nearby — with its kind, battery and link quality, the one action that fits its state (connect, disconnect, pair, forget), a filter, a scan and the controller's power and visibility switches. The card the system popup's bluetooth tile opens instead of a terminal full of `bluetoothctl` — see [Bluetooth manager](#bluetooth-manager). |
| Volume card | `hypr-osd-volume` | Speaker glyph, draggable slider and percentage for the default sink. Owns the volume step for `XF86Audio{RaiseVolume,LowerVolume,Mute}`. |
| Media card | `hypr-osd-media` | Cover art, title and artist, with previous/next buttons. Appears when a new medium starts playing — no keybinding involved. |
| Session card | `hypr-osd-session` | Lock, suspend, log out, reboot and shut down. `SUPER + SHIFT + L`, or the hardware power key. |
| Window switcher | `hypr-osd-switcher` | Every window you have open, most recently used first, with its application icon and title. `ALT + TAB`, and `ALT + SHIFT + TAB` to walk backwards. |
| Workspace overview | `hypr-osd-overview` | The whole desktop as a picture of itself: every workspace as a chip, every window as a real thumbnail in its own shape, scaled to fit one screen. `SUPER + SHIFT + TAB`. |
| Launcher | `hypr-osd-launcher` | A search card in the middle of the screen: it matches the applications installed on the machine (names, generic names, keywords, categories) and the files in your own directories, and opens the selected one. `SUPER + SPACE`. Replaces the Tauri/Next.js [`linux-launchpad`](../linux-launchpad) — see [Launcher](#launcher). |
| Applications panel | `hypr-osd-apps` | Every application installed on the machine, as a grid of tiles with a drawer list down the side (and a **Pinned** one), searched through the same field the launcher has. The old launchpad's main window, rebuilt as a card and opened by hovering the **grid button at the bar's left end** (the pointer leaving it closes it again) — see [Applications panel](#applications-panel). |
| Settings | `hypr-osd-settings` | Every setting this desktop has, in one window, under four tabs: **Hypr OSD** (a page per element of this collection, plus the Shortcuts list), **Hyprland** (a page per area of the compositor itself — display, general, decoration, input, animations, misc) and **Appearance** (the wallpaper, the lock screen and the login screen, with file choosers and a one-click "use this picture everywhere") and **System** (the energy policy, with the machine's live power data). A control per key, the `.conf` file rewritten as you change it, and the Hyprland ones handed to the compositor at once. Opened by the **gear at the right end of the system popup's header** — see [Settings](#settings). |
| Energy saving | `hypr-osd-energy` | A headless daemon that drops the compositor's expensive effects — animations, blur, shadows, opacity, dimming, rounded corners — while the machine is in a saving power profile, and puts the machine's own values back when it is not. The policy is the settings window's **System** tab; the machine's live profile, CPU and battery are the data that page shows. |

This desktop's own machine is a Surface Pro 7, so everything here is also built
for a finger: a tap opens the panels a hover opens, a long press is the right
click, the controls grow while the keyboard is folded away, and an on-screen
keyboard appears when something asks to be typed into. That half of the design is
in [Touch](#touch).

Siblings of this repo, which share the design language:
[`linux-launchpad`](../linux-launchpad) — whose spotlight overlay this collection
took over — and [`hyprland-settings-gui`](../hyprland-settings-gui).

```
crates/
  hypr-osd-core/       the shared half: the palette and the card/bar recipes, the
                       layer surface, the single-instance command line, the config
                       reader, Hyprland's two sockets with their event stream,
                       MPRIS, the window and workspace list with its application
                       icons, the machine's own keyboard layouts and variants, and
                       the two ways to run a command without blocking
  hypr-osd-bar/        the bar, and the StatusNotifier tray host it carries
  hypr-osd-island/     the popup that unfolds from the player pill
  hypr-osd-stats/      the system popup under the status pill
  hypr-osd-bluetooth/ element #10: the bluetooth manager (the popup's Manage chip)
  hypr-osd-volume/     element #1: the volume card
  hypr-osd-media/      element #2: the media card
  hypr-osd-session/    element #3: the session card
  hypr-osd-switcher/   element #4: the Alt-Tab window switcher
  hypr-osd-overview/   element #5: the workspace overview
  hypr-osd-launcher/   element #6: the search card on SUPER + SPACE
  hypr-osd-apps/      element #7: the applications panel (the bar's grid button)
  hypr-osd-settings/  element #8: every setting of this collection and of
                      Hyprland, in one window (the gear in the system popup)
  hypr-osd-energy/    element #9: the headless daemon that saves energy while the
                      machine is in a saving power profile
scripts/
  install.sh           build + install + wire up Hyprland (and drop waybar)
  hyprland/osd.lua     the Hyprland side (autostart, keys, layer rule, and the
                       generated desktop settings)
configs/
  theme.css            the palette: every colour of this desktop
  hyprpaper.conf       the wallpaper template (the file itself is generated by
                       the Wallpaper page)
  hyprlock.conf        the lock screen: a reference copy of the *generated*
                       config, with the reasoning behind each number
  hypridle.conf        idle timers: lock, blank the panel, sleep
```

## Look and feel

There is exactly one place to change a colour, and it is not in a program:

1. **The palette is `~/.config/hypr-osd/theme.css`** — installed from
   [`configs/theme.css`](configs/theme.css), and the single source of truth for
   the whole desktop: the bar, every card, the island popup, the lock screen and
   the login screen.
   At start-up a process reads the `@define-color` block out of that file and
   pastes it into its own stylesheet, because GTK stylesheets are per process and
   a copy is the only way to share one; reading it at runtime is what keeps the
   copy honest. A mirror of the same block
   (`crates/hypr-osd-core/src/css.rs`) is used only when the file is missing, so
   a token can never come out undefined — GTK drops a declaration it cannot
   resolve, silently, which from the outside looks like a styling bug.
2. **The recipes are `base.css`.**
   `crates/hypr-osd-core/src/base.css` carries the typography
   (`MesloLGS Nerd Font Mono` 13px/500 — a text face; the icons are not
   characters but Lucide drawings bundled into `hypr-osd-core`, see
   `crates/hypr-osd-core/icons/`), the transparent surface, the card and
   the bar, plus the pieces both are built from (tiles, chips, the progress
   recipe). It lives in the binary rather than in the config directory because it
   is the same everywhere and is not something you retheme.
3. **Element-specific rules live with the element**: the bar's pills in
   `crates/hypr-osd-bar/src/bar.css`, the panel's in
   `crates/hypr-osd-island/src/island.css`, the volume card's in
   `crates/hypr-osd-volume/src/volume.css`. They borrow from each other on
   purpose — the card's slider is the same progress recipe the island's playback
   bar uses, the bar's workspace capsule wears the accent fill the session card
   puts on a row you are about to act on, and a muted sink is the same `@crit`
   red everywhere.

GTK CSS is a subset: no `var()` (colour tokens are `@define-color`/`@name`), no
`transform`, and no CSS animations — the bar and the cards fade in through
Hyprland's own layer animation (`layersIn`/`layersOut`), which also keeps them
consistent with every other surface on the desktop.

```sh
# after editing ~/.config/hypr-osd/theme.css
pkill -f hypr-osd-bar;     hypr-osd-bar &        # the bar (and its tray)
pkill -f hypr-osd-island;  hypr-osd-island &     # the popup under the player pill
```

## Install

```sh
./scripts/install.sh                  # build, install, wire up Hyprland
./scripts/install.sh --no-hyprland    # binary + config only, no config edits
./scripts/install.sh --take-over-keys # …and comment out the old wpctl volume binds
./scripts/install.sh --keep-waybar    # leave the old bar in the autostart
```

What it does:

| | |
| --- | --- |
| `~/.local/bin/hypr-osd-*` | the element binaries (the bar, the island, the cards and the settings window), plus `hypr-osd-lock-vars` — the lock screen's generator, for after retinting the theme by hand |
| `~/.config/hypr-osd/theme.css` | the palette — only if the file is missing, so your edits survive |
| `~/.config/hypr-osd/bar.conf` | the bar's settings (only created if missing) |
| `~/.config/hypr-osd/island.conf` | the island popup's settings (only created if missing) |
| `~/.config/hypr-osd/*.conf` | one config template per card (only created if missing) |
| `~/.config/hypr-osd/settings.conf` | the settings window's own geometry and behaviour — see [Settings](#settings) |
| `~/.config/hypr-osd/desktop.conf` | the Hyprland tab's settings: the compositor's own options, as option paths, plus one `monitor:<output>:<field>` entry per display you have configured and the collection's own `primary` display (only created if missing) |
| `~/.config/hypr/hypr-osd-desktop.lua` | **generated** by the settings window from that file, and loaded by `osd.lua` — the one file in your `~/.config/hypr` this collection writes, and only when you change a Hyprland setting |
| `~/.config/hypr/osd.lua` | the bar, the autostart, every keybinding, the layer rule and the generated Hyprland settings, generated from `scripts/hyprland/osd.lua` |
| `~/.config/hypr/hyprland.lua` | one appended line: `require("osd")` (backup: `hyprland.lua.bak`), then `hyprctl reload` |
| `~/.config/hypr/hyprland.lua` | waybar's autostart commented out, and waybar stopped (backup: `hyprland.lua.bak-waybar`) — see [Waybar](#waybar) |
| `~/.config/hypr/hyprland.lua` | `require("launchpad")` commented out, and the old launcher daemon stopped, because `SUPER + SPACE` now belongs to `hypr-osd-launcher` (backup: `hyprland.lua.bak-launchpad`) — see [Launcher](#launcher) |
| `~/.config/hypr-osd/wallpaper.conf` | the **Wallpaper** page's keys — hyprpaper's own options, as `key = value`, edited by the settings window (only created when you change something) |
| `~/.config/hypr-osd/lock.conf` | the **Lock screen** page's keys: every knob hyprlock has for the screen, edited by the settings window (only created if missing) |
| `~/.config/hypr-osd/login.conf` | the **Login screen** page's keys: SDDM's picture, rendering and palette, edited by the settings window (only created when you change something) |
| `~/.config/hypr/hyprpaper.conf` | **generated** from `~/.config/hypr-osd/wallpaper.conf` by the Wallpaper page; the installer writes the template only when hyprpaper is installed and the file is missing |
| `~/.config/hypr/hyprlock.conf` | **generated** by `hypr-osd-lock-vars` from `theme.css` and `~/.config/hypr-osd/lock.conf` — the whole lock screen, palette included. A hand-written file from before that is backed up once (`hyprlock.conf.bak-handwritten`). See [Lock screen](#lock-screen) |
| `~/.config/hypr/sddm-theme/theme.conf` | **generated** from `~/.config/hypr-osd/login.conf` by the Login screen page, beside the rendered `images/background.jpg` — see [Login screen](#login-screen) |
| `~/.config/hypr/hypridle.conf` | idle timers — only when `hypridle` is installed, and only if the file is missing |
| a running element | stopped and started again, once the new binary and the config repairs are in place: every element reads its binary, its config and the palette exactly once, at start-up, so a re-install that left the process alone would keep the old code until the next login. Only the elements that were running are restarted |

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
toolchain; at runtime `wpctl` (PipeWire/WirePlumber) and `hyprctl`. The bar reads a
few more things and simply leaves a pill out when one is missing: `iw` (the
network pill), `playerctl` (the media pill), `swaync-client` (the island's
notification hub) and `wpctl` (the volume pill). The island's notification *rows*
are read off the session bus with `busctl` (systemd ≥ 257), because swaync
publishes no list of them — without it the tile keeps its count and buttons and
loses the rows. The installer reports which of them it cannot find.

```sh
sudo pacman -S --needed base-devel pkgconf gtk4 gtk4-layer-shell rust
sudo pacman -S --needed pipewire wireplumber playerctl iw            # the pills
```

## Waybar

**waybar is gone.** It used to be this desktop's bar, and everything here was
built to sit beside it: the palette lived in `~/.config/waybar/style.css`, the
two apps mirrored it, and the cards read it out of that file at start-up. That
arrangement had two problems, and they are why `hypr-osd-bar` exists:

* **Two programs wanted the same strip of screen.** Both bars reserve an exclusive
  zone along the top edge, both draw a full-width surface there, and the layer
  rules that make the transparent parts click-through have to cover both. Running
  the two together produced overlapping pills, clicks landing on the wrong bar,
  and a `waybar` that had to be restarted whenever the theme changed.
* **The palette had no owner.** The bar that *defined* the colours was not the one
  that drew most of them, so every new element meant another parser for someone
  else's stylesheet.

So the palette moved into this repository (`configs/theme.css`), the bar moved
into it too (`crates/hypr-osd-bar`), and the installer takes waybar out of
the autostart — commenting the line out rather than deleting it, with
`hyprland.lua.bak-waybar` as a backup, and stopping the two GTK processes that
existed only to expand a waybar pill on hover.

Going back is a handful of commands (the files are all still there):

```sh
./scripts/install.sh --keep-waybar      # don't touch it in the first place
cp ~/.config/hypr/hyprland.lua.bak-waybar ~/.config/hypr/hyprland.lua
hyprctl reload && waybar &
```

…but the two bars do not coexist well, so stop this one first
(`pkill -f hypr-osd-bar`). The two hover panels (`island_panel.py`,
`stats_panel.py`) are replaced by the island popup, which is the same idea built
as an element: it registers its own surface, follows the pointer only while it is
open, and is styled from the same palette as everything else.

## Launcher

`SUPER + SPACE` is the same gesture it has been on this desktop for a while: a
search bar in the middle of the screen, applications and files, Enter to open.
What changed is what is behind it.

The old one — [`linux-launchpad`](../linux-launchpad), a Tauri app with a Next.js
front end — is a fine program built out of pieces this desktop does not want: a
WebKitGTK webview, a Node build step, and a bundled browser engine, all of it
resident, to draw a rectangle with a text field in it. It also kept a background
thread over a SQLite FTS5 index of every file it had ever seen, with an inotify
watcher to keep it fresh. `hypr-osd-launcher` is one GTK4 binary from this
collection: a card, a `GtkEntry`, a list of rows, and a bounded `read_dir` walk
for the file half. The search it does is the search the old overlay did — name,
generic name, keywords and categories, ranked on the same ladder — with the file
results merged into the same ranking instead of queued behind the applications.

Two things are worth knowing, both deliberate:

* **It takes the keyboard while it is up.** A layer-shell surface that wants to be
  typed into has no other way, and needing a click before the field accepts a
  character is not a launcher. The price is that a click elsewhere does not
  dismiss it: the compositor cannot take the keyboard back from an exclusive
  surface. So it ends the way the other keyboard cards do — Escape, or `SUPER +
  SPACE` again — with `idle_close_ms` (a minute) as the safety net for having
  walked away.
* **The card keeps its size.** A mapped layer surface grows with its content but
  does not shrink with it (measured: narrowing a full list down to one result left
  the card 566 px tall), and a card that resized under the pointer while you typed
  would be worse anyway. The list therefore reserves room for `max_results` rows
  and pads the tail with rows that hold space and show nothing.

Going back to the old launcher is the same shape as [Waybar](#waybar):

```sh
./scripts/install.sh --keep-launchpad   # don't touch it in the first place
cp ~/.config/hypr/hyprland.lua.bak-launchpad ~/.config/hypr/hyprland.lua
hyprctl reload && launchpad --hidden &
```

The installer only ever comments the `require("launchpad")` line out; the
application and its own `launchpad.lua` are left exactly as they were. (The one
key they cannot share is `SUPER + SPACE`: with both wired up, one press opens both
search bars.)

## Applications panel

The old launcher had **two** windows. One was the overlay on `SUPER + SPACE`; the
other was the main window behind it — a category list down the side, a grid of
every application installed, a search field across the top and the favourites
above the grid. [The card above](#launcher) is the first one; this element is the
second, rebuilt the same way and for the same reasons (one GTK4 binary, no
webview, no Node build), with one difference that is the whole point of it:

**It is opened by a button on the bar, not by a key.** The far-left pill of the
bar is a grid of squares; the pointer resting on it asks this element to `open`
(the pill lights up in the accent while the panel is up), and the panel takes
itself away again once the pointer has left both the button and the panel — the
same gesture as the island under the player pill and the system popup under the
status pill. A launcher you have to remember a key for is one you use when you
already
know what you want to run; a grid of everything installed is what you use when you
do not, and the pointer is already on the bar. `osd.lua` has a commented
`SUPER + A` bind if you want a key for it as well.

| | |
| --- | --- |
| Open it | hover the grid button at the bar's **left** end, or `hypr-osd-apps show` |
| Walk the tiles | the arrow keys (left/right/up/down, `Page Up`/`Page Down`, `Home`/`End`), or the pointer |
| Change drawer | `Tab`/`Shift+Tab`, a click on a drawer row, or `hypr-osd-apps drawer <name\|number>` |
| Open | `Enter`, or a left click on a tile |
| Pin | a **right click** on a tile (`~/.config/hypr-osd/apps-pinned`, one id per line) |
| Close | move the pointer off the panel, press `Escape`, or `hypr-osd-apps hide` |

The drawers are the freedesktop categories, mapped the way the old window mapped
them (`Development`, `Graphics`, `Internet & Networking`, …) with anything that
claims nothing in an `Other` drawer, and a `Pinned` drawer that exists only once
something is pinned. `hypr-osd-apps apps` prints every application with the drawer
it landed in, which is how "why is that one under Utilities?" gets answered, and
`hypr-osd-apps status` prints the count, the drawers, the pinned list and the size
the card came out at.

Two things are worth knowing, and they are the same two as the launcher's:

* **It takes the keyboard while it is up** — a layer surface that wants to be typed
  into has no other way, and the search field is the point. So it ends the way the
  other keyboard cards do: the pointer leaving it, `Escape`, or `idle_close_ms` (a
  minute by default) if you walked away.
* **The card keeps its size.** Picking a drawer with two applications in it cannot
  make the panel jump: the grid lives in a fixed-height scroller (`grid_height`,
  which is why that key wants to be a whole number of tile rows), so the card is
  the same shape whichever drawer you are on.

Unlike the launcher, nothing here waits for a key press: the button is the
trigger, and it is the *bar* that starts the element — `apps_command` in
`bar.conf`. The bar only ever asks the panel to *open*; the panel itself watches
the pointer (it is the only program that can see both the button and the card)
and closes once the pointer has left them, which is why a click on the button can
never leave it stuck. It is also why the panel is autostarted with the others: it
reads the installed applications once, while nobody is waiting, so the first
hover paints at once.

## Bluetooth manager

The system popup's bluetooth tile is a *report*: the controller's state, its
power and visibility switches, and the first few devices with their batteries and
link meters. It is deliberately small, because it sits in a column of readings
and a list longer than five rows would either scroll under the pointer (and close
the popup) or push the controls out of shape. But "the first five devices" is not
a manager, and the alternative used to be the **Manage** chip opening
`bluetoothctl` in a terminal.

This element is that other half: `hypr-osd-bluetooth` is a card of this
collection, opened by the tile's **Manage** chip (or `hypr-osd-bluetooth show`),
with every device BlueZ knows, a filter, a scan and the actions that go with
them.

| | |
| --- | --- |
| Open it | the **Manage** chip in the system popup's bluetooth tile, or `hypr-osd-bluetooth show` |
| Filter | type into the field on a laptop (it is focused when the card opens), or tap it on a tablet |
| Walk the rows | `Up`/`Down`, `Page Up`/`Page Down`, `Home`/`End`, or a click on a row |
| Act on a row | its button, or `Enter` on the selected row (connect, disconnect or pair) |
| Forget a device | the ✕ beside **Connect** on a paired, unconnected row |
| Scan | the **Scan** chip in the card - `bluetoothctl --timeout` exits by itself, and the exit re-reads the list |
| Controller | the ⏻ switch (the radio) and the eye (answer pairing requests) in the header |
| Close | `Esc`, the ✕, the Manage chip again, or `idle_close_ms` (two minutes by default) |

The two lists are **Paired** and **Nearby**: a device that has been paired before
is always listed, and a device a scan has merely *seen* is the row that offers
`Pair` (which pairs, trusts and connects it). The row is the same shape in both
lists: the kind of device as a Lucide drawing in a rounded badge, the name over
its address, the link quality as four blocks, the battery as a capsule, and the
buttons in a fixed slot at the end - so a row with one button and a row with two
keep their meters on the same edge. A connected device wears the accent bar at
its left edge; the card's one saturated mark is the bluetooth badge in the
header.

Three things are worth knowing, and they are the same three the other keyboard
cards follow:

* **It takes the keyboard while it is up** — the filter is typed into, and
  `Escape` is the way out. It lets go after `idle_close_ms` of silence, which is
  what makes that safe across a session lock (a locked screen takes the keyboard
  from every other surface, this card included).
* **A pairing that fails says so.** Pairing runs with a `NoInputNoOutput` agent,
  because the card has no passkey dialog: a device that insists on a passkey
  cannot be paired here, and the note under the list says so instead of silently
  going back to "Pair". `bluetoothctl` in a terminal can still ask for one.
* **The card keeps its size.** The list lives in a scroller at `list_height`, so
  a scan finding a dozen devices cannot make the card jump, and every name is
  capped so it cannot widen the card either.

The commands are the same ones the buttons run, so a script can do everything a
finger can. `hypr-osd-bluetooth status` prints the controller and every device,
with the addresses the verbs take; `power on|off`, `discoverable on|off`,
`connect|disconnect|pair|remove <address>`, `scan`, `refresh`, `show [filter]`,
`hide` and `toggle` are the rest.

## Settings

Every element reads its own `.conf` once, at start-up, and every key is optional —
which is a fine thing to document and a tedious thing to use. This element is the
other way in: one card, a page per element, a control per key, and the file
rewritten as the values change.

**It is opened by the gear at the right end of the system popup's header** (hover
the bar's status pill and the popup is there), and the gear wears the accent while
the window is up. It has no keybinding of its own; `osd.lua` carries a commented
`SUPER + SHIFT + S` if you want one.

It has **four tabs**, because this desktop is four things at once: the elements,
the compositor, the pictures behind them, and the machine they run on.

* **Hypr OSD** — this collection. Twelve element pages, with exactly the keys and
  defaults its own binary reads, then a twelfth that is not a form at all:
  **Shortcuts**, every key the collection implements. See [The Shortcuts
  page](#the-shortcuts-page).
* **Hyprland** — the compositor itself: the **displays** (`display`, one row per
  connected output — resolution and refresh rate, position, scale, rotation,
  mirror, on or off), the geometry (`general`), the corners and the blur
  (`decoration`), the keyboard and the pointers (`input`), the transitions
  (`animations`) and the switches nobody knows where to look for (`misc`). These
  are Hyprland's own options, so a change here is applied to the running session
  the moment you make it; see [The Hyprland tab](#the-hyprland-tab).
* **Appearance** — the desktop's three faces, which are one picture seen three
  times: the **Wallpaper** (hyprpaper — the picture, how it fills a screen, the
  slideshow it can be), the **Lock screen** (hyprlock — every knob for the
  background treatment, the glass, the clock, the date, the field and the hint,
  grouped so eighty rows read as a screen rather than a wall) and the **Login
  screen** (SDDM — the picture, the rendering that blurs it for the greeter, the
  face and the palette). The two picture pages open with the picture itself and
  its size, every background row carries a **file chooser** (plus a folder button
  where a directory is meaningful), and the Wallpaper page's buttons point the
  lock and login screens at the same picture in one click; see
  [Wallpaper](#wallpaper), [Lock screen](#lock-screen) and
  [Login screen](#login-screen).
* **System** — the machine under all of it: the **energy policy**, which opens with
  live readings (the power profile and its driver, the CPU governor and frequency,
  the battery, and which managed effects are on right now) and then the switches
  that say which effects to drop while the machine is saving, plus two buttons to
  act now; see [The System tab](#the-system-tab-energy-saving).

| | |
| --- | --- |
| Open it | the gear in the system popup's header, or `hypr-osd-settings toggle` |
| Switch tabs | a click on `Hypr OSD` / `Hyprland` / `Appearance` / `System`, or `hypr-osd-settings tab <name\|number>` |
| Pick a page | a click on the list down the left, or `hypr-osd-settings section <name\|number>` |
| Change a value | the control on the row: a switch, a spinner, a text field (Enter or leaving it commits), a dropdown for a value that is one of a list, or a path with a folder button that opens the desktop's file chooser (the window steps aside while it is up, because a layer surface would otherwise sit on top of it) |
| Reset one key | the ↺ beside a key that is set — the line is *removed*, so the built-in default applies again |
| Apply it | the **Restart** chip on an element's page (that element stops and starts again), the **Apply** chip on a Hyprland one, the **Write** chip on a Wallpaper, Lock screen or Login screen page, or nothing at all: a Hyprland value, a wallpaper, a lock screen and a login screen are all applied as you change them |
| Close | `Escape`, the ✕, or `hypr-osd-settings hide` |

Three things about it are worth knowing:

* **The file stays the source of truth.** A change rewrites exactly one line
  (`hypr-osd-core`'s `config::edit_value`), in place, so the comments and the order
  of a file you have edited by hand survive; a key set back to its default is
  *deleted* rather than written out, because in this format an absent key means
  "whatever the element's default is". A key the file sets that the catalogue does
  not know about is still shown, as plain text, so nothing is invisible here.
* **On the Hypr OSD tab a change takes effect when that element is restarted** —
  the config is read once, at start-up, so there is nothing to push it into a
  running daemon. That is what the Restart chip is for: it stops the element and
  starts it again, waiting for the old one to let go of its D-Bus name (an element
  that starts too early would just forward its empty command line to the process
  that is going away and leave nothing running). The Hyprland tab is the opposite —
  the compositor takes a change at once. The **Appearance** tab needs neither:
  hyprlock reads its config at every lock, hyprpaper and the greeter read theirs at
  start-up, so a change regenerates the file and is live at the next lock or login,
  with nothing to restart (the wallpaper is also handed to the running hyprpaper
  through its IPC, which is why the picture changes on screen at once).
* **It takes the keyboard while it is up**, because the text fields have to be
  typed into and a layer-shell surface has no other way. It ends with `Escape`, the
  ✕, or `idle_close_ms` (two minutes by default) if you walked away — and the card
  is one size whichever page is on it.

The pages, the labels and the defaults come from `hypr-osd-core`'s `settings`
module — one table per element, with the control each key wants — so the window and
the elements cannot disagree about what a key is called or what it is worth unset.
A test holds every default against its own control type. The window itself is
scriptable, which is also how it is tested:

```sh
hypr-osd-settings sections              # the four tabs, and the pages of each
hypr-osd-settings tab hyprland          # ...and show one of them
hypr-osd-settings tab appearance        # the wallpaper, the lock screen and the login screen
hypr-osd-settings keys bar              # every catalogue key of one element, set or default
hypr-osd-settings keys general          # a Hyprland page, plus what the compositor has each option at *now*
hypr-osd-settings keys display          # every connected output, field by field
hypr-osd-settings keys shortcuts        # every key this collection implements
hypr-osd-settings choices input         # every name the dropdowns of one page offer
hypr-osd-settings set bar tray_icon_size 22
hypr-osd-settings unset bar tray_icon_size
hypr-osd-settings set decoration decoration:rounding 12   # applied to the running session
hypr-osd-settings apply                 # write the generated Lua again and hand it over
hypr-osd-settings restart bar
hypr-osd-settings command apply           # run the shown page's button (System: apply/restore)
hypr-osd-settings status                # the tabs, the pages, the files, the last apply, open or closed
```

### The Shortcuts page

The last page of the Hypr OSD tab is a list rather than a form: every key this
collection implements, drawn as a keycap with what it does under it. Nothing on it
can be changed, so it has no chip - the footer says where the bound half lives
instead.

Its two groups come from different places, which is the point of the page. The
**bound** keys - the volume steps and mute, the session card, the tray, ALT + TAB,
the overview, the launcher and the lock - are binds in `osd.lua`, so Hyprland
sends them from anywhere, card or no card. `scripts/hyprland/osd.lua` is their
source of truth: `cargo test` reads that template at compile time and requires the
table and the file to name each other's binds, so a key added to the template and
forgotten on the page (or the other way round) is a test failure. The **cards'
own** keys - Escape, the arrows, Tab, Enter, 1-9 - are read by the element itself
while its card owns the keyboard; Hyprland never sees them, which is also why the
compositor cannot be asked what they are (a Lua-configured bind answers `__lua`
and a function number, not the command it runs).

`hypr-osd-settings keys shortcuts` prints the same list without the window.

### The Hyprland tab

The elements read a `.conf` file, Hyprland reads the Lua in `~/.config/hypr`, and
the two do not sound alike: an element's `tick_ms = 1000` and the compositor's
`decoration:rounding = 10` are different kinds of thing. So the second tab keeps a
file of its own, `~/.config/hypr-osd/desktop.conf`, in the same `key = value` shape
— the keys being **option paths**, the names `hyprctl getoption` answers to — and
the window translates it:

* **A change is live.** The window regenerates `~/.config/hypr/hypr-osd-desktop.lua`
  from the file — a generated document: one `hl.config({...})` call, holding
  *only* the options you have set, each table nested the way the path says, then
  one `hl.monitor({...})` call per output the Display page has been given a value
  for — and
  hands it to the compositor with `hyprctl eval 'dofile("…")'`. That is how this
  Hyprland takes a runtime option change; `hyprctl keyword` is refused by its Lua
  parser. `dofile` and not `require`, because a `require` would hand back the first
  version it ever read. The **Apply** chip re-runs it, and each row shows the
  value the compositor *reports* for the option (`hyprctl -j getoption`), so the
  window cannot show a number the session is not using — including the case where
  your own `hyprland.lua` sets something else.
* **It survives a login**, because `osd.lua` loads that same generated file at the
  end of its own run: `dofile` inside a `pcall`, so a missing file (nothing set
  yet) or one this Hyprland refuses (an option it does not have) is a line in the
  compositor's log rather than a config that does not load. Since `require("osd")`
  is the last thing in `hyprland.lua`, what you set here wins over a line set by
  hand earlier in your config — and your config is never rewritten.
* **A value reset with the ↺ is a line removed** from `desktop.conf`, and therefore
  an option this collection stops having an opinion about: from the next login your
  own value is the one in force again. Until then Hyprland keeps what it has,
  because there is no way to *unset* an option at runtime — setting it back to the
  value your config uses is the honest way to undo it.
* **Only options this Hyprland actually has are listed**, and only those are ever
  written into the document: a `hl.config` call naming an option that does not
  exist is refused *whole* ("unknown config key"), which would take the rest of the
  settings down with it. A key you add to `desktop.conf` by hand that the tab does
  not list stays in the file (and is shown in the window, as text) but is not part
  of what the compositor is handed.
* **A value that has to be a *name* is picked from a list, never typed.** That is
  the keyboard rows, and it is the one place where a wrong value has no error
  message at all: `kb_layout = de` with `kb_variant = ch` (the two swapped, or a
  variant that belongs to another layout) is accepted by `hl.config`, and then xkb
  is the thing that quietly does not work. So the layout, the variant, the model
  and the options are dropdowns, and the lists come from this machine's own
  xkeyboard-config (`hypr-osd-core`'s `keymap`): the layout and the variants with
  the descriptions the data has for them, so `ch` reads as `German (Switzerland)`
  and not as a two-letter code. The *variant* list is the variants of the layout in
  force, asked of the compositor — a variant belongs to a layout, and the list
  changes with it. A value the list does not know (a comma list like `de,us`, or a
  name this machine has never heard of) is still shown, as an entry of its own,
  labelled `not a variant of de` / `unknown name` — which is how such a setting
  becomes visible instead of merely broken. `hypr-osd-settings choices <page>`
  prints those lists without opening the window.
* **The Display page is a group per output, not a list of keys.** Which outputs
  exist is the compositor's to say, so the page is built when it is drawn:
  `hyprctl -j monitors` gives each monitor's current mode, position, scale,
  rotation, mirror and whether it is on, and that is what an unset row shows — the
  value actually in force, not a made-up default. Each output gets a heading — its
  connector, what it is showing right now (`2560 × 1440 · 59.95 Hz`) and, on the
  screen you are on, a `focused` mark — with its six fields under it. `mode`,
  `scale` and `mirror` are dropdowns whose labels are friendlier than the values
  they write (`2560 × 1440 · 59.95 Hz`, `1.5 · 150 %`, a mirror candidate with the
  picture it would copy); position and rotation are a field and a spinner. A key is
  the field qualified by the output — `monitor:eDP-1:mode` — and `hypr-osd-settings
  set desktop monitor:eDP-1:scale 2` writes it exactly as the window would. One
  thing to know: an `hl.monitor` rule *replaces* everything known about that
  output, so a field you have not set is filled from what the monitor is doing at
  the moment (Hyprland's default, for an output that is not plugged in) — otherwise
  setting the mode would silently reset the scale. An output with no key at all is
  left completely alone, so a machine that never opens this page keeps whatever its
  own `hyprland.lua` says.
* **The page opens with the layout itself, and screens are dragged into place.**
  Each output is a box whose proportions are its *logical* size — resolution
  divided by scale, so a 2x panel is not drawn twice as wide as a 1x one, exactly
  as Windows and macOS draw theirs — and the whole layout is fitted into the pane,
  so a screen at a negative position is still on the picture (a stack, taller than
  it is wide, gets a taller canvas: two screens above one another would otherwise
  shrink to a stripe). Dragging a box writes that output's
  `monitor:<output>:position` and hands it to the compositor like any other row.
* **The gesture decides where a screen goes, not the distance.** The picture is
  fitted to the layout as it *is*, so there is often no room on it to drag a screen
  under another — the laptop beside a desk monitor can move a few pixels down and
  no further. So a *deliberate* vertical drag puts the screen above or below the
  nearest output, centred against it when the widths differ, and a deliberate
  horizontal drag puts it beside; the box previews that slot while the pointer is
  still travelling, so the picture shows the result before the release does. A
  drag shorter than about 24 canvas pixels is still a nudge, and the fields below
  the picture are there for exact numbers. A screen dragged *past* the canvas's
  edge parks beyond everything, for a new row of its own.
  `hypr-osd-settings drop <output> <dx> <dy>` answers what a drag of that many
  canvas pixels would write, without moving anything — how a drop that surprises
  you is explained.
* **Screens never overlap.** A drop that would land on top of another output is
  pushed clear along the gesture's axis, and the whole arrangement is checked again
  after the snap, so a snap can never undo the push. Edges, middles and the
  layout's origin are snap targets within a few pixels (Hyprland would otherwise
  leave a sliver of desktop between two screens that the pointer cannot cross). A
  click that moves nothing writes nothing, so looking at the picture never pins a
  value nobody chose.
* **The page opens with the collection's own choice: the primary display.** One
  row, above the per-output groups, picks the output the notification cards — the
  volume, media and session cards — appear on, or leaves it on `Follow focus`,
  which is the behaviour every card had before. It is the one key on the page that
  is *not* a monitor field and is never handed to the compositor (Hyprland has no
  notion of a primary output): it is a `primary` key in `desktop.conf`, read back
  by the elements themselves, and it takes effect on the next card rather than
  needing a restart. `hypr-osd-settings keys display` prints it, and `hypr-osd-settings
  set desktop primary eDP-1` writes it.

### The System tab: energy saving

The **System** tab's one page is the energy policy. It is not a Hyprland page and
not an element page, though its values live in an element's file
(`~/.config/hypr-osd/energy.conf`) and are applied to the compositor: it is the
collection's *own* policy about the compositor, which is why it gets a tab.

* **It opens with data, not controls.** Four readings from the kernel and the
  compositor: the **power profile** and the platform driver behind it
  (`powerprofilesctl`), the **CPU** governor and the frequency it is running at
  (`/sys/…/cpufreq`), the **battery's** charge, state and time left at the current
draw, and **which of the managed effects are on** right now. They are drawn as
  rows with no control — a reading is not a setting — and `hypr-osd-energy status`
  prints the same numbers without a window.
* **The policy is one switch per effect.** Animations, the two dragging
  animations, blur, shadows, dimming, transparency (one switch for both opacities)
  and rounded corners, plus the **trigger profile** (which power profile counts as
  saving — `power-saver` by default) and how often the profile is re-read. A change
  takes effect when the daemon is restarted, like any element page: the **Restart**
  chip is what does it.
* **Two buttons act now** — **Save now** and **Restore** — and they run
  `hypr-osd-energy apply` / `restore`, so the daemon does not have to wait for its
  next profile poll. *Save now* is deliberately sticky: it keeps the effects off
  until *Restore* is pressed (or the daemon restarts), so it is also the way to
  save energy while the machine is in a profile you would rather not change.
  (Clicks cannot be scripted on a layer surface, so the window carries a
  `command <id>` verb that runs a page's button exactly as a click would.)

The daemon itself (`hypr-osd-energy`, element #9) is **headless** — its business
is the compositor's options, not a card, so it never shows one — and it is
autostarted, because a machine that boots straight into `power-saver` should not
have to wait for the first poll to drop its blur. Two rules make it safe:

* **It remembers before it overrides.** The first time the saving profile takes
  hold, every effect key is read off the *running* compositor and written to
  `$XDG_RUNTIME_DIR/hypr-osd-energy.state`. Leaving the saving state puts those
  values back, so a machine whose own `hyprland.lua` says something this collection
  does not know about gets it back exactly. A daemon restarted mid-save finds the
  remembered values and restores them rather than mistaking its own overrides for
  the machine's own.
* **It only writes when it disagrees.** Each poll compares the values it wants with
  the ones the compositor has (a handful of `hyprctl getoption` reads); when they
  match, nothing runs. Nothing is written at all before a saving profile has taken
  hold once, so a machine that has never saved keeps its own values completely
  untouched. That is also what makes it self-healing — a settings change that
  restored an effect under a saving profile is corrected on the next poll — and the
  reason the daemon leaves the effects completely alone when `powerprofilesctl`
  does not answer at all.

It hands the compositor its own small generated document
(`~/.config/hypr-osd/energy.lua`, one `hl.config({...})` call, the same shape the
settings window writes) and applies it with `hyprctl eval 'dofile("…")'`. The
file is complete every time — there is no way to *unset* an option at runtime — so
the last program to apply a document is the whole truth, and the settings window's
Hyprland document and this one compose rather than fight.

## Use

Every element answers to verbs on its own binary, and they work the same whether
its daemon is already running or not: the binary is single-instance, so a second
invocation forwards its arguments to the running instance over D-Bus (and gets
the exit status and any output back). That is also why a keybinding can just be
`exec, hypr-osd-volume up`.

The bar and the island popup are daemons like the rest, and answer to verbs the
same way — they simply have nothing bound to them.

**`hypr-osd-bar` — the bar**

```sh
hypr-osd-bar            # start the bar (what the autostart runs)
hypr-osd-bar show       # make sure it is up
hypr-osd-bar hide       # take it away until the next rebuild
hypr-osd-bar toggle     # show or hide
hypr-osd-bar refresh    # re-read every pill now
hypr-osd-bar status     # print what every pill currently reads, no bar
hypr-osd-bar minimise   # put the focused window away, into the tray
hypr-osd-bar restore    # bring the last put-away window back
hypr-osd-bar minimised  # list what is put away, no bar
```

It has one keybinding, and it only has that one because the bar is where the
tray is: `SUPER + H` puts the focused window away and `SUPER + SHIFT + H` brings
the last one back (see the tray below). Everything else about it is a verb,
because a bar is not something you call for. `status` is the one to remember: it
prints the workspace row, the focused window's title, what is playing, what is
put away, the sink's level, the battery, and the wireless link as the bar sees
them — which answers "why does the volume pill say 0 %" without guessing. `restore`
takes an optional argument in place of the last one: the number the icon has in
`minimised` (counting from 1), or the window's `0x…` address.

The bar's left end is the grid button, then the workspaces (click to focus one,
wheel to walk to the next or previous) and the focused window's title. The
workspace you are on wears its number in a pale mint capsule; the rest are dots -
solid where a window lives, hollow where the workspace is empty, mint-tinted while
it is up on another monitor - so the row reads at a glance without counting from a
fixed edge.

The capsule is a widget of the row's own rather than the active pill's
background, because a background cannot slide from pill to pill: the row rebuilds
its pills only when the *set* of workspaces changes and restyles them in place on
a switch, so the capsule always has a pill to leave from. Rather than jump to the
new pill, the capsule glides from the one you were on over 180 ms on an
ease-out-cubic curve — the same size of gesture as the pills' own colour
transitions — with the number inside changing at the halfway point, where the
movement hides the change; a switch that arrives mid-glide is retargeted from
wherever the capsule has got to, and the first paint and a desktop with GTK's
animations switched off both put it on the pill without gliding. It is drawn
frame by frame from a tick callback rather than declared in CSS, because where it
has to go is only known once the row has been laid out: the move starts and ends
at two pills whose positions no stylesheet can ask for, and an overlay child is
placed by its alignment alone. It has no key of its own: it follows GTK's
animations switch (`gtk-enable-animations`), the accessibility switch for
motion.

The middle is the player, the one pill that is not about this machine - and the
island popup's handle: hover it and the panel unfolds below it. It carries the
cover art (from `mpris:artUrl`, decoded from a file or fetched with `curl` through
`core::art`; the player's own mark stands in while it loads and when the player
publishes none), the track and its artist, and a small play/pause mark - click to
play/pause, right-click for the next track, middle-click for the previous one,
wheel to seek. A finger's tap opens the island instead, where the transport is
sized for it.

Right: the tray and the windows put away, the **combined status pill** (the link,
the sound and the battery, and the handle that unfolds the system popup: hover or
click it, right-click to toggle the wireless radio, middle-click to mute the
microphone, wheel to step the volume *through the volume element*), the power
button (which opens the session card) and, last, the clock: the time over the
date. Every pill has a tooltip with the whole story behind it.

The left and right parts are one *tab* each, and the player is the third - three
pills with the wallpaper showing between them. **The bar paints nothing behind
them**: its surface is transparent, `bar.conf`'s `margin_top` and `margin_x` are
0, and so each outer tab is welded into a screen corner - square where it meets
the top and side borders, a deep 22px curve only on the corner facing the
desktop, with a warm gradient pouring down from the edge. The strip the surface
still occupies is what the compositor reserves, so tiled windows start below the
tabs rather than under them.

The tray at the right end carries two kinds of icon. The **application
indicators** (Steam, the Nextcloud client, nm-applet …) come from the
StatusNotifierItem protocol — the bar *is* the tray host, which is what makes
them visible at all. To their left stand the windows you have **put away**:
Hyprland has no minimise of its own, so `SUPER + H` moves the focused window onto
a special workspace that is never shown and the bar draws it as one more small
icon. A click brings that window back to the workspace it came from, `SUPER +
SHIFT + H` brings back the one you put away last, and `hypr-osd-bar minimised`
lists them all. The tooltip of an icon says which window it is, which application
it belongs to, and where a click will put it.

There is no state in between: the list of icons is read from Hyprland every time,
so it survives the bar being restarted — a restart only loses *where* each window
came from, which the tooltip admits (those windows come back to the workspace you
are on). A put-away window is on `special:minimized`, one workspace name rather
than a mechanism of the bar's own, which is why the switcher and the overview
leave it out: they offer the windows you can actually switch to.

**`hypr-osd-island` — the player's popup**

```sh
hypr-osd-island open    # what hovering the bar's player pill runs
hypr-osd-island close   # take it away
hypr-osd-island toggle  # close it if it is up, otherwise show it
hypr-osd-island show    # put it up and keep it there (no pointer tracking)
hypr-osd-island status  # what the panel would show, without showing it
```

Hovering the player pill in the bar unfolds a panel below it: the time and date,
the player below, the notification hub, and a calendar you can page through by
month, with today in the accent colour.

**The player** is everything MPRIS offers except the queue. The cover comes from
`mpris:artUrl` (a `file://` URL decoded, an `http(s)://` one fetched with `curl`,
anything else left to the glyph it falls back to — the same loader the media card
uses, `core::art`), with the title, the artist and the album beside it. Under it a
progress bar and the times, which are one click target: **a click seeks** to where
it landed. Then the five transport buttons — shuffle, previous, play/pause, next,
repeat — where shuffle and repeat wear the accent while they are on, and the
repeat tooltip says which kind it is (this track, or the whole queue). Then the
player's **own** volume, drawn only when the player reports one (a browser usually
does not), which is deliberately not the sink's: a player at 40% of a sink at 80%
is an ordinary state, and the sink's step belongs to the volume element.

When more than one player is running, a row of chips appears under the volume
with one per player — `vlc`, `firefox` — and the one on screen wears the accent.
A click on another chip switches the whole tile to it, transport and all. Nothing
is offered while there is only one player, because a choice of one is not a
choice. The list comes from `playerctl --list-all`, with the duplicate names one
player can register collapsed: VLC claims `vlc` *and* `vlc.instance10492` for a
single process (both names resolve to the same PID), and two chips for it would be
a lie. The tile follows `playerctl`'s own choice until a chip is clicked, which is
the same choice the media keys make.

The notification hub is read in two halves, because `swaync` publishes them
separately. The *state* — how many are waiting, do-not-disturb, whether the
control centre is up — comes from `swaync-client -swb`, one JSON line per change,
and drives the count, the dnd switch and the four buttons (open the control
centre, hide what is showing, clear everything). The *notifications themselves* —
the application, the summary, the icon, how long ago — swaync does not publish at
all: no verb lists them, and its own D-Bus objects expose nothing but GTK's own
interfaces. So the rows are read off the bus instead, with
`busctl --user --json=short monitor org.freedesktop.Notifications`, whose output
is one JSON line per D-Bus message — the `Notify` calls going in (application,
summary, body, icon hint, urgency) and the `NotificationClosed` signals coming
back out, so a notification that is dismissed or expires leaves the list again.
The newest few are drawn, each with the application's icon (or its first letter),
the application's name, the summary and the age; a critical one wears the warning
colour, `notification_rows` in `~/.config/hypr-osd/island.conf` says how many are
shown, and `+N more` admits the ones the daemon is holding that arrived before the
panel started.

Monitoring the bus is a privileged operation, which is why the two halves are
kept apart: where it is refused the rows stay away and the count, the dnd switch
and the buttons still work.

The island is a program of its own because a GTK widget can never paint outside
its own window: an "expansion" of a 40-pixel pill is necessarily a second
layer-shell surface. It also does the hover logic itself — the bar only says
"the pointer is on the player pill" (`open`), and the panel then samples the pointer to
decide when to appear and when to go away, because *leaving* means moving onto
the panel, somewhere the bar cannot see. Nothing is sampled while the panel is
closed, so an island that nobody is hovering costs nothing at all.

**`hypr-osd-stats` — the system popup**

```sh
hypr-osd-stats open eDP-1 1097 5 192 28   # what hovering the bar's status pill runs
hypr-osd-stats close    # take it away
hypr-osd-stats toggle   # close it if it is up, otherwise pin it up
hypr-osd-stats show     # put it up and keep it there (no pointer tracking)
hypr-osd-stats wifi     # switch the wireless radio (what the pill's right click runs)
hypr-osd-stats bluetooth on|off|scan                            # the controller
hypr-osd-stats bluetooth connect|disconnect|pair|remove MAC     # one device
hypr-osd-stats status   # every reading, the devices and their addresses, no card
```

The panel is the island's right-hand twin and works the same way: the bar only
says "the pointer is on the pill" — with the pill's own rectangle, so the panel
can measure its hot zone — and the panel samples the pointer from then on,
because *leaving* means moving onto the panel itself, somewhere the bar cannot
see. Nothing is sampled while it is closed.

Left column: three gauges (CPU, memory, temperature, with the same amber/red
thresholds as the bar's pill) and the **bluetooth** radios — the controller's
state with a power and a *visibility* switch (whether it answers pairing
requests), then a row per device BlueZ knows, each with the icon it reports,
its link quality as a signal meter and its battery as a pill (warning-coloured
when it runs low), and the action that follows from its state: disconnect what
is connected, connect — or *forget* — what is paired, pair what a scan found.
Right column:
the controls as a table — the wireless link, the sound, the power profile as a
switch you click *directly* (no cycling), presentation mode, the brightness
slider and the keyboard layout — with the pending updates under them. The device
list is re-read only while the panel is open, and it costs a `bluetoothctl` per
device, so a closed panel asks the controller nothing at all.

`status` is the one to run when the panel looks wrong: it prints every reading,
the devices with their addresses (which is what `bluetooth connect` wants), the
width the card actually came out at, and the two hover zones in the layout.

**`hypr-osd-volume` — the volume card**

```sh
hypr-osd-volume up        # +5 %, unmuting on the way up
hypr-osd-volume down      # −5 %
hypr-osd-volume toggle    # mute/unmute, keeping the level
hypr-osd-volume set 40    # an absolute level
hypr-osd-volume show      # reveal the card without changing anything
hypr-osd-volume status    # print the level, no card (for scripts)
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

The card's rows are the five actions the desktop offers for this: there is no
second session menu anywhere now that waybar is gone, so the card is *the* menu
(it was designed from the old `~/.config/waybar/scripts/popup.py`, whose
commands and wording it keeps, and whose icons it now draws as Lucide marks).
The "destructive actions ask twice" rule still holds:
`Reboot` and `Shut down` arm on the first click and fire on the second, so a card
that appeared under the pointer cannot end your session by accident. A verb is
never confirmed — typing or binding it is already deliberate. **Lock** uses
whatever screen locker is installed (and **Suspend** locks first, then sleeps) —
see [Lock screen](#lock-screen).

**`hypr-osd-switcher` — the window switcher**

```sh
hypr-osd-switcher next     # open the card and step forward (what Alt+Tab runs)
hypr-osd-switcher prev     # the same, backwards (Alt+Shift+Tab)
hypr-osd-switcher show     # open it without moving the selection
hypr-osd-switcher commit   # switch to the selected window, and close
hypr-osd-switcher cancel   # close without switching
hypr-osd-switcher status   # the list, in switcher order, and no card
```

It is one of the four elements that take the keyboard, and it has to be: a switch
ends when you let go of Alt, and the only way to see that release is to own the
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

**`hypr-osd-overview` — the workspace overview**

```sh
hypr-osd-overview toggle   # open it, or close it again (what SUPER+Shift+Tab runs)
hypr-osd-overview show     # open it
hypr-osd-overview close    # close it without switching anything
hypr-osd-overview recapture  # take fresh pictures of every window (what R does)
hypr-osd-overview move <window> <workspace>   # what dropping a tile does
hypr-osd-overview status   # the workspaces and the windows, and no card
```

One tap of `SUPER + SHIFT + TAB` and the desktop is on the screen as a picture of
itself: the workspaces that exist as a row of chips along the top (the one you
are on wears the accent), and every window as a tile, drawn in its own shape and
scaled until all of them fit on one screen. The tiles are *captures*, not icons —
a photograph of the window's own contents, taken when the card opens — so a window
behind three others, or on a workspace that is not on screen at all, shows what is
really in it.

Arrows (or `h`/`j`/`k`/`l`) walk the tiles, Enter or a click goes to the one you
are on, `1`–`9` jump to that workspace, `R` takes the pictures again (for a window
that appeared while the card was up, or a capture that failed), and Escape — or a
click on the card where there is no tile — closes it. `Shift + Tab` walks
backwards.

**A tile can be dragged onto a workspace chip** (or onto another tile, which means
"wherever that one lives") to move that window there. The window goes, the desktop
does not follow — the card is where workspaces are arranged, not where you go —
and the card stays up, so several windows can be put away in one look. It is the
same gesture with a fingertip, and `move <window> <workspace>` is what it does
from a script: a drop goes through `hypr-osd-overview move`'s code, so that is how
the gesture is checked on a machine where a pointer press cannot be scripted (the
window is named by its address or by its position in the tile order).

Like the switcher, this card takes the keyboard while it is up: Escape and the
arrows have to be the card's own keys, or they would be fighting whatever is
behind the card. It is one of the three elements that do, and all of them are
cards you opened on purpose. Because it takes the keyboard, it also lets go of it
by itself: after
a minute with no key and no mouse movement over it (`idle_close_ms`) the card
closes. The case that makes this necessary is a session lock — a lock takes the
keyboard from every other surface, this one included, so a card that was up when
the screen locked would otherwise sit there for good.

**`hypr-osd-launcher` — the search card** (see [Launcher](#launcher))

```sh
hypr-osd-launcher toggle        # open it, or close it again (what SUPER+Space runs)
hypr-osd-launcher show [query]  # open it, with an empty field or the query given
hypr-osd-launcher hide          # take it away (`close` is the same thing)
hypr-osd-launcher refresh       # re-scan the installed applications
hypr-osd-launcher search <text> # print the ranked results, without a card
hypr-osd-launcher apps          # list every application that was found
hypr-osd-launcher status        # how many applications, which file roots, open or closed
```

`SUPER + SPACE` opens a card in the middle of the screen with the keyboard already
in it: type, and the list narrows as you go. `↑`/`↓` (or `Page Up`/`Page Down`)
walk it, Enter opens the selected row, Escape — or the key again — takes the card
away. A click on a row opens it too.

The applications come from the freedesktop `.desktop` entries, filtered to the
ones that should be offered in this desktop (`Type=Application`, no `NoDisplay` or
`Hidden`, and `OnlyShowIn`/`NotShowIn` honoured). What you type is matched against
the name first, then the generic name, then the keywords, categories and comment;
`search` prints the ranking it decided on, which is what to run when the card
offers something you did not expect. Enter hands the entry to `gio launch`, so
`Exec` field codes, `Terminal=true` and `TryExec` behave exactly as they do in the
application menu.

Files come second and are found the honest way: a bounded walk of `Documents`,
`Downloads`, `Desktop`, `Pictures`, `Music`, `Videos`, `Projects`, `Templates` and
`Public`, on its own thread, debounced per keystroke. There is no index and no
watcher — the launcher this element replaces kept a SQLite FTS5 index of every file
it had ever seen, which is the right answer for millions of files and a lot of
machinery for a desktop. Applications and files are then ranked **together**, so a
file whose name starts with what you typed beats an application that merely
mentions it somewhere. `files = false` in `launcher.conf` turns the file half off
entirely.

**`hypr-osd-apps` — the applications panel** (see [Applications panel](#applications-panel))

```sh
hypr-osd-apps open             # what the bar's button runs: the pointer arrived
hypr-osd-apps toggle           # open it (pinned), or close it again
hypr-osd-apps show [query]     # open it (pinned), with an empty field or the query given
hypr-osd-apps hide             # take it away (`close` is the same thing)
hypr-osd-apps refresh          # re-scan the installed applications
hypr-osd-apps drawer <name|n>  # open it on one drawer, by name or by number
hypr-osd-apps pin <id>         # pin an application (a right click on a tile does this)
hypr-osd-apps unpin <id>       # ... and undo it
hypr-osd-apps apps             # one line per application: drawer, id, name
hypr-osd-apps status           # applications, drawers, pins, open or closed, the size
```

The trigger is the grid button at the bar's left end; the panel's own notes are in
the [section above](#applications-panel). `drawer 3` and `drawer Pinned` are the
scriptable equivalents of clicking a drawer row, and `apps` is what answers "which
drawer did that application land in?" without a card. `open` is the one the bar
sends (it only *asks*, so the panel can close itself when the pointer leaves);
`show` and `toggle` pin the panel, which is right for a script and wrong for the
bar.

**`hypr-osd-bluetooth` — the bluetooth manager** (see [Bluetooth manager](#bluetooth-manager))

```sh
hypr-osd-bluetooth toggle                 # what the popup's Manage chip runs
hypr-osd-bluetooth show [filter]          # the card, with the filter pre-filled
hypr-osd-bluetooth scan                   # look for devices nearby, and open the card
hypr-osd-bluetooth refresh                # read the controller and the devices again
hypr-osd-bluetooth power on|off           # the radio itself
hypr-osd-bluetooth discoverable on|off    # answer pairing requests, or stop
hypr-osd-bluetooth connect <address>      # pick a paired device up
hypr-osd-bluetooth disconnect <address>   # let a connected device go
hypr-osd-bluetooth pair <address>         # pair, trust and connect a scanned device
hypr-osd-bluetooth remove <address>       # forget a paired device
hypr-osd-bluetooth status                 # the controller, every device, the state
```

The addresses are the ones `status` prints. The card's own buttons never go
through the verbs - they *are* the element - but every verb is the same path a
button takes, so a script can reproduce a click exactly. `hypr-osd-stats
bluetooth on|off|scan|connect|disconnect|pair|remove <address>` still works for
the popup's compact tile, because both elements share `hypr-osd-core`'s bluetooth
module.

## Touch

This desktop runs on a Surface Pro 7, so the bar and every card are built to be
used with a finger as well as with a pointer: a tap opens the same panels a hover
does, a long press is the right click, the controls grow while the machine is
being held as a tablet, and an on-screen keyboard appears when something asks to
be typed into. None of it is a mode to enable. A machine with no touchscreen
behaves exactly as it did before, and even on this one the mouse and the Type
Cover keep working the way they always have.

The pieces are:

1. **The bar and the cards are laid out for a fingertip** while the machine is a
   tablet — the bar grows, the pills and rows get 44-pixel targets, the sliders
   get a thumb you can grab, the scrollbars get wide enough to drag. That is the
   shared `touch` CSS class (see [Look and feel](#look-and-feel)); nothing is
   duplicated per element.
2. **A panel opened by a finger stays up until you tap outside it.** There is no
   pointer to leave, so a tapped pill opens its panel *pinned* and the shell dims
   the rest of the screen with a [tap catcher](#the-tap-catcher) — a tap anywhere
   outside the card closes it. The same panel opened by hovering still closes the
   way it always did when the pointer leaves. Tapping the pill again closes it
   too, and so does `Escape` on the cards that take the keyboard.
3. **A long press is the right click.** The tray's context menus, pinning an
   application, the clock's notifications: everything that used to need button 3
   is on a long press as well, and the mouse keeps its right button.
4. **An on-screen keyboard** (squeekboard) appears when the launcher, the
   applications panel or the settings window has a field focused — and only on a
   tablet, because a keyboard daemon on a laptop would cover the field it is
   supposed to be typing into.
5. **Gestures** open the panels that a flat screen has no button for: sweep down
   from the top edge for the system popup, up from the bottom for the workspace
   overview, and so on. They need the [hyprgrass](#gestures) plugin.

### Tablet mode, and what it means

The Surface's Type Cover has a **tablet-mode switch**, and it is the only thing
that knows the keyboard has been folded back. Hyprland 0.56 does not publish
switch changes on its event socket — about the only way to see one is a *bind* —
so `osd.lua` binds the switch's two directions to the bar:

```lua
hl.bind("switch:on:Microsoft Surface Type Cover Tablet Mode Switch", hl.dsp.exec_cmd(bar .. " tablet on"))
hl.bind("switch:off:Microsoft Surface Type Cover Tablet Mode Switch", hl.dsp.exec_cmd(bar .. " tablet off"))
```

The bar writes the answer into `$XDG_RUNTIME_DIR/hypr-osd-tablet`, and every
element reads that flag: a panel that was not running when you folded the machine
must not come up believing the wrong thing about it. Elements poll the flag about
once a second and add or remove their `touch` class as it changes, so folding the
cover back folds the layout back too, with nothing to restart.

The switch's name has to match `hyprctl devices` exactly. If yours is called
something else, change the two binds; a machine with no switch at all falls back
to the keyboard: no typing keyboard attached means a tablet. That fallback is
also what makes a session that *started* folded back correct, since a switch that
never moves never fires a bind.

The bar is also what starts and stops the on-screen keyboard, because it is the
one element that is always running. `hypr-osd-bar tablet status` answers the
question directly:

```sh
hypr-osd-bar tablet on      # what the switch's "on" bind runs
hypr-osd-bar tablet off     # ... and the "off" one
hypr-osd-bar tablet status  # touch: tablet / touch: laptop, and what the keyboard is
```

`enabled = false` in [`touch.conf`](#touchconf) switches the whole behaviour off
while leaving the touchscreen working as a pointer, which is the setting to reach
for if a touch-sensitive laptop panel is being bumped by a sleeve.

### The tap catcher

A hover popup closes when the pointer leaves it. A finger does not leave: it
lifts, and the pointer GDK leaves behind stays exactly where the finger was — so a
tapped panel would stay up until something else happened. The way out is a second
layer-shell surface, a **tap catcher**: a full-screen surface on the overlay
layer, drawn as four dimmed bands around a hole that is exactly the card. A tap
on a band closes the card (and is swallowed, so it does not also activate whatever
was behind it); a tap on the card reaches the card, because the hole is
transparent and the compositor's `ignore_alpha` rule lets it through.

It exists only while a finger-opened panel is up. The stack is the same palette as
everything else, the dim is `box.tap-catcher` in `base.css`, and it never takes
the keyboard — a card with a field keeps focus, which is what keeps the
on-screen keyboard usable above it. Implementation:
`Osd::tap_catcher(Some(Hole), close)` in `crates/hypr-osd-core/src/osd.rs`.

### The on-screen keyboard

`squeekboard` is the one in the Arch repositories and the one this desktop
supports:

```sh
sudo pacman -S squeekboard
```

The bar starts it when the machine becomes a tablet and stops it when the cover
comes back — deliberately, because squeekboard shows itself for *any* focused
text field, whether or not a real keyboard is in your hands. On a tablet it is
what types into the launcher's search field, the applications panel's filter and
every text row of the settings window; `gtk::Entry` speaks the text-input
protocol that makes it appear, so none of the cards has to ask twice.

`touch.conf`'s `osk = auto` finds squeekboard, then `wvkbd-mobintl`, then `wvkbd`;
`osk = none` turns the keyboard off; anything else is a command line
(`osk = wvkbd-mobintl -H 300`), for an AUR wvkbd. If a tablet has no keyboard
installed at all, the cards that want one say so on stderr instead of silently
doing nothing.

**The lock screen** is the one place the keyboard does not come by itself:
hyprlock is not a text-input client, so an on-screen keyboard has nothing to
appear for, and Hyprland draws nothing above a locked session that is not allowed
to be there. So locking goes through `~/.local/bin/hypr-osd-lock` — which `SUPER
+ L`, hypridle and the session card all run — and on a tablet it starts the locker
and then asks squeekboard to show itself, over the layer rule that permits exactly
that:

```lua
hl.layer_rule({ name = "squeekboard-above-lock", match = { namespace = "^osk$" }, above_lock = 2 })
```

`above_lock = 2` draws the keyboard over the lock screen *and* makes it
interactive, which is the point: its keys are what type the password. It is also
the one deliberate hole in the lock screen's isolation, and it is a trade-off
worth knowing about — the surface that can be touched while locked is the
keyboard, and nothing else. On a laptop the script is just the locker.

### Right click, long press, and what is where

| Mouse | Finger | Where |
| --- | --- | --- |
| right click on a tray icon | long press | the application indicator's own `ContextMenu` |
| right click on an application tile | long press | pin/unpin it in the applications panel |
| right click on the clock | long press | notification centre / do-not-disturb |
| right click on the status pill | long press | the wireless radio on/off |
| right click on the media pill | long press | next track |
| middle click on the status pill | — | microphone mute lives in the system popup's sound row on a touchscreen |
| wheel on the status pill | two-finger swipe up/down | the volume, one step per gesture |
| hover the player pill / status pill / grid button | tap | the island / system popup / applications panel, pinned with a tap catcher |

The bar's pills keep their pointer behaviour unchanged, including the wheel and
the right and middle buttons; the touchscreen is an *addition*, not a
replacement. What a finger cannot do — hover — is the one thing that was
redesigned, and that is the tap catcher.

### Gestures

The touchscreen gestures are **hyprgrass**'s, not Hyprland's: Hyprland's own
`hl.gesture` only knows the trackpad. The plugin is a hyprpm one:

```sh
hyprpm add https://github.com/horriblename/hyprgrass
hyprpm enable hyprgrass
hyprpm reload -n
```

`hyprctl plugins list` names it when it is loaded, and `scripts/install.sh`
reports it when it is not. Without it the guard around the gesture block in
`osd.lua` leaves the block unread and everything else keeps working — the panels
are still reachable by tapping the bar.

| Gesture | Opens |
| --- | --- |
| one finger from the **top** edge, downward | the system popup (wireless, sound, bluetooth, brightness) |
| one finger from the **bottom** edge, upward | the workspace overview — the tablet's "home" |
| one finger from the **left** edge, inward | the applications panel (the bar's left button) |
| one finger from the **right** edge, inward | the island: what is playing, and the notifications |
| two fingers up / down | the volume, one step per gesture |
| three fingers up | the window switcher |
| three fingers down | the session card |
| three fingers sideways | the workspace swipe — but this one is **not registered here**: it is Hyprland's own gesture (in `hyprland.lua`'s hyprgrass block on this desktop), and a second registration of the same pattern is reported by the plugin as “Gesture will be overshadowed by a previous gesture”. `osd.lua` only registers gestures that open this collection's panels |
| three-finger tap | the window switcher, without the travel |

`edge_margin` (in the plugin's options, 40px by default in `osd.lua`) is how far
in from an edge a swipe may start and still count as an edge swipe, and
`long_press_delay` is the long press behind the right-click table above. Everything
runs the same verb a key would, so the card a gesture opens is the card a key
opens — and a gesture can be replaced by a keybind with no other change.

The workspace swipe is the one touchscreen gesture that is *not* in `osd.lua`: it
is Hyprland's own (`hyprgrass.gesture` with `action = "workspace"`, usually in
`hyprland.lua` already). Registering the same pattern in both files is what makes
the plugin log `Gesture will be overshadowed by a previous gesture`; only the
first registration is used, so there is nothing to gain by repeating it. If your
`hyprland.lua` has no touchscreen workspace gesture, add the four lines shown at
the end of `osd.lua`'s gesture block.

Two details of this build are worth knowing if you edit the block. A hyprgrass
*gesture* action may be a string (`"workspace"`) or a Lua function, but a
dispatcher (`hl.dsp.exec_cmd(...)`) only works in a **bind**, so commands are
written as `hyprgrass.bind`; and the plugin clears and re-adds every gesture on
every config reload, which is why the block is guarded.

### What a finger cannot do (yet)

- **Rotate the panel.** Hyprland does not auto-rotate, and a rotation needs both
  the monitor and the touch transform set (a rotated output does *not* rotate its
  touch input). The accelerometer is there; nothing reads it yet. If you add it,
  set `input:touchdevice:transform` alongside the monitor's, or the finger lands
  in the wrong place.
- **Use the pen's barrel button as a right click.** Hyprland forwards stylus
  buttons to tablet-v2 clients only; the pen is otherwise a pointer, so it taps
  and drags, and the long press is the way to a context menu with it too.
- **Type on the lock screen without squeekboard.** See above — the script and the
  layer rule are the whole mechanism, and they need squeekboard installed.
- **See tooltips.** They are still there for the pointer; on a touchscreen the
  meaning moves to a visible label wherever it matters (the system popup's rows),
  and the rest is left to the card itself.

### `touch.conf`

One file, not one per element: the questions it answers — is this machine a
tablet, is there a keyboard on screen — are about the machine.

| Key | Default | |
| --- | --- | --- |
| `enabled` | `true` | the whole touch behaviour: tap-open panels, long presses, the `touch` layout and the on-screen keyboard. Off, the touchscreen still clicks and drags like a pointer |
| `osk` | `auto` | `auto` (squeekboard, then wvkbd), `none`, or a command line (`wvkbd-mobintl -H 300`) |
| `osk_height` | `260` | how tall wvkbd is asked to be, in pixels; squeekboard sizes itself |
| `long_press_ms` | `500` | how long a finger must rest for a long press |

The file lives at `~/.config/hypr-osd/touch.conf` and is read once, at start-up,
like every element's config — a change is a restart of the elements that read it
(`scripts/install.sh` restarts the ones that are running).

## Configure

One file per element under `~/.config/hypr-osd/`, every key optional (the
defaults are in the comments of the installed templates). `theme.css` is not a
element config: it is the palette, and it is [its own section](#look-and-feel).
`touch.conf` is not an element's either — it is the machine's, and it is
documented in [Touch](#touchconf).

**`bar.conf`**

| Key | Default | |
| --- | --- | --- |
| `height` | `40` | the bar's height in pixels |
| `margin_top` | `0` | how far below the top edge it floats - 0 is what welds its pills to the edge |
| `margin_x` | `0` | the inset at each side - 0 welds the outer tabs to the left and right screen edges |
| `exclusive` | `true` | reserve the strip with the compositor, so windows start below it |
| `output` | *(empty)* | the connector to put the bar on, and *only* that one; empty = a bar on every monitor |
| `show_apps`, `show_workspaces`, `show_title`, `show_clock`, `show_media`, `show_status`, `show_tray`, `show_minimised`, `show_power` | all `true` | **what is on the bar**, one switch each in the settings window's Bar page: a part switched off is not built, not read and not drawn. The two handles go with their pills — hiding the player takes the island popup's trigger with it, hiding the status pill takes the system popup's (and with `show_tray = false` the bar does not claim the tray's D-Bus names at all) |
| `workspaces` | `5` | how many numbered workspaces the row keeps room for |
| `title_width` | `320` | how much room the window title may take before it is ellipsised |
| `media_width` | `340` | the same for the player in the middle of the bar |
| `clock_format` | `%H:%M` | the clock's `strftime` format (the top line) |
| `date_format` | `%a, %b %d` | the small line under the time; empty gives a one-line clock |
| `battery`, `adapter` | `BAT1`, `ADP1` | the battery to watch, and the mains supply that says whether it is charging |
| `network_interface` | `wlan0` | the wireless interface to read through `iw` |
| `tray_icon_size` | `18` | tray icon size in pixels (the application indicators and the put-away windows alike) |
| `tick_ms` | `1000` | the heartbeat; the clock's resolution |
| `volume_every_ms`, `network_every_ms`, `battery_every_ms` | `1000`, `5000`, `30000` | how often the slow parts of the status pill are re-read |
| `resync_every_ms` | `5000` | how often Hyprland and MPRIS are re-read even with no event |
| `volume_command` | `hypr-osd-volume` | what the volume pill's click and wheel run |
| `session_command` | `hypr-osd-session` | what the power button runs |
| `island_command` | `hypr-osd-island` | what the player's hover runs |
| `stats_command` | `hypr-osd-stats` | what the status pill's hover and click run |
| `apps_command` | `hypr-osd-apps` | what the grid button at the bar's **left** end runs (the pointer arriving on it asks the panel to `open`) |
| `notifications_command` | `swaync-client` | what the clock's clicks run |

The bar is event driven: Hyprland's own event socket drives the workspace row and
the title, and `playerctl --follow` drives the media pill. The `*_every_ms` keys
are the intervals for the things that have no event to listen to (a battery
discharging has nothing to say about itself), and `resync_every_ms` is the safety
net for an event socket that quietly died.

**`island.conf`**

| Key | Default | |
| --- | --- | --- |
| `bar_height`, `bar_margin_top` | `40`, `0` | where the bar is — these have to match `bar.conf` |
| `gap` | `6` | the distance between the bar's bottom edge and the panel |
| `hot_width` | `280` | the fallback hover strip in the bar's middle, for an `open` typed by hand without the pill's rectangle |
| `left_width`, `right_width` | `252`, `178` | the panel's two columns |
| `open_delay_ms` | `120` | how long the pointer has to rest on the player before the panel opens |
| `close_delay_ms` | `300` | how long it may be away before the panel closes |
| `poll_ms` | `50` | how often the pointer is sampled, while the panel matters |
| `tick_ms` | `1000` | the clock's resolution |
| `media_every_ms` | `700` | how often the progress bar is refreshed while the panel is up |
| `notifications_command` | `swaync-client` | what the hub's buttons run |

The panel is told where the bar is through `bar_height`, `bar_margin_top` and
`gap`, which is why those three have to agree with `bar.conf`. The pill's hot zone
is not derived: the player is as wide as its track, so the bar measures the pill
and sends the rectangle with `open` (`hot_width` is only the fallback for a
hand-typed one). A transparent, monitor-sized canvas (`Placement::Canvas`) draws
one rounded shape from that pill into the panel and back, while the bar retracts
its other pills to make room. The same morph connects the applications and system
popups to their own opening buttons.

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
pkill -f hypr-osd-bar && hypr-osd-bar &           # the bar, too
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

**`overview.conf`**

| Key | Default | |
| --- | --- | --- |
| `tile_min_height` | `84` | the smallest a tile may become, however full the desktop is |
| `tile_max_height` | `300` | the largest: what a single window gets, and the ceiling for any tile |
| `gap` | `18` | space between tiles, and between rows of them |
| `icon_size` | `64` | the icon in a tile that has no picture |
| `thumbnails` | `true` | `false` skips the captures — the card becomes a workspace switcher drawn with icons |
| `idle_close_ms` | `60000` | close after this long with no key and no pointer movement; `0` never closes by itself |

**`launcher.conf`**

| Key | Default | |
| --- | --- | --- |
| `width` | `640` | the card's width in pixels, and so how much of a file name is readable |
| `max_results` | `7` | how many rows the list holds — and how tall the card is, because the list reserves room for all of them |
| `icon_size` | `22` | application (and file type) icon size in pixels |
| `files` | `true` | `false` searches applications only, and touches no directory |
| `debounce_ms` | `120` | how long a keystroke waits before the file walk starts; applications answer immediately |
| `idle_close_ms` | `60000` | close after this long with no key and no pointer movement; `0` never closes by itself |

Applications are read from the freedesktop locations once, at start-up — which is
why the element is autostarted rather than left to be started by the first key
press: the scan is a few hundred small files, and paying for it while the card is
supposed to appear would be visible. `hypr-osd-launcher refresh` re-reads them
without a restart. Icons go through GTK's own icon theme, from the entry's `Icon=`
value; an application the theme has nothing for gets its first letter, the same
stand-in the switcher uses.

**`apps.conf`**

| Key | Default | |
| --- | --- | --- |
| `width` | `760` | the panel's width in pixels |
| `sidebar_width` | `168` | how much of it the drawer list takes; the tile pane gets the rest |
| `grid_height` | `402` | the height of the tile pane — and so the height the card settles at, whichever drawer you are on. Make it a whole number of rows (`rows * tile_height + (rows - 1) * tile_gap`), or the last row sits sliced in half |
| `tile_width`, `tile_height` | `100`, `96` | one tile, and how much of a name fits under its icon (two lines, then ellipsised) |
| `icon_size` | `40` | application icon size in pixels |
| `tile_gap` | `6` | space between tiles, and between rows of them |
| `gap` | `6` | the distance between the bar's bottom edge and the panel's top edge |
| `bar_height`, `bar_margin_top`, `bar_margin_x` | `40`, `0`, `0` | where the bar is — these have to match `bar.conf` |
| `hot_width` | `60` | the fallback hover strip at the bar's left end, for an `open` typed by hand without the button's rectangle |
| `open_delay_ms`, `close_delay_ms` | `120`, `300` | how long the pointer rests on the button before the panel opens, and how long it may be away before it closes |
| `poll_ms` | `50` | how often the pointer is sampled, while the panel matters |
| `idle_close_ms` | `60000` | close after this long with no key and no pointer movement; `0` never closes by itself |

The four hover keys are the same mechanism (and the same defaults) as the island's
and the system popup's: the bar's button only *asks*, and the element's own pointer
poll decides when the dwell is over and when the pointer has left. `hot_width` is
only for `hypr-osd-apps open` typed into a terminal — the bar sends the button's
measured rectangle instead.

`width`, `sidebar_width` and `tile_width` are what decide how many tiles fit in a
row (5 by default), and the element's arrow keys and the grid both count them the
same way, so "down" always moves down a row. The pinned applications are **not** a
setting: they are the state file `~/.config/hypr-osd/apps-pinned`, written by a
right click on a tile — delete it to unpin everything.

The scan itself is not duplicated: the launcher and the panel both read the
applications through `hypr-osd-core`'s `apps` module (the same `.desktop` parser,
the same category map and the same ranking), so an application is classified and
scored identically in both cards.

**`bluetooth.conf`**

| Key | Default | |
| --- | --- | --- |
| `width` | `640` | the card's width in pixels; every row is capped to fit it |
| `list_height` | `380` | the height of the scrolling device list, below the header and the filter |
| `max_devices` | `24` | how many devices details are read for (one `bluetoothctl info` each); the rest are still listed from the cheap answer |
| `devices_every_ms` | `5000` | how often the controller and its devices are re-read *while the card is open* (it costs one `bluetoothctl` per device, and a closed card asks nothing) |
| `scan_seconds` | `12` | how long the Scan chip looks; `bluetoothctl --timeout` exits by itself, and that exit ends the scan |
| `idle_close_ms` | `120000` | close after this long with no key and no pointer movement; `0` never closes by itself |

The bluetooth parsing, the pair/trust/connect chain and the scan live in
`hypr-osd-core`'s `bluetooth` module, so the manager and the system popup's
tile can never disagree about what a device is doing.

**`stats.conf`**

| Key | Default | |
| --- | --- | --- |
| `left_width`, `right_width` | `260`, `240` | the panel's two columns: the load on the left, the controls on the right. Both are *floors* — a widget the theme will not let be narrower (a slider's trough, a progress bar) wins over them |
| `bar_height`, `bar_margin_top`, `bar_margin_x` | `40`, `0`, `0` | where the bar is — these have to match `bar.conf` |
| `gap` | `6` | the distance between the bar's bottom edge and the panel |
| `hot_width` | `240` | the fallback hover strip at the bar's right end, for an `open` typed by hand without the pill's rectangle |
| `open_delay_ms`, `close_delay_ms` | `120`, `300` | how long the pointer rests before it opens, and how long it may be away before it closes |
| `poll_ms` | `50` | how often the pointer is sampled, while the panel matters |
| `tick_ms`, `slow_every_ms` | `1000`, `5000` | the heartbeat, and how often the readings that cost a command are re-read |
| `devices_every_ms` | `10000` | how often the bluetooth device list is re-read *while the panel is open* (it costs one `bluetoothctl` per device) |
| `max_devices`, `scan_seconds` | `5`, `12` | how many device rows the list shows, and how long Scan looks |
| `updates_command` | `checkupdates` | what prints the pending-update list; empty turns the updates tile off |
| `network_interface` | `wlan0` | the interface the NETWORK row reports |
| `terminal_command` | `kitty` | where btop, nmtui and pacman are opened |
| `settings_command` | `hypr-osd-settings` | what the header's **gear** runs ([Settings](#settings)) |
| `bluetooth_command` | `hypr-osd-bluetooth` | what the bluetooth tile's **Manage** chip runs ([Bluetooth manager](#bluetooth-manager)) |

**`settings.conf`**

| Key | Default | |
| --- | --- | --- |
| `width` | `760` | the window's width in pixels |
| `pane_height` | `380` | the height of the scrolling pane — and so the height the card settles at, whichever page is on it |
| `sidebar_width` | `190` | the list of elements down the side |
| `idle_close_ms` | `120000` | close after this long with no key and no pointer movement; `0` never closes by itself |
| `bin_dir` | `~/.local/bin` | where the element binaries live, for the Restart chip |
| `terminal_command` | `kitty` | the terminal a button that needs one opens — the Login screen page's Install runs `sudo` there |

The settings window's own page is in its catalogue too, so its numbers are
tweakable from inside it — with the same caveat as every element: change one and
the window has to be restarted for it to show.

**`desktop.conf`** — the Hyprland tab, and a different kind of file: its keys are
Hyprland **option paths** (`general:gaps_out = 20`, `input:kb_layout = ch`), i.e.
the names `hyprctl getoption` knows — and, for the Display page, a field qualified
by the output (`monitor:eDP-1:scale = 2`). The window edits it like any element's
file, but what it does with it is another thing entirely: it regenerates
`~/.config/hypr/hypr-osd-desktop.lua` from it and hands that to the compositor, so
a change there is *live* instead of waiting for a restart — see
[Settings](#settings).

## Lock screen

**hyprlock** draws it (`sudo pacman -S hyprlock`), from
`~/.config/hypr/hyprlock.conf` — which is **generated**, not shipped:
[`configs/hyprlock.conf`](configs/hyprlock.conf) is kept as the reference copy
with the reasoning behind each measurement, and `hypr-osd-lock-vars` writes the
real file from `~/.config/hypr-osd/lock.conf` and the palette. A hand-written
file from before that is backed up once (`hyprlock.conf.bak-handwritten`).

hyprlock cannot read CSS, and nothing here should be kept in step by hand: the
palette is **generated** out of `~/.config/hypr-osd/theme.css` — the same
`@define-color` values every element reads, converted to hyprlock's `rgb()` /
`rgba()` dialect, plus the accent at the opacities the panel, the field's fill and
its outline are cut from — together with every number, which is a row on the
settings window's **Lock screen** page ([Appearance → Lock
screen](#settings)). One file is written, with the values in place:

```ini
general { hide_cursor = true … }
background { path = /home/lars/Bilder/Wallpapers/1022522.jpg … }
shape { size = 800, 670 … }          # the glass
label { text = cmd[update:1000] … }  # the clock, the date, the lock mark, the hint
input-field { … }                    # the field
```

so a retint reaches this screen with no hyprlang to edit, and no colour on it can
be out of step with the bar. Nothing has to be restarted either — hyprlock reads
its config at every lock, which is what makes the page live as you change a value:

```sh
hypr-osd-lock-vars            # write the whole config now
hypr-osd-lock-vars --print    # ...or just look at what it would be
```

The installer runs it, the settings window runs it when it opens and on every
change on that page, and after editing `theme.css` by hand it is the one command
to run. `crates/hypr-osd-core/src/lock.rs` is the generator; a test changes every
row of that page in turn and requires the generated file to change with it, so a
switch cannot sit on the page and change nothing.

Every key of `~/.config/hypr-osd/lock.conf` is a row on that page, and every
option hyprlock 0.9 has for the widgets this screen uses is a key. The groups:

| keys | what |
| --- | --- |
| `wallpaper`, `monitor` | the picture behind the lock screen (empty follows hyprpaper's, `screenshot` re-takes the live desktop), and the output it draws on |
| `blur_passes`, `blur_size`, `brightness`, `contrast`, `vibrancy`, `vibrancy_darkness`, `noise`, `reload_time`, `crossfade_time` | the whole `background` treatment |
| `hide_cursor`, `ignore_empty_input`, `immediate_render`, `text_trim`, `fractional_scaling`, `screencopy_mode`, `fail_timeout`, `animations` | `general` and `animations` |
| `pam_enabled`, `pam_module`, `fingerprint_enabled`, `fingerprint_ready_message`, `fingerprint_present_message`, `fingerprint_retry_delay` | `auth` |
| `panel_*` (width, height, x, y, rounding, border, opacity, shadow size/passes/boost/opacity) | the sheet of dark glass |
| `clock_*` (font, size, x, y, format, accent colon), `date_*` (format, size, x, y, uppercase) | the clock and the date |
| `show_rule`, `rule_*` | the accent rule between them |
| `show_lock_icon`, `lock_icon`, `lock_size`, `lock_y` | the lock mark |
| `field_*` (font, width, height, x, y, rounding, tint, outline, the two gradient opacities), `dots_*` (size, spacing, centring, rounding, text), `placeholder_text`, `check_text`, `fail_text`, `fade_on_empty`, `fade_timeout`, `hide_input`, `swap_font_color`, `invert_numlock` | the password field |
| `hint_text`, `hint_size`, `hint_x`, `hint_y` | the resting hint under the field |

The rows are drawn under **headings** — Background, Behaviour, Authentication,
Glass panel, Clock and date, Accent rule, Lock mark, Password field, Hint — so the
page reads like the screen it describes instead of one long list. Its **Check**
button is the other half of that: it asks hyprlock's own parser to read the
generated file against a display that does not exist (so it cannot lock anything)
and reports the number of bad options. It is a button rather than a reading
because that check ends in an abort — hyprlock cannot connect — and doing it on
every page render would fill the journal with crash entries.

What it draws, over **your desktop wallpaper** — the lock screen points straight at
the image the desktop uses, rather than at a screenshot of the desktop, so it
shows the background you chose, never the shape of what you had open, and the
backdrop holds still while everything else sleeps:

- a sheet of dark glass — one `shape`, `panel_width` × `panel_height`
  (`Glass width` / `Glass height` on the page, 800×670 unset), hairline accent
  border, deep shadow — so the type sits on a known contrast instead of on the
  wallpaper;
- the **clock at 170px**, twice the bar's own type, with an accent colon and a
  soft shadow; the date in caps underneath, then a short accent rule;
- the password field **only while there is something to type into**: a tall
  440×78 box, glass tinted towards the accent, a gradient border that carries
  the state (accent while PAM checks, red when the password is wrong, amber while
  capslock is on).

The background `path` is the `path` of
[`hyprpaper.conf`](configs/hyprpaper.conf), read when the lock screen is written,
so the wallpaper and the lock screen's backdrop are one picture and one file
decides it. A picture of its own is the page's **Wallpaper** row (with a file
chooser beside it); empty follows hyprpaper, and `screenshot` (the fallback when
there is no hyprpaper config at all) re-takes the live desktop on every lock.
hyprlock draws a still image, so a *video* wallpaper needs a frame of it instead —
the comment above that line has the one-liner. Blur, dim, contrast, vibrancy and
grain are that page's other picture rows: at **Blur size** 1 the image is
essentially untouched, and 8–16 is frosted glass.

Switching wallpapers at runtime is one command —
`hyprctl hyprpaper wallpaper ",/path/to/image.jpg"` — and the Wallpaper page's
**Use for → Lock** does the same plus the write, in one click. The lock
screen follows the next time it is written, which is the next time the settings
window opens or changes anything (or `hypr-osd-lock-vars`).

The field is what makes the resting screen quiet: it fades out while the buffer
is empty and back in with the first keystroke, so what you walk up to says
"locked" instead of "type here". A padlock glyph and one faint line carry that.
**Enter on its own does not reveal it** — hyprlock follows the buffer, not the
key, and an empty Enter is deliberately a no-op (`ignore_empty_input`, a switch on
the Lock screen page, so a stray Enter cannot spend one of PAM's three attempts).
Once there is something to submit, Enter submits it, as always.

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

## Wallpaper

**hyprpaper** draws the desktop's background (`sudo pacman -S hyprpaper`), from
`~/.config/hypr/hyprpaper.conf` — generated from
`~/.config/hypr-osd/wallpaper.conf`, the settings window's **Wallpaper** page
([Appearance → Wallpaper](#settings)). Every option hyprpaper 0.8 has is a row:

| key | default | what |
| --- | --- | --- |
| `path` | *(the picture already there)* | the image, or a directory it walks as a slideshow; the row has a **file** button and a **folder** button, and the page opens with the picture itself (or the slideshow's first frame) above the rows |
| `fit_mode` | `cover` | `cover` crops the overflow, `contain` letterboxes, `tile` repeats, `fill` stretches |
| `monitor` | *(empty)* | the output this block is for; empty makes it the fallback for every output |
| `timeout`, `order`, `recursive` | `30`, *(none)*, `false` | the slideshow: seconds per picture, `random` order, descend into subdirectories |
| `splash`, `splash_offset`, `splash_opacity` | `false`, `20`, `0.8` | Hyprland's splash over a freshly loaded wallpaper |
| `ipc` | `true` | let `hyprctl hyprpaper` talk to the daemon — which is how a new picture is applied |

A change writes the config **and** hands the picture to the running daemon
(`hyprctl hyprpaper wallpaper "[mon],path[,fit_mode]"`), so the screen changes as
you pick it. The page opens with the picture itself — cropped to a banner, with
its name and size, or the first frame of a slideshow — and a reading line says
whether hyprpaper is running (the first thing to check when a picture change
seems to do nothing). `splash` and `ipc` are read once at start-up, so those two
need the page's **Restart** button. The page's **Use for** row is the one-click
way to share it:

- **Lock** writes `wallpaper` into `~/.config/hypr-osd/lock.conf` and regenerates
  the lock screen;
- **Login** writes `background` into `~/.config/hypr-osd/login.conf` and re-renders
  the greeter's image;
- **Everywhere** does both — one picture for the desktop, the lock screen and the
  login screen.

The **Follow wallpaper** button on the Lock screen page gives the lock screen back
to hyprpaper (the `wallpaper` row becomes unset again).

The installer writes the template only when hyprpaper is present and the file is
missing; the settings window is what keeps it up to date afterwards. hyprpaper
maps a background layer named `hyprpaper` over every monitor, which is the
cleanest proof it is running: `hyprctl -j layers`.

## Login screen

The login screen is **SDDM**, before any compositor: a Qt greeter that draws the
`hypr-osd` theme from `/usr/share/sddm/themes/hypr-osd/`. Its source lives in
`~/.config/hypr/sddm-theme/` (Main.qml, metadata.desktop, `theme.conf`,
`images/background.jpg`), and the settings window's **Login screen** page
([Appearance → Login screen](#settings)) edits `~/.config/hypr-osd/login.conf` and
regenerates two of those files:

- **`theme.conf`** — the picture, the face, and the eight palette colours the QML
  reads (`bg`, `fg`, `fg_dim`, `fg_faint`, `accent`, `on_accent`, `warn`, `crit`).
  The greeter cannot read CSS, so those colours are written out in hyprlock's
  place too: a colour `login.conf` does not set follows `theme.css` (the page shows
  the live token as `· now …`), and a line in `login.conf` overrides it — the same
  rule as the lock screen, and why a retint of the theme reaches the greeter.
- **`images/background.jpg`** — rendered from the wallpaper (or a picture of your
  own) with `magick`: resize, crop, blur, dim and saturate. The greeter has no
  compositor behind it and no blur of its own, so this happens once, here.

| keys | what |
| --- | --- |
| `background` | the picture behind the login; empty follows hyprpaper's wallpaper. The row has a **file chooser** |
| `render`, `blur`, `dim`, `saturation`, `render_width`, `render_height` | the rendering: whether to do it, how much, and at what size |
| `font` | the face the greeter draws |
| `bg`, `fg`, `fg_dim`, `fg_faint`, `accent`, `on_accent`, `warn`, `crit` | the palette; an unset colour follows `theme.css`, a set one wins |
| `cursor_size` | SDDM's cursor size, for the drop-in `install.sh` writes |

The page opens with the picture the greeter would show and two readings: whether
the installed theme still carries what the page writes, and what the rendered
background came out as (its size). The page's **Render** button writes the theme
and renders the image now; **Install** copies both into `/usr/share/sddm` (it runs
the theme's own `install.sh` through `sudo` in a terminal — `terminal_command` on
the Settings page, `kitty` by default — so the password is typed where the
installer's output can be read); **Preview** opens
`sddm-greeter --test-mode --theme ~/.config/hypr/sddm-theme`, which is how
the theme is looked at without logging out (in test mode the power pills and the
user name are absent, because there is no daemon socket and no last user — not a
bug).

Installing is the one step the window cannot take itself: `/usr/share/sddm/` and
`/etc/sddm.conf.d/` belong to root. The **Install** button does exactly what this
command does,

```sh
sudo ~/.config/hypr/sddm-theme/install.sh
```

which copies the theme, renders the background into it, and writes
`/etc/sddm.conf.d/hypr-osd.conf` (`Current=hypr-osd`, `CursorSize=…`). It takes
effect at the next logout; the greeter is deliberately not restarted, because that
would end the session. `--render-only` rebuilds just the picture.

## How it works

- **The bar is not a card.** It is the same kind of surface — one GTK4
  layer-shell app — but anchored to the *top* layer (not the overlay one),
  stretched across the screen, with its height reserved as an exclusive zone so
  tiled windows start below it, and it is up from the moment its process starts
  until it ends. `Placement::Bar` in `osd.rs` is the whole difference, plus one
  detail: a fullscreen window covers the bar exactly as it covers everything
  else, which is what the top layer means.
- **The bar is the one element with a surface per screen.** A GTK widget has a
  single parent, so there is no one bar that could be shown on two monitors: the
  element hands the shell a *factory* (`Content::PerOutput`) and the shell calls
  it once per output, reconciling its surfaces against GDK's monitor list every
  time that changes. `hypr-osd-bar` keeps one view per connector and paints them
  all from the same values — the workspaces, the track and the volume are
  properties of the session, not of a screen — so two bars cannot disagree, and a
  screen plugged in halfway through is painted immediately instead of waiting for
  the battery's thirty-second tick. The tray is a session-wide D-Bus object with
  one set of icons, so exactly one bar carries it: the screen `output` names, else
  the one that was focused when the bar started.
- **Nothing that runs a command waits for it.** `hypr_osd_core::hardware::launch`
  spawns and hands the exit to GLib; `run` (which waits) is only for tools that
  answer once and leave. Half of what the bar *does* is another element —
  `hypr-osd-stats open eDP-1 1105 5 184 28` unfolds the system popup, the power
  button runs `hypr-osd-session`, the wheel runs `hypr-osd-volume` — and the first
  invocation of an element nobody has started yet *becomes* that element's daemon,
  staying up as long as its card does. Waiting for one therefore means waiting for
  the user to close a panel, with the bar's own main loop parked: the status pill's
  click froze the whole bar, permanently. There is a regression test for it
  (`launching_a_command_does_not_wait_for_it`).
- **The bar asks Hyprland over its own sockets**, not through `hyprctl`: raw
  requests on `.socket.sock`, and the event stream on `.socket2.sock`, so the
  workspace row keeps up with a swipe instead of with a timer. Events arrive in
  bursts and are coalesced into one read, and the read is a GIO stream on the
  main loop, so nothing blocks. The timer that remains is a safety net for a
  socket that quietly died.
- **There is no minimise; there is a hidden workspace.** `SUPER + H` moves the
  focused window onto `special:minimized`, a special workspace that is never
  toggled onto a screen — the mechanism a scratchpad uses, pointed at the tray
  instead. Three details are what make it behave like a minimised window. The
  compositor goes on reporting the window as *mapped*, so the workspace name (not
  `mapped`) is the only thing that tells it apart from a window you can switch to,
  and `windows::list` filters it out on exactly that basis — which is why the
  switcher and the overview never offer a put-away window. Moving a window *onto*
  a special workspace makes Hyprland **show** it (that is what the gesture is for
  on a scratchpad: put it there and look at it) and keeps the moved window
  focused while it is shown, so a minimise is two dispatches and not one — the
  move, and hiding the workspace again, which is what actually takes the window
  off the screen. `specialWorkspace` on the monitor is the field that says
  whether it is on screen; `activeWorkspace` says nothing about it. And the
  workspace the window came from is remembered by the bar, in memory, because the
  compositor keeps the window and not its history.
- **A put-away window can still be the focused one.** On a workspace whose only
  window is the one being put away, hiding it leaves the compositor with nothing
  to hand the focus to, and it goes on considering the hidden window the focused
  one. That is why the bar's title refuses to name a window that is put away:
  nothing on screen may be named there. The state is a dead end for the user's
  typing and nowhere else — the next click, workspace switch or tray icon leaves
  it — and the alternative (focusing another monitor's workspace to clear it)
  would move their attention somewhere they did not ask for.
- **The tray is a D-Bus service the bar owns.** `hypr-osd-bar` claims
  `org.kde.StatusNotifierWatcher`, because without a host application indicators
  are simply invisible, and then renders each registered item: its own pixmap when
  it sends one, otherwise a lookup in the icon theme. Clicks are passed on to the
  item (`Activate`, `SecondaryActivate`, `ContextMenu`). The item's *menu* is the
  one thing the bar does not draw — that is a second protocol
  (`com.canonical.dbusmenu`) and a project of its own — so items that implement
  `ContextMenu` themselves get theirs and the rest do nothing on a right click.
- **One process per element**, started at login by `osd.lua`'s `exec-once`
  (nothing is shown), or lazily by the first key press. A card, not a window:
  `gtk4-layer-shell` puts the surface on the *overlay* layer, so it appears above
  fullscreen windows and is not touched by the tiling layout.
- **It never steals focus — except for the cards that are driven by keys.**
  The layer surface is created with `KeyboardMode::None`, so the window you were
  typing in keeps the keyboard while the card is up; pointer input still works —
  the slider needs it. The switcher, the overview, the launcher, the
  applications panel, the bluetooth manager and the settings window ask for the
  keyboard (`Opts::keyboard`) for as long as their card is on screen, because all
  six are driven by keys that must not also reach the window underneath: an Alt
  release ends a switch, Escape and the arrows cancel an overview, and a launcher,
  the panel, the manager or the settings window has to be *typed into*. All six
  are opened on purpose, which is what makes eating a keystroke acceptable there,
  and all six let go of the keyboard by themselves if they are left alone — the
  switcher when Alt comes back up, the rest after `idle_close_ms`.
- **The transparent frame is click-through.** The surface is the card plus an
  18px ring that lets the card's shadow breathe; `osd.lua`'s `ignore_alpha` layer
  rule makes those transparent pixels pass clicks to whatever is behind them, so
  only the card itself is hit-testable.
- **A card follows the focused monitor — unless you have named a primary one.**
  Each time a card is shown it asks `hyprctl -j activeworkspace` which output is
  focused, and the surface is (re)built for that monitor, so the card appears
  where you are looking. One monitor means this is a no-op. There are two
  deliberate exceptions: the bar is on *every* output (see above) and a popup that
  unfolds from it is told which of the bars it came from, and the **notification
  cards** (volume, media, session) go on the *primary display* named in the
  settings window's Display page — that is what designating one is for. A card you
  *drive* with the keyboard (the switcher, the overview, the launcher, the
  settings window, the applications panel and the bluetooth manager) keeps
  following the focused output, because one that opened on a screen you are not
  using would be unusable.
- **`wpctl` is the single source of truth.** The volume element reads the sink, applies
  the change and reads it back, so the card can never show a number the sink does
  not have — and it can never disagree with the bar's volume pill, which asks the
  same element for its step.
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
- **Cover art** is read from `mpris:artUrl` by `core::art`, which the media card
  and the island's player tile share: a `file://` URL is percent-decoded and
  loaded directly (VLC, and most local players, extract covers there), an
  `http(s)://` URL is fetched with `curl` in the background. Anything else — a
  `blob:` URL, a `data:` URI, which some browser players use — has no file behind
  it, so the caller keeps its own stand-in (the music glyph). While a new cover is
  being fetched the stand-in stays up: showing the previous track's cover would be
  a lie.
- **Auto-hide.** `duration_ms` after the last change the card hides — unless the
  volume slider is being dragged, or (for the cards with buttons) the pointer is
  resting on it, so they stay usable. The keyboard cards instead close after
  `idle_commit_ms` / `idle_close_ms` of silence, which is a *different* timer: not
  "the news is stale", but "nobody is here to press Escape".
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
- **The overview captures a window, not the screen.** `grim -T` asks the
  compositor for one *toplevel's* own buffer, which is what makes the tiles real
  pictures: a window behind another is as complete as the one in front, and a
  window on a workspace that is not on screen has a picture at all (grabbing the
  screen once and cropping per window could do neither). `hyprctl -j clients`
  names each window for it (`stableId`), and the capture is scaled down to the
  pixels the tile can show — the monitor scale cancels, so the factor is just the
  fit between the window and its tile, and a 1x screen is not asked for twice the
  detail it has — then encoded as JPEG before it is read: the card holds textures
  for as long as it is up, and a dozen full-size windows would be a couple of
  hundred megabytes.
- **Rearranging is a gesture, not a re-read.** Dropping a window on a workspace
  moves it with `hl.dsp.window.move({ workspace = …, follow = false })` — the
  `follow` is what keeps the desktop where it is — and then the card *corrects its
  own picture*: one window changed workspace, the thumbnails it already holds are
  still what those windows look like, so it re-plans and re-renders against the
  pictures it has instead of paying a `grim` per window. A press that never moves
  is a click, which is why the tile's `GtkGestureDrag` claims the press (a
  `GtkButton`'s own gesture claims it in the bubble phase, and a gesture a level up
  would never hear the motion after it).
- **The overview sizes itself to fit.** Tile height is solved, not guessed: a
  tile's width follows from the window's shape, so "does it fit the screen" is a
  question about one number, and the largest number that fits is the answer — no
  scrolling, because the whole point is seeing everything at once. The
  arithmetic is a pure function in `layout.rs`, which is also where its tests
  are. The search is a bisection (the answer is monotone in the tile height), so
  opening the card costs eight layout passes rather than two hundred.
- **Stopping a daemon stops its helpers.** Follower children are not killed by
  their parent dying, so `run` turns SIGTERM/SIGINT into a clean shutdown and
  `Osd::on_shutdown` gives each element the chance to stop what it started.
  `pkill -x hypr-osd-media` therefore leaves no orphaned `playerctl` behind.

## Adding an element

1. `crates/hypr-osd-<name>/`, a `Cargo.toml` with a `[[bin]]` whose name matches
   the crate, and `src/main.rs` + `src/<name>.css`.
2. Use `hypr_osd_core::run(…)`: give it `Opts` (`app_id` `com.schells2.osd.<name>`,
   namespace `hypr-osd`, width, and — if it is not a bottom-anchored card —
   `placement` and `keyboard`), a `Build` closure that returns the card's content
   widget, and a `Handle` closure that implements the verbs and shows the card
   (`osd.reveal(duration)` for a notification, `osd.show()` for a card that stays
   up until something dismisses it).
3. An element that reacts to something happening rather than to a key press starts
   a follower in `Build` (`hypr_osd_core::follow`) and stops it in
   `osd.on_shutdown(…)` — that is the whole of the media card's wiring. For a
   command that answers *once* with bytes, `hypr_osd_core::output::read` is the
   same idea with an end to it; that is how the overview captures windows.
4. Draw from what the core already knows before inventing it again: the window and
   workspace list (`windows`), an application's icon (`icons`), a tile's stand-in
   letter and a label that cannot widen its card (`text`), what is playing
   (`mpris`), Hyprland's sockets and event stream (`hypripc`), and the one-file
   flags two elements use to talk to each other (`state`).
5. Keep the `hypr-osd` namespace prefix: the layer rule in `osd.lua` matches it,
   and that rule is what makes the transparent frame click-through.
6. Style it from the tokens (`@accent`, `@card_top`, …) and the shared classes
   (`box.card`, `box.bar`, `box.tile`, `label.chip`, `progressbar`), and add only
   what is special about that element. A new *pill in the bar* is not an element at
   all: it is a `Pill` in `crates/hypr-osd-bar/src/view.rs` plus a `render_*`
   method, a class in `bar.css`, and a line in the bar's tick.
7. Add the binary to `ELEMENTS` in `scripts/install.sh` and its keybinding/
   autostart to `scripts/hyprland/osd.lua`.
8. Give it a page in the settings window: a `Section` in `hypr-osd-core`'s
   `settings::OSD` (a test holds that every element has one, so adding an element
   without a page fails the suite). A compositor option belongs in
   `settings::DESKTOP` instead — one line per Hyprland option path, with the kind
   `hyprctl -j getoption` reports for it — and a *display* field (one the Display
   page repeats for every connected output) goes in `settings::MONITOR_SETTINGS`,
   with a `Kind::Monitor` list for a value the monitor itself names.
9. `cargo fmt && cargo clippy --all-targets && cargo test`.

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
| Two bars, one on top of the other | waybar is still in the autostart, or was started by hand: comment the line out (the installer does it) and `pkill -x waybar`. |
| The bar is there but a pill is missing | That is how the bar reports “nothing to say”: no battery on the machine, no wireless interface, nothing playing, no tray items. `hypr-osd-bar status` prints what each one reads and why. |
| The bar is not there at all | `pgrep -f hypr-osd-bar`, then run it in a terminal and read stderr — a missing `gtk4-layer-shell` means it cannot create the surface. `hyprctl -j layers` shows the surface (namespace `hypr-osd`, top layer) when it exists. |
| No bar on one of the monitors | With `output` set in `bar.conf`, only that connector gets one — clear it to put a bar on every screen. Otherwise `hypr-osd-bar status` prints the screens it is on (and which of them carries the tray). A monitor GDK does not know about — a nested session, a headless output — cannot have a surface. |
| A tray icon is missing | The indicator registered with the watcher before the bar owned it — indicators register once, when they start, so restart the application (or log out and in). `busctl --user get-property org.kde.StatusNotifierWatcher /StatusNotifierWatcher org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems` lists what the bar knows about. |
| A right click on a tray icon does nothing | The item has no `ContextMenu` of its own, and the bar does not draw `dbusmenu` menus — see [How it works](#how-it-works). The left click is the one that works everywhere. |
| A put-away window is missing from `ALT + TAB` | That is what putting it away means: the switcher and the overview offer the windows you can switch to, and the tray is where a put-away window is offered instead. `hypr-osd-bar minimised` names them. |
| A put-away window came back to the wrong workspace | It is on the workspace you were on, because the bar was restarted or replaced in between and the workspace it came from went with the old process: the icon's tooltip said so before you clicked it. Click it on the workspace it belongs on, or move it there. |
| The island popup never opens | Is it running (`pgrep -f hypr-osd-island`)? Does `bar_height`/`bar_margin_top` match `bar.conf` (the panel's hot zone is the pill the bar measured and sent, so `hot_width` is only the fallback for a hand-typed `open`)? `hypr-osd-island status` prints the geometry it works with. |
| The island opens or closes too eagerly | `open_delay_ms` and `close_delay_ms` in `island.conf`: the first is how long the pointer has to rest on the player pill, the second how long it may be away before the panel closes. |
| The clock pill stays lit with no panel behind it | A stale “panel is open” flag — the island writes it on show/hide, so a `kill` leaves it behind. It is cleared at start-up: `pkill -f hypr-osd-island; hypr-osd-island &`. |
| The bar's clock says the wrong time | It ticks once a second by default; `tick_ms` in `bar.conf` is the resolution (and the clock's `strftime` format is `clock_format`). |
| No card, but the volume changes | The daemon died — check its stderr, and `pgrep -x hypr-osd-volume`. On a session without `gtk4-layer-shell` it cannot create the surface at all. |
| The volume moves two steps per press | `hyprland.lua` still binds the volume keys to `wpctl`; re-run with `--take-over-keys`. |
| A tap on the bar opens nothing, or the panel opens and shuts at once | The machine is not being read as a tablet, so the tap is being treated as a hover with no pointer to follow. Check `hypr-osd-bar tablet status`; if it says `laptop` with the cover off, the switch bind's name does not match `hyprctl devices` — edit the two `switch:on/off:` binds in `osd.lua` (see [Touch](#tablet-mode-and-what-it-means)). |
| The bar is still the laptop size with the cover folded back | Same cause as above — the `touch` layout follows the tablet flag, and the flag only moves when the switch bind fires or the keyboard disappears. `hypr-osd-bar tablet on` sets it by hand for a test. |
| The screen is dimmed around a card and tapping does nothing | That is the tap catcher of a panel that is still up: tap outside it, or run the panel's `hide` verb (`hypr-osd-apps hide`, `hypr-osd-island close`, `hypr-osd-stats close`). `hyprctl layers` shows the two surfaces. It only exists for panels opened by a finger, so a stuck one means the panel never closed. |
| No on-screen keyboard on a tablet | Is squeekboard installed (`pacman -Q squeekboard`)? Is the machine reading as a tablet (`hypr-osd-bar tablet status`)? Is GNOME's keyboard switch on (`gsettings get org.gnome.desktop.a11y.applications screen-keyboard-enabled`)? The bar starts the daemon, so `pgrep -x squeekboard` while folded back is the second question. |
| A gesture does nothing | Is hyprgrass loaded (`hyprctl plugins list`)? Gestures are cleared and re-added on every config reload, and they never appear in `hyprctl binds` — so a missing plugin looks exactly like a gesture that was never registered. |
| `hyprctl configerrors` says `Gesture will be overshadowed by a previous gesture` | The same gesture pattern is registered twice. hyprgrass keeps the first registration and reports every later one, which is not harmful but is a config error on every reload. The usual pair is the touchscreen workspace swipe: `hyprland.lua`'s own hyprgrass block already registers it, so `osd.lua` must not (see [Gestures](#gestures)). Delete the duplicate `hl.plugin.hyprgrass.gesture(...)` and `hyprctl reload`. |
| The on-screen keyboard is missing over the lock screen | squeekboard must be running (the bar starts it on a tablet) and the layer rule `squeekboard-above-lock` must match its namespace — `hyprctl layers` while locked shows it (it is `osk`), and `hypr-osd-lock` is what force-shows it. Without the rule Hyprland draws nothing above a locked session at all. |
| A locked tablet cannot be unlocked | It has no hardware keyboard and the on-screen keyboard did not come up: see the row above, then check that locking goes through `hypr-osd-lock` rather than `hyprlock` directly (`~/.config/hypr/hypridle.conf`'s `lock_cmd`, and `SUPER + L` in `osd.lua`). |
| The touch position is wrong after rotating the panel | Rotating the monitor does not rotate its touch input. Set the same number in `input:touchdevice:transform` (or `hl.device({ name = "ipts-045e:099f-touchscreen", transform = N })`) and in the monitor's transform. |
| `wpctl … failed` on stderr | PipeWire/WirePlumber trouble; `wpctl status` should list a default sink. |
| Card in the wrong place | `bottom_margin` / `width` in the element's config. |
| Card on the wrong monitor | It follows the focused output through `hyprctl`; if `hyprctl` is unavailable it stays on the monitor it was built for. A volume/media/session card that keeps appearing on one screen is most likely the Display page's `primary` key — `hypr-osd-settings keys display` shows it, and `unset` puts it back on `Follow focus`. |
| No media card appears | Is the daemon running (`pgrep -x hypr-osd-media`)? Does the player speak MPRIS (`playerctl -l`)? A track that is merely loaded, never played, does not trigger a card unless `only_when_playing = false`. |
| Media card shows no cover | The player reported none, or reported it as a `blob:`/`data:` URL — only `file://` and `http(s)://` artwork can be read. `playerctl metadata | grep -i arturl` shows what it sends. |
| “Lock” is greyed out | No screen locker is installed — `hypr-osd-session status` names the candidates (`hyprlock`, `swaylock`, `gtklock`). The row re-checks every time the card is shown, so installing one is enough. |
| Nothing locks when you walk away | That is hypridle's job, and it is not installed: `sudo pacman -S hypridle`, then re-run `install.sh`. `pgrep -af hypridle` should show it running. |
| The power button doesn't show the session card | systemd-logind is handling the key (`HandlePowerKey=`); see the note in [How it works](#how-it-works) and re-run `install.sh` to see the current value. |
| The session card won't go away | Press `SUPER + SHIFT + L` again, or set `duration_ms` to something shorter in `session.conf` (`0` means “stay until dismissed”, which is also what a card with no way out looks like). |
| The overview shows icons instead of pictures | No `grim`, or a compositor that cannot hand over a single window (`hypr-osd-overview status` says which windows have a capture id; a *nested* Hyprland cannot capture a toplevel, a real one can). `thumbnails = false` makes the icon tiles the deliberate look. |
| The overview won't go away | It takes the keyboard while it is up: Escape, a click on the card where there is no tile, or the key again. `hypr-osd-overview close` works from anywhere, and after `idle_close_ms` (a minute by default) it closes itself — which is what gets rid of it after a session lock. |
| A card is on screen but nothing responds to keys | A keyboard card is up (switcher, overview, launcher, applications panel, bluetooth manager or settings) and something else grabbed the keyboard first — usually a session lock. Wait for `idle_close_ms`, or `hypr-osd-overview close` / `hypr-osd-switcher cancel` / `hypr-osd-launcher hide` / `hypr-osd-apps hide` / `hypr-osd-bluetooth hide`. |
| `SUPER + SPACE` opens nothing | Is the launcher running (`pgrep -f hypr-osd-launcher`)? `hyprctl -j layers` shows the surface (namespace `hypr-osd`, overlay layer) when it is up, and `hypr-osd-launcher status` says `state open` or `closed`. |
| Two search bars open at once | Both launchers are bound to `SUPER + SPACE`: `launchpad.lua` is still loaded. Re-run `./scripts/install.sh` (it comments the `require("launchpad")` line out), or bind one of them elsewhere. |
| The launcher's results are not what I expected | `hypr-osd-launcher search <text>` prints the ranking it decided on, with the desktop file each application came from. Keywords and categories count as matches, which is why an application can appear for a word that is not in its name. |
| The launcher lists applications but no files | Do the directories exist? `hypr-osd-launcher status` prints the file roots it found (`Documents`, `Downloads`, … — a root that is not a directory is skipped). `files = false` in `launcher.conf` turns the file half off on purpose. |
| The launcher won't go away | It takes the keyboard while it is up: Escape, or `SUPER + SPACE` again. `hypr-osd-launcher hide` works from anywhere, and after `idle_close_ms` (a minute by default) it closes itself — which is what gets rid of it after a session lock. |
| The bar has no grid button at its left end | The running bar is older than the panel: re-run `./scripts/install.sh` — it appends `apps_command` to an existing `bar.conf` (the key the button needs) and restarts the bar it finds running. |
| The grid button does nothing | `apps_command` in `bar.conf` does not resolve: is the element running (`pgrep -f hypr-osd-apps`), and does that command work in a terminal? The built-in default is the bare program name, which needs `~/.local/bin` on the bar's `PATH` — that is why the installer writes an absolute path. `hypr-osd-apps status` prints the two hover zones: if the `pill` rectangle is not where the button is, the panel will never see the pointer arrive. |
| The applications panel opens and closes on its own | The element thinks the pointer has left it. `hypr-osd-apps status` prints the two zones it reasons with: the `panel` rectangle has to be where the panel actually is, so `bar_height`, `bar_margin_top`, `bar_margin_x` and `gap` in `apps.conf` have to match `bar.conf`, and the `pill` rectangle has to be where the button is (the bar sends it; a `(fallback strip)` means the `open` came without one). |
| The button stays lit with no panel behind it | A stale “panel is open” flag, the same one the island's clock can leave behind: `hypr-osd-apps hide` clears it, and the element clears it at start-up. |
| The applications panel won't go away | Move the pointer off it — that is how it normally ends — or press `Escape`, or `hypr-osd-apps hide` from anywhere. After `idle_close_ms` (a minute by default) it closes itself, which is what gets rid of it after a session lock. `hypr-osd-apps status` says `open, pinned` when a `show`/`toggle` (not the bar) put it up, and then only a verb closes it. |
| Applications are in the wrong drawers | The drawer comes from the entry's `Categories=` (and, for entries that claim nothing, from its name and keywords). `hypr-osd-apps apps` prints every application with the drawer it landed in; changing it means changing the `.desktop` file. |
| A pinned tile is not in the **Pinned** drawer | Pins are desktop file ids in `~/.config/hypr-osd/apps-pinned` (`hypr-osd-apps status` prints the list) — an id that no installed entry has any more is ignored. |
| The bluetooth manager is empty or says "No controller" | `hypr-osd-bluetooth status` reads the controller and every device without a card. `No controller` means BlueZ did not answer (`systemctl status bluetooth`); `Bluetooth is off` means the radio is off — the header's ⏻ switches it. A device a scan found is listed under **Nearby** until it is paired. |
| A bluetooth device will not pair | The card pairs with a `NoInputNoOutput` agent, so a device that insists on a passkey cannot be paired from it — the note under the list says so. `bluetoothctl` in a terminal can still ask for one. |

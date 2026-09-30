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
[Look and feel](#look-and-feel). It does not take the keyboard, with four
deliberate exceptions: cards that are *driven* by keys have to own them while they
are up (see [How it works](#how-it-works)).

| Element | Binary | What it does |
| --- | --- | --- |
| The bar | `hypr-osd-bar` | Workspaces, window title, media, tray (the application indicators **and** the windows you have put away), the system info pill, the combined status pill (link, sound, battery), clock and the power button, across the top of every screen. Hovering the clock unfolds the island popup below it, hovering (or clicking) the status pill at the right end unfolds the system popup. `SUPER + H` puts the focused window away, `SUPER + SHIFT + H` brings the last one back. |
| Island popup | `hypr-osd-island` | What is playing (cover, title, artist, progress with click-to-seek, shuffle/previous/play/next/repeat, the player's own volume and a chip per player), the notification hub, and a calendar — the panel that comes out of the bar's clock. |
| System popup | `hypr-osd-stats` | CPU, memory and temperature as gauges, the pending updates, the bluetooth radios **with the devices they know** (power, visibility, connect, disconnect, pair, forget, scan, battery and signal readouts) and the switches: the wireless link, the sound, the power profile, presentation mode, brightness and the keyboard layout — the panel that comes out of the bar's status pill. |
| Volume card | `hypr-osd-volume` | Speaker glyph, draggable slider and percentage for the default sink. Owns the volume step for `XF86Audio{RaiseVolume,LowerVolume,Mute}`. |
| Media card | `hypr-osd-media` | Cover art, title and artist, with previous/next buttons. Appears when a new medium starts playing — no keybinding involved. |
| Session card | `hypr-osd-session` | Lock, suspend, log out, reboot and shut down. `SUPER + SHIFT + L`, or the hardware power key. |
| Window switcher | `hypr-osd-switcher` | Every window you have open, most recently used first, with its application icon and title. `ALT + TAB`, and `ALT + SHIFT + TAB` to walk backwards. |
| Workspace overview | `hypr-osd-overview` | The whole desktop as a picture of itself: every workspace as a chip, every window as a real thumbnail in its own shape, scaled to fit one screen. `SUPER + SHIFT + TAB`. |
| Launcher | `hypr-osd-launcher` | A search card in the middle of the screen: it matches the applications installed on the machine (names, generic names, keywords, categories) and the files in your own directories, and opens the selected one. `SUPER + SPACE`. Replaces the Tauri/Next.js [`linux-launchpad`](../linux-launchpad) — see [Launcher](#launcher). |
| Applications panel | `hypr-osd-apps` | Every application installed on the machine, as a grid of tiles with a drawer list down the side (and a **Pinned** one), searched through the same field the launcher has. The old launchpad's main window, rebuilt as a card and opened by the **grid button at the bar's left end** — see [Applications panel](#applications-panel). |

Siblings of this repo, which share the design language:
[`linux-launchpad`](../linux-launchpad) — whose spotlight overlay this collection
took over — and [`hyprland-settings-gui`](../hyprland-settings-gui).

```
crates/
  hypr-osd-core/       the shared half: the palette and the card/bar recipes, the
                       layer surface, the single-instance command line, the config
                       reader, Hyprland's two sockets with their event stream,
                       MPRIS, the window and workspace list with its application
                       icons, and the two ways to run a command without blocking
  hypr-osd-bar/        the bar, and the StatusNotifier tray host it carries
  hypr-osd-island/     the popup that unfolds from the clock
  hypr-osd-volume/     element #1: the volume card
  hypr-osd-media/      element #2: the media card
  hypr-osd-session/    element #3: the session card
  hypr-osd-switcher/   element #4: the Alt-Tab window switcher
  hypr-osd-overview/   element #5: the workspace overview
  hypr-osd-launcher/   element #6: the search card on SUPER + SPACE
  hypr-osd-apps/      element #7: the applications panel (the bar's grid button)
scripts/
  install.sh           build + install + wire up Hyprland (and drop waybar)
  hyprland/osd.lua     the Hyprland side (autostart, keys, layer rule)
configs/
  theme.css            the palette: every colour of this desktop
  hyprpaper.conf       the desktop wallpaper (the lock screen's, too)
  hyprlock.conf        the lock screen, in this theme's palette
  hypridle.conf        idle timers: lock, blank the panel, sleep
```

## Look and feel

There is exactly one place to change a colour, and it is not in a program:

1. **The palette is `~/.config/hypr-osd/theme.css`** — installed from
   [`configs/theme.css`](configs/theme.css), and the single source of truth for
   the whole desktop: the bar, every card, the island popup and the lock screen.
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
   bar uses, the bar's workspace pills wear the accent fill the session card puts
   on a row you are about to act on, and a muted sink is the same `@crit` red
   everywhere.

GTK CSS is a subset: no `var()` (colour tokens are `@define-color`/`@name`), no
`transform`, and no CSS animations — the bar and the cards fade in through
Hyprland's own layer animation (`layersIn`/`layersOut`), which also keeps them
consistent with every other surface on the desktop.

```sh
# after editing ~/.config/hypr-osd/theme.css
pkill -f hypr-osd-bar;     hypr-osd-bar &        # the bar (and its tray)
pkill -f hypr-osd-island;  hypr-osd-island &     # the popup under the clock
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
| `~/.local/bin/hypr-osd-*` | the ten binaries (the bar, the island, and the eight cards) |
| `~/.config/hypr-osd/theme.css` | the palette — only if the file is missing, so your edits survive |
| `~/.config/hypr-osd/bar.conf` | the bar's settings (only created if missing) |
| `~/.config/hypr-osd/island.conf` | the island popup's settings (only created if missing) |
| `~/.config/hypr-osd/*.conf` | one config template per card (only created if missing) |
| `~/.config/hypr/osd.lua` | the bar, the autostart, every keybinding and the layer rule, generated from `scripts/hyprland/osd.lua` |
| `~/.config/hypr/hyprland.lua` | one appended line: `require("osd")` (backup: `hyprland.lua.bak`), then `hyprctl reload` |
| `~/.config/hypr/hyprland.lua` | waybar's autostart commented out, and waybar stopped (backup: `hyprland.lua.bak-waybar`) — see [Waybar](#waybar) |
| `~/.config/hypr/hyprland.lua` | `require("launchpad")` commented out, and the old launcher daemon stopped, because `SUPER + SPACE` now belongs to `hypr-osd-launcher` (backup: `hyprland.lua.bak-launchpad`) — see [Launcher](#launcher) |
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
bar is a grid of squares; clicking it asks this element to `toggle`, and the pill
lights up in the accent while the panel is up, so the button is also the way back
out. A launcher you have to remember a key for is one you use when you already
know what you want to run; a grid of everything installed is what you use when you
do not, and the pointer is already on the bar. `osd.lua` has a commented
`SUPER + A` bind if you want a key for it as well.

| | |
| --- | --- |
| Open it | the grid button at the bar's **left** end, or `hypr-osd-apps toggle` |
| Walk the tiles | the arrow keys (left/right/up/down, `Page Up`/`Page Down`, `Home`/`End`), or the pointer |
| Change drawer | `Tab`/`Shift+Tab`, a click on a drawer row, or `hypr-osd-apps drawer <name\|number>` |
| Open | `Enter`, or a left click on a tile |
| Pin | a **right click** on a tile (`~/.config/hypr-osd/apps-pinned`, one id per line) |
| Close | `Escape`, the button again, or `hypr-osd-apps hide` |

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
  other keyboard cards do: `Escape`, the button again, or `idle_close_ms` (a
  minute by default) if you walked away.
* **The card keeps its size.** Picking a drawer with two applications in it cannot
  make the panel jump: the grid lives in a fixed-height scroller (`grid_height`,
  which is why that key wants to be a whole number of tile rows), so the card is
  the same shape whichever drawer you are on.

Unlike the launcher, nothing here waits for a key press: the button is the
trigger, and it is the *bar* that starts the element — `apps_command` in
`bar.conf`. That is also why the panel is autostarted with the others: it reads
the installed applications once, while nobody is waiting, so the first click paints
at once.

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

The bar's left half is the workspaces (click to focus one, wheel to walk to the
next or previous) and the focused window's title. Right half: the media pill
(click to play/pause, right-click for the next track, middle-click for the
previous one, wheel to seek), the network pill (left click opens `nmtui` in a
terminal, right click toggles the radio), the **system info pill** — CPU load,
memory in use, package temperature and the number of pending updates, with the
same amber/red thresholds the old bar used (CPU 60/85 %, memory 70/90 %,
temperature 75/90 °C) — then the volume pill (click mutes, wheel steps the volume
*through the volume element*, right-click opens `pavucontrol`, middle-click mutes
the microphone), the battery, the tray, and the power button (which opens the
session card). Every pill has a tooltip with the whole story behind it.

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

**`hypr-osd-island` — the clock's popup**

```sh
hypr-osd-island open    # what hovering the bar's clock runs
hypr-osd-island close   # take it away
hypr-osd-island toggle  # close it if it is up, otherwise show it
hypr-osd-island show    # put it up and keep it there (no pointer tracking)
hypr-osd-island status  # what the panel would show, without showing it
```

Hovering the clock in the bar unfolds a panel below it: the time and date, the
player below, the notification hub, and a calendar you can page through by month,
with today in the accent colour.

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
"the pointer is on the clock" (`open`), and the panel then samples the pointer to
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
hypr-osd-apps toggle           # open it, or close it again (what the bar's button runs)
hypr-osd-apps show [query]     # open it, with an empty field or the query given
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
drawer did that application land in?" without a card.

## Configure

One file per element under `~/.config/hypr-osd/`, every key optional (the
defaults are in the comments of the installed templates). `theme.css` is not a
element config: it is the palette, and it is [its own section](#look-and-feel).

**`bar.conf`**

| Key | Default | |
| --- | --- | --- |
| `height` | `40` | the bar's height in pixels |
| `margin_top` | `8` | how far below the top edge it floats |
| `margin_x` | `12` | the inset at each side |
| `exclusive` | `true` | reserve the strip with the compositor, so windows start below it |
| `output` | *(empty)* | the connector to put the bar on, and *only* that one; empty = a bar on every monitor |
| `workspaces` | `5` | how many numbered workspaces the row keeps room for |
| `title_width` | `320` | how much room the window title may take before it is ellipsised |
| `clock_format` | `%H:%M` | the clock's `strftime` format |
| `battery`, `adapter` | `BAT1`, `ADP1` | the battery to watch, and the mains supply that says whether it is charging |
| `network_interface` | `wlan0` | the wireless interface to read through `iw` |
| `tray_icon_size` | `18` | tray icon size in pixels (the application indicators and the put-away windows alike) |
| `tick_ms` | `1000` | the heartbeat; the clock's resolution |
| `volume_every_ms`, `network_every_ms`, `battery_every_ms` | `1000`, `5000`, `30000` | how often the slow pills are re-read |
| `stats_every_ms` | `1000` | how often CPU, memory and temperature are read |
| `updates_every_ms` | `1800000` | how often a stale update count is refreshed (half an hour) |
| `updates_command` | `checkupdates` | what prints the pending-update list; empty turns the count off |
| `resync_every_ms` | `5000` | how often Hyprland and MPRIS are re-read even with no event |
| `volume_command` | `hypr-osd-volume` | what the volume pill's click and wheel run |
| `session_command` | `hypr-osd-session` | what the power button runs |
| `island_command` | `hypr-osd-island` | what the clock's hover runs |
| `stats_command` | `hypr-osd-stats` | what the status pill's hover and click run |
| `apps_command` | `hypr-osd-apps` | what the grid button at the bar's **left** end runs (it asks the panel to `toggle`) |
| `notifications_command` | `swaync-client` | what the clock's clicks run |
| `terminal_command` | `kitty` | the terminal the network pill opens `nmtui` in |

The bar is event driven: Hyprland's own event socket drives the workspace row and
the title, and `playerctl --follow` drives the media pill. The `*_every_ms` keys
are the intervals for the things that have no event to listen to (a battery
discharging has nothing to say about itself), and `resync_every_ms` is the safety
net for an event socket that quietly died.

**`island.conf`**

| Key | Default | |
| --- | --- | --- |
| `bar_height`, `bar_margin_top` | `40`, `8` | where the bar is — these have to match `bar.conf` |
| `gap` | `6` | the distance between the bar's bottom edge and the panel |
| `hot_width` | `280` | how wide the hover zone around the bar's centre is |
| `left_width`, `right_width` | `252`, `178` | the panel's two columns |
| `open_delay_ms` | `120` | how long the pointer has to rest on the clock before the panel opens |
| `close_delay_ms` | `300` | how long it may be away before the panel closes |
| `poll_ms` | `50` | how often the pointer is sampled, while the panel matters |
| `tick_ms` | `1000` | the clock's resolution |
| `media_every_ms` | `700` | how often the progress bar is refreshed while the panel is up |
| `notifications_command` | `swaync-client` | what the hub's buttons run |

The panel is told nothing by the bar: it works the bar's geometry out from
`bar_height`, `bar_margin_top` and `gap`, which is why those three have to agree
with `bar.conf`. The hover zone is derived rather than measured, because the
clock is the bar's middle child and therefore always centred on it.

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
| `bar_height`, `bar_margin_top`, `bar_margin_x` | `40`, `8`, `12` | where the bar is — these have to match `bar.conf` |
| `idle_close_ms` | `60000` | close after this long with no key and no pointer movement; `0` never closes by itself |

`width`, `sidebar_width` and `tile_width` are what decide how many tiles fit in a
row (5 by default), and the element's arrow keys and the grid both count them the
same way, so "down" always moves down a row. The pinned applications are **not** a
setting: they are the state file `~/.config/hypr-osd/apps-pinned`, written by a
right click on a tile — delete it to unpin everything.

The scan itself is not duplicated: the launcher and the panel both read the
applications through `hypr-osd-core`'s `apps` module (the same `.desktop` parser,
the same category map and the same ranking), so an application is classified and
scored identically in both cards.

## Lock screen

**hyprlock** draws it (`sudo pacman -S hyprlock`), from
`~/.config/hypr/hyprlock.conf`, which this repo carries as
[`configs/hyprlock.conf`](configs/hyprlock.conf) and the installer writes when
the file is missing — it is meant to be edited, so re-running `install.sh` leaves
your version alone.

hyprlock cannot read CSS, so the palette is mirrored at the top of that file as
`$variables` — the same `@define-color` values every element reads out of
`~/.config/hypr-osd/theme.css`, written as `#RRGGBBAA`. Same deal as everywhere
else in this theme: change a colour in `theme.css`, change it here.

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
  the slider needs it. The switcher, the overview, the launcher and the
  applications panel ask for the keyboard (`Opts::keyboard`) for as long as their
  card is on screen, because all four are driven by keys that must not also reach
  the window underneath: an Alt release ends a switch, Escape and the arrows
  cancel an overview, and a launcher or the panel has to be *typed into*. All four
  are opened on purpose, which is what makes eating a keystroke acceptable there,
  and all four let go of the keyboard by themselves if they are left alone — the
  switcher when Alt comes back up, the rest after `idle_close_ms`.
- **The transparent frame is click-through.** The surface is the card plus an
  18px ring that lets the card's shadow breathe; `osd.lua`'s `ignore_alpha` layer
  rule makes those transparent pixels pass clicks to whatever is behind them, so
  only the card itself is hit-testable.
- **A card follows the focused monitor.** Each command asks `hyprctl -j
  activeworkspace` which output is focused, and the surface is (re)built for that
  monitor, so the card appears where you are looking. One monitor means this is a
  no-op. The bar is the deliberate exception: it is on *every* output (see
  above), and a popup that unfolds from it is told which of the bars it came
  from.
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
  names each window for it (`stableId`), and the capture is scaled down and
  encoded as JPEG before it is read: the card holds textures for as long as it is
  up, and a dozen full-size windows would be a couple of hundred megabytes.
- **The overview sizes itself to fit.** Tile height is solved, not guessed: a
  tile's width follows from the window's shape, so "does it fit the screen" is a
  question about one number, and the largest number that fits is the answer — no
  scrolling, because the whole point is seeing everything at once. The
  arithmetic is a pure function in `layout.rs`, which is also where its tests
  are.
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
8. `cargo fmt && cargo clippy --all-targets && cargo test`.

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
| The island popup never opens | Is it running (`pgrep -f hypr-osd-island`)? Is `island.conf`'s `hot_width` wide enough for the clock pill, and do `bar_height`/`bar_margin_top` match `bar.conf`? `hypr-osd-island status` prints the geometry it works with. |
| The island opens or closes too eagerly | `open_delay_ms` and `close_delay_ms` in `island.conf`: the first is how long the pointer has to rest on the clock, the second how long it may be away before the panel closes. |
| The clock pill stays lit with no panel behind it | A stale “panel is open” flag — the island writes it on show/hide, so a `kill` leaves it behind. It is cleared at start-up: `pkill -f hypr-osd-island; hypr-osd-island &`. |
| The bar's clock says the wrong time | It ticks once a second by default; `tick_ms` in `bar.conf` is the resolution (and the clock's `strftime` format is `clock_format`). |
| No card, but the volume changes | The daemon died — check its stderr, and `pgrep -x hypr-osd-volume`. On a session without `gtk4-layer-shell` it cannot create the surface at all. |
| The volume moves two steps per press | `hyprland.lua` still binds the volume keys to `wpctl`; re-run with `--take-over-keys`. |
| `wpctl … failed` on stderr | PipeWire/WirePlumber trouble; `wpctl status` should list a default sink. |
| Card in the wrong place | `bottom_margin` / `width` in the element's config. |
| Card on the wrong monitor | It follows the focused output through `hyprctl`; if `hyprctl` is unavailable it stays on the monitor it was built for. |
| No media card appears | Is the daemon running (`pgrep -x hypr-osd-media`)? Does the player speak MPRIS (`playerctl -l`)? A track that is merely loaded, never played, does not trigger a card unless `only_when_playing = false`. |
| Media card shows no cover | The player reported none, or reported it as a `blob:`/`data:` URL — only `file://` and `http(s)://` artwork can be read. `playerctl metadata | grep -i arturl` shows what it sends. |
| “Lock” is greyed out | No screen locker is installed — `hypr-osd-session status` names the candidates (`hyprlock`, `swaylock`, `gtklock`). The row re-checks every time the card is shown, so installing one is enough. |
| Nothing locks when you walk away | That is hypridle's job, and it is not installed: `sudo pacman -S hypridle`, then re-run `install.sh`. `pgrep -af hypridle` should show it running. |
| The power button doesn't show the session card | systemd-logind is handling the key (`HandlePowerKey=`); see the note in [How it works](#how-it-works) and re-run `install.sh` to see the current value. |
| The session card won't go away | Press `SUPER + SHIFT + L` again, or set `duration_ms` to something shorter in `session.conf` (`0` means “stay until dismissed”, which is also what a card with no way out looks like). |
| The overview shows icons instead of pictures | No `grim`, or a compositor that cannot hand over a single window (`hypr-osd-overview status` says which windows have a capture id; a *nested* Hyprland cannot capture a toplevel, a real one can). `thumbnails = false` makes the icon tiles the deliberate look. |
| The overview won't go away | It takes the keyboard while it is up: Escape, a click on the card where there is no tile, or the key again. `hypr-osd-overview close` works from anywhere, and after `idle_close_ms` (a minute by default) it closes itself — which is what gets rid of it after a session lock. |
| A card is on screen but nothing responds to keys | A keyboard card is up (switcher, overview, launcher or applications panel) and something else grabbed the keyboard first — usually a session lock. Wait for `idle_close_ms`, or `hypr-osd-overview close` / `hypr-osd-switcher cancel` / `hypr-osd-launcher hide` / `hypr-osd-apps hide`. |
| `SUPER + SPACE` opens nothing | Is the launcher running (`pgrep -f hypr-osd-launcher`)? `hyprctl -j layers` shows the surface (namespace `hypr-osd`, overlay layer) when it is up, and `hypr-osd-launcher status` says `state open` or `closed`. |
| Two search bars open at once | Both launchers are bound to `SUPER + SPACE`: `launchpad.lua` is still loaded. Re-run `./scripts/install.sh` (it comments the `require("launchpad")` line out), or bind one of them elsewhere. |
| The launcher's results are not what I expected | `hypr-osd-launcher search <text>` prints the ranking it decided on, with the desktop file each application came from. Keywords and categories count as matches, which is why an application can appear for a word that is not in its name. |
| The launcher lists applications but no files | Do the directories exist? `hypr-osd-launcher status` prints the file roots it found (`Documents`, `Downloads`, … — a root that is not a directory is skipped). `files = false` in `launcher.conf` turns the file half off on purpose. |
| The launcher won't go away | It takes the keyboard while it is up: Escape, or `SUPER + SPACE` again. `hypr-osd-launcher hide` works from anywhere, and after `idle_close_ms` (a minute by default) it closes itself — which is what gets rid of it after a session lock. |
| The bar has no grid button at its left end | The running bar is older than the panel: re-run `./scripts/install.sh` and restart it (`pkill -f hypr-osd-bar; hypr-osd-bar &`). The installer also appends `apps_command` to an existing `bar.conf`, which is the key the button needs. |
| The grid button does nothing | `apps_command` in `bar.conf` does not resolve: is the element running (`pgrep -f hypr-osd-apps`), and does that command work in a terminal? The built-in default is the bare program name, which needs `~/.local/bin` on the bar's `PATH` — that is why the installer writes an absolute path. |
| The button stays lit with no panel behind it | A stale “panel is open” flag, the same one the island's clock can leave behind: `hypr-osd-apps hide` clears it, and the element clears it at start-up. |
| The applications panel won't go away | It takes the keyboard while it is up: `Escape`, the button again, or `hypr-osd-apps hide` from anywhere. After `idle_close_ms` (a minute by default) it closes itself — which is what gets rid of it after a session lock. |
| Applications are in the wrong drawers | The drawer comes from the entry's `Categories=` (and, for entries that claim nothing, from its name and keywords). `hypr-osd-apps apps` prints every application with the drawer it landed in; changing it means changing the `.desktop` file. |
| A pinned tile is not in the **Pinned** drawer | Pins are desktop file ids in `~/.config/hypr-osd/apps-pinned` (`hypr-osd-apps status` prints the list) — an id that no installed entry has any more is ignored. |

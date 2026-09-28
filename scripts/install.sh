#!/usr/bin/env bash
#
# Install the hypr-osd elements as user-level applications (no root required).
#
# Installs, per element:
#   - binary -> ~/.local/bin/<element>
#   - config -> ~/.config/hypr-osd/<element>.conf (a commented template, only
#               written when the file does not exist yet)
#
# The desktop's palette (~/.config/hypr-osd/theme.css) is installed the same way:
# it is the single source of truth for every colour in this theme, and every
# element reads it at start-up.
#
# On Hyprland it also wires up the compositor side (Wayland cannot be driven from
# inside an app):
#   - ~/.config/hypr/osd.lua            the bar, autostart, keys, layer rule
#   - a `require("osd")` line in ~/.config/hypr/hyprland.lua
#   - waybar is taken out of the autostart (see --keep-waybar), because the bar
#     in this repository replaces it
#   - the launcher takes over SUPER + SPACE from linux-launchpad, whose
#     `require("launchpad")` line is commented out (see --keep-launchpad); the
#     applications panel is the old launcher's *main window*, and is opened by
#     the button at the bar's left end
#   - ~/.config/hypr/hyprlock.conf      the lock screen, when hyprlock is there
#   - ~/.config/hypr/hypridle.conf      idle timers, when hypridle is there
#
# Usage:
#   ./scripts/install.sh                    # build + install (+ Hyprland setup)
#   ./scripts/install.sh --no-hyprland      # skip the Hyprland integration
#   ./scripts/install.sh --take-over-keys   # comment out the old wpctl keybinds
#   ./scripts/install.sh --keep-waybar      # leave the old bar running alongside
#   ./scripts/install.sh --keep-launchpad   # leave the old SUPER + SPACE binding
#
set -euo pipefail

cd "$(dirname "$0")/.."

# The elements this script installs. Add an element here (and to osd.lua) when
# the collection grows. Order is only the order they are reported in.
ELEMENTS=(hypr-osd-bar hypr-osd-island hypr-osd-stats hypr-osd-volume hypr-osd-media hypr-osd-session hypr-osd-switcher hypr-osd-overview hypr-osd-launcher hypr-osd-apps)

SKIP_HYPRLAND=0
TAKE_OVER_KEYS=0
KEEP_WAYBAR=0
KEEP_LAUNCHPAD=0
for arg in "$@"; do
  case "$arg" in
    --no-hyprland) SKIP_HYPRLAND=1 ;;
    --take-over-keys) TAKE_OVER_KEYS=1 ;;
    --keep-waybar) KEEP_WAYBAR=1 ;;
    --keep-launchpad) KEEP_LAUNCHPAD=1 ;;
    -h|--help) sed -n '2,31p' "$0"; exit 0 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
HYPR_DIR="$CONFIG_HOME/hypr"
OSD_CONFIG_DIR="$CONFIG_HOME/hypr-osd"

# ---------------------------------------------------------------------------
# Preflight: fail with the exact fix instead of a cryptic pkg-config error
# ---------------------------------------------------------------------------
missing=()
command -v cargo >/dev/null || missing+=("cargo (Rust toolchain)")
if command -v pkg-config >/dev/null; then
  pkg-config --exists gtk4 || missing+=("pkg-config: gtk4")
  pkg-config --exists gtk4-layer-shell-0 || pkg-config --exists gtk4-layer-shell \
    || missing+=("pkg-config: gtk4-layer-shell-0")
else
  missing+=("pkg-config")
fi

if [ "${#missing[@]}" -gt 0 ]; then
  {
    echo "ERROR: missing build dependencies:"
    printf '  - %s\n' "${missing[@]}"
    echo
    echo "On CachyOS/Arch:"
    echo "  sudo pacman -S --needed base-devel pkgconf gtk4 gtk4-layer-shell"
    echo "  sudo pacman -S --needed rust        # if the toolchain is missing"
  } >&2
  exit 1
fi

command -v wpctl >/dev/null || {
  echo "WARNING: wpctl not found - the volume card and the bar's volume pill read" >&2
  echo "         and write the sink through it, so install PipeWire/WirePlumber." >&2
}

# The bar reads the system directly, and reports what it cannot read by leaving a
# pill out. These are the tools whose absence costs a pill or two, not the whole
# bar - so they are reported, never required.
for tool in iw playerctl swaync-client busctl hyprctl; do
  command -v "$tool" >/dev/null || {
    case "$tool" in
      iw) echo "WARNING: iw not found - the bar's network pill stays hidden." >&2 ;;
      playerctl) echo "WARNING: playerctl not found - no media pill, and the media card" >&2
                 echo "         has nothing to watch." >&2 ;;
      swaync-client) echo "WARNING: swaync-client not found - the island popup's notification" >&2
                     echo "         tile stays empty." >&2 ;;
      busctl) echo "WARNING: busctl not found - the island cannot read what the" >&2
              echo "         notifications say (it needs systemd 257 or later for" >&2
              echo "         `busctl monitor --json=`), so the tile shows the count" >&2
              echo "         and the buttons only." >&2 ;;
      hyprctl) echo "WARNING: hyprctl not found - the bar cannot see workspaces or windows." >&2 ;;
    esac
  }
done

# ---------------------------------------------------------------------------
# Build and install the binaries
# ---------------------------------------------------------------------------
echo "== Building release =="
# Every element is a normal binary; there is no bundling step and no dev server.
cargo build --release

mkdir -p "$BIN_DIR"
for element in "${ELEMENTS[@]}"; do
  binary="target/release/$element"
  if [ ! -x "$binary" ]; then
    echo "ERROR: $binary was not built" >&2
    exit 1
  fi
  install -m755 "$binary" "$BIN_DIR/$element"
  echo "   -> $BIN_DIR/$element"
done

# ---------------------------------------------------------------------------
# Configuration templates
# ---------------------------------------------------------------------------
echo "== Configuration =="
mkdir -p "$OSD_CONFIG_DIR"

# The palette first: every element reads it at start-up, and it is the one file
# that decides what the whole desktop looks like. It is installed like the
# element configs - only when missing - so an edit here survives a re-install.
if [ ! -f "$OSD_CONFIG_DIR/theme.css" ]; then
  install -m644 configs/theme.css "$OSD_CONFIG_DIR/theme.css"
  echo "   -> $OSD_CONFIG_DIR/theme.css (new)"
else
  echo "   -> $OSD_CONFIG_DIR/theme.css (left alone)"
fi

if [ ! -f "$OSD_CONFIG_DIR/bar.conf" ]; then
  cat > "$OSD_CONFIG_DIR/bar.conf" <<'EOF'
# hypr-osd-bar - settings for the top bar (hyprland_lvmgui).
# Everything here has a built-in default (shown in the comments); delete a line
# to fall back to it. The bar reads this file once, at start-up:
#     pkill -f hypr-osd-bar && ~/.local/bin/hypr-osd-bar &
# (`-f`, because the element's name is longer than the 15 characters `pkill -x`
#  can match.)
#
# The palette is NOT here: it lives in ~/.config/hypr-osd/theme.css, which the
# bar reads at start-up and paints itself from.

# The bar's geometry, in pixels: it floats below the top edge with an equal
# margin at each side.
height = 40
margin_top = 8
margin_x = 12

# Whether the bar reserves its strip with the compositor. On, tiled windows are
# laid out below it; off, they slide under it.
exclusive = true

# Which output the bar lives on, and *only* that one. Empty means a bar on every
# monitor, which is what a bar is for: one surface per screen, kept in step with
# the compositor's monitor list, so a screen plugged in later grows one and an
# unplugged one takes its bar away. Set a connector name (eDP-1, DP-2, ...) to
# keep the bar on a single screen.
output =

# How many numbered workspaces the row keeps room for, so a workspace you have
# never visited is still clickable. Workspaces that exist beyond this number get
# a pill as well.
workspaces = 5

# How much room the focused window's title may take before it is ellipsised.
title_width = 320

# The clock, in strftime terms. The island popup shows the date and the time;
# this is the pill.
clock_format = %H:%M

# The battery to watch, and the mains supply that says whether it is charging.
# A battery that is not there is looked up by name prefix instead (so a renamed
# one is still found); a machine with no battery gets no battery pill.
battery = BAT1
adapter = ADP1

# The wireless interface to read, through `iw`. No such interface - or no `iw` -
# means no network pill.
network_interface = wlan0

# Tray icon size, in pixels.
tray_icon_size = 18

# The heartbeat, in milliseconds, and how often each source is read. The tick is
# the clock's resolution; the three intervals are how often the slow pills are
# refreshed, counted in ticks (so `volume_every_ms = 1000` with a 1000 ms tick
# means every tick).
tick_ms = 1000
volume_every_ms = 1000
network_every_ms = 5000
battery_every_ms = 30000

# The system info pill (CPU, memory, temperature). Three file reads, so it keeps
# up with the heartbeat - the CPU number is a delta between two of them, which is
# why the pill appears on the second tick rather than the first.
stats_every_ms = 1000

# The pending-update count in the same pill. It comes from `checkupdates`, which
# syncs a throwaway pacman database and takes a second or two, so its answer is
# cached in a file and believed for half an hour; this is how often a stale cache
# is refreshed, in the background. Leave `updates_command` empty to drop the
# count and show the three readings alone.
updates_every_ms = 1800000
updates_command = checkupdates

# How often Hyprland is re-read even with no event, and how often MPRIS is asked
# again. The Hyprland event socket and `playerctl --follow` are the real
# mechanisms; this is the safety net for one of them quietly dying.
resync_every_ms = 5000

# What the pills run. The installer substitutes absolute paths, so the bar does
# not depend on ~/.local/bin being on Hyprland's PATH.
#
# `volume_command` is the volume *element*: it owns the volume step, so a scroll
# on the status pill asks it rather than calling wpctl behind its back (and the
# card appears, which is the feedback a key press gets). If it cannot be run, the
# bar falls back to wpctl and the step is 5 %.
volume_command = __BIN_DIR__/hypr-osd-volume
session_command = __BIN_DIR__/hypr-osd-session
island_command = __BIN_DIR__/hypr-osd-island
# The status pill's handle: it asks this element to open the system popup on
# hover, to toggle it on a click, and to switch the wireless radio on a right
# click.
stats_command = __BIN_DIR__/hypr-osd-stats
# The clock's left click opens the notification centre, the right one toggles
# do-not-disturb.
notifications_command = swaync-client
# The applications button at the bar's *left* end: a left click asks this element
# to open the applications panel (the drawer of every installed application).
apps_command = __BIN_DIR__/hypr-osd-apps
# Where the popups put the things they open: btop, nmtui, bluetoothctl, pacman.
terminal_command = kitty
EOF
  sed -i "s|__BIN_DIR__|$BIN_DIR|g" "$OSD_CONFIG_DIR/bar.conf"
  echo "   -> $OSD_CONFIG_DIR/bar.conf (new)"
else
  echo "   -> $OSD_CONFIG_DIR/bar.conf (left alone)"
fi

# A bar.conf that was written before the applications panel existed has no
# `apps_command` in it, and the built-in default is the bare program name - which
# only works when ~/.local/bin happens to be on the bar's PATH, and a Hyprland
# started by the session manager usually does not have it. The template is never
# overwritten, so the one key the bar needs is appended here, once. Every other
# new key of this collection lives in the regenerated osd.lua or in a new
# element's own config file, which is why this repair is the only one.
if ! grep -qE '^[[:space:]]*apps_command' "$OSD_CONFIG_DIR/bar.conf"; then
  cat >> "$OSD_CONFIG_DIR/bar.conf" <<EOF

# The applications button at the bar's *left* end: a left click asks this element
# to open the applications panel. (Added by the installer; the built-in default is
# the bare name, which needs ~/.local/bin on the bar's PATH.)
apps_command = $BIN_DIR/hypr-osd-apps
EOF
  echo "   -> $OSD_CONFIG_DIR/bar.conf (apps_command added)"
fi

if [ ! -f "$OSD_CONFIG_DIR/island.conf" ]; then
  cat > "$OSD_CONFIG_DIR/island.conf" <<'EOF'
# hypr-osd-island - settings for the panel that unfolds from the bar's clock.
# Read once at start-up:
#     pkill -f hypr-osd-island && ~/.local/bin/hypr-osd-island &

# Where the bar is, so the panel can hang below it - and so it knows which part
# of the screen counts as "the pointer is still on the clock". These have to
# match ~/.config/hypr-osd/bar.conf: the bar tells the panel nothing, it works
# the geometry out from these numbers.
bar_height = 40
bar_margin_top = 8
bar_margin_x = 12

# The distance between the bar's bottom edge and the panel's top edge.
gap = 6

# How wide the hover zone around the bar's centre is. The clock is the bar's
# middle child, so it is centred on the bar; this is how much of that centre
# counts as hovering it.
hot_width = 280

# The panel's two columns, in pixels: what is playing and what is waiting on the
# left, the calendar on the right. The panel's width follows from them.
left_width = 252
right_width = 178

# How long the pointer has to rest on the clock before the panel opens, and how
# long it may be away - crossing the gap between the two, or leaving for good -
# before it closes. Both exist to keep a passing pointer from flickering it.
open_delay_ms = 120
close_delay_ms = 300

# How often the pointer is sampled, in milliseconds. Only while the panel
# matters: an island that is closed asks the compositor for nothing at all.
poll_ms = 50

# The clock's resolution, and how often what is playing is re-read while the
# panel is up (the progress bar needs this; `playerctl` only speaks when the
# metadata changes).
tick_ms = 1000
media_every_ms = 700

# How many notifications the notification tile lists, newest first, with the
# application's icon and what it said. The hub remembers a few more than it
# draws, so dismissing one promotes the next, and it says "+N more" when the
# daemon is holding notifications this panel never saw (they arrived before it
# started, and swaync does not hand them over). Zero lists none.
#
# The rows are read off the bus (`busctl monitor`, i.e. systemd 257 or later),
# because swaync publishes no list: where the bus refuses to be monitored, the
# count, the dnd switch and the buttons stand on their own.
notification_rows = 3

# What the notification buttons run.
notifications_command = swaync-client
EOF
  echo "   -> $OSD_CONFIG_DIR/island.conf (new)"
else
  echo "   -> $OSD_CONFIG_DIR/island.conf (left alone)"
fi

if [ ! -f "$OSD_CONFIG_DIR/stats.conf" ]; then
  cat > "$OSD_CONFIG_DIR/stats.conf" <<'EOF'
# hypr-osd-stats - settings for the system popup, the card that unfolds from the
# bar's status pill. Read once at start-up:
#     pkill -f hypr-osd-stats && ~/.local/bin/hypr-osd-stats &

# Where the bar is, so the panel can hang below it and line its right edge up
# with the bar's. These have to match ~/.config/hypr-osd/bar.conf.
bar_height = 40
bar_margin_top = 8
bar_margin_x = 12

# The distance between the bar's bottom edge and the panel's top edge.
gap = 6

# How wide the fallback hover zone at the bar's right end is, in pixels. It is
# only used for an `open` that arrives without a rectangle - `hypr-osd-stats
# open` typed by hand: the bar measures its own status pill and sends that
# (`open <connector> <x> <y> <w> <h>`), so the zone is normally exactly the pill
# and never reaches the tray.
hot_width = 240

# The panel's two columns, in pixels: the machine's *load* on the left (the three
# gauges, then the bluetooth radios and the devices they know), the machine's
# *controls* on the right, with the pending updates under them. The panel's width
# follows from them - and they are a floor, not a ceiling: a widget the theme
# will not let be narrower (a progress bar's trough, a slider's) wins over them,
# and `hypr-osd-stats status` prints the width the card actually came out at.
left_width = 260
right_width = 240

# How long the pointer has to rest on the pill before the panel opens, and how
# long it may be away - crossing the gap between the two, or leaving for good -
# before it closes.
open_delay_ms = 120
close_delay_ms = 300

# How often the pointer is sampled, in milliseconds. Only while the panel
# matters: a closed popup asks the compositor for nothing at all.
poll_ms = 50

# The readings that are file reads (CPU, memory, temperature, the update count)
# keep up with the heartbeat; the ones that cost a command (powerprofilesctl,
# rfkill, the backlight, the keyboard layout) are on the slower interval.
tick_ms = 1000
slow_every_ms = 5000

# The bluetooth device list, on the slowest clock of all: it costs one
# `bluetoothctl` per device, so it is re-read every few seconds *while the panel
# is open* and never at all while it is closed.
devices_every_ms = 10000

# How many device rows the list shows before it switches to a count, and how long
# the Scan button looks for devices. bluetoothctl exits by itself after that,
# which is what ends the scan.
max_devices = 5
scan_seconds = 12

# The pending-update count, from the same cache the bar's pill uses. Stale after
# half an hour, at which point `checkupdates` runs again - in the background, in
# both programs. Leave `updates_command` empty to turn the updates tile off.
updates_every_ms = 1800000
updates_command = checkupdates

# The wireless interface the NETWORK row reports, and where the buttons that open
# something put it (btop, nmtui, bluetoothctl, pacman).
network_interface = wlan0
terminal_command = kitty
EOF
  echo "   -> $OSD_CONFIG_DIR/stats.conf (new)"
else
  echo "   -> $OSD_CONFIG_DIR/stats.conf (left alone)"
fi

if [ ! -f "$OSD_CONFIG_DIR/volume.conf" ]; then
  cat > "$OSD_CONFIG_DIR/volume.conf" <<'EOF'
# hypr-osd-volume - settings for the volume card (hyprland_lvmgui).
# Everything here has a built-in default (shown in the comments); delete a line
# to fall back to it. `hyprctl reload` is not needed - the daemon reads this file
# once, at start-up, so restart it to pick changes up:
#     pkill -x hypr-osd-volume && ~/.local/bin/hypr-osd-volume &

# Volume step per key press, in percent.
step_percent = 5

# The slider's ceiling, in percent of full scale. 100 means "no
# over-amplification"; raise it if you want the volume keys to go past 100 %.
max_percent = 100

# How long the card stays after the last change, in milliseconds.
duration_ms = 1400

# Whether `up` (and dragging the slider) also unmutes a muted sink. A volume key
# that changes nothing audible is the most common complaint about the stock
# keybindings, so this defaults to yes.
unmute_on_raise = true

# The card's width in pixels.
width = 340

# Distance from the bottom edge of the screen, as a fraction of the monitor's
# height: 0.12 is roughly a tenth of the way up.
bottom_margin = 0.12
EOF
  echo "   -> $OSD_CONFIG_DIR/volume.conf (new)"
else
  echo "   -> $OSD_CONFIG_DIR/volume.conf (left alone)"
fi

if [ ! -f "$OSD_CONFIG_DIR/media.conf" ]; then
  cat > "$OSD_CONFIG_DIR/media.conf" <<'EOF'
# hypr-osd-media - settings for the media card (hyprland_lvmgui).
# Everything here has a built-in default (shown in the comments); delete a line
# to fall back to it. Like the volume card, this file is read once at start-up:
#     pkill -x hypr-osd-media && ~/.local/bin/hypr-osd-media &

# How long the card stays after the last change, in milliseconds. Longer than
# the volume card's, because there is a title and an artist to read.
duration_ms = 4000

# Only show the card for something that is actually playing. With this off, the
# card also appears when a player loads a track while paused.
only_when_playing = true

# The card's width in pixels - cover, title, artist and the two skip buttons.
width = 400

# Distance from the bottom edge of the screen, as a fraction of the monitor's
# height.
bottom_margin = 0.12
EOF
  echo "   -> $OSD_CONFIG_DIR/media.conf (new)"
else
  echo "   -> $OSD_CONFIG_DIR/media.conf (left alone)"
fi

if [ ! -f "$OSD_CONFIG_DIR/session.conf" ]; then
  cat > "$OSD_CONFIG_DIR/session.conf" <<'EOF'
# hypr-osd-session - settings for the session card (hyprland_lvmgui).
# Read once at start-up:
#     pkill -f hypr-osd-session && ~/.local/bin/hypr-osd-session &
# (`-f`, because the element's name is longer than the 15 characters that
#  `pkill -x` can match.)

# How long the card stays, in milliseconds. It is a menu rather than a
# notification, so it stays longer than the other cards - and 0 keeps it up until
# an action is picked or the key is pressed again.
duration_ms = 8000

# Minimum card width in pixels. 0 lets the rows decide: they are the widest
# thing in the card, and a greyed-out "Lock" row is wider than an enabled one.
width = 0

# Distance from the bottom edge of the screen, as a fraction of the monitor's
# height.
bottom_margin = 0.12
EOF
  echo "   -> $OSD_CONFIG_DIR/session.conf (new)"
else
  echo "   -> $OSD_CONFIG_DIR/session.conf (left alone)"
fi

if [ ! -f "$OSD_CONFIG_DIR/switcher.conf" ]; then
  cat > "$OSD_CONFIG_DIR/switcher.conf" <<'EOF'
# hypr-osd-switcher - settings for the Alt-Tab window switcher (hyprland_lvmgui).
# Read once at start-up:
#     pkill -x hypr-osd-switcher && ~/.local/bin/hypr-osd-switcher &

# Tiles per row. The card's width follows from this and the tile width below, so
# it is the number to change for a wider or narrower grid.
columns = 5

# Width of one tile in pixels - and so how much of a window title fits in it.
tile_width = 132

# Application icon size in pixels.
icon_size = 72

# Commit the switch if no key arrives for this long, in case the Alt release was
# missed: a release that happens before the card owns the keyboard is a release
# nobody can see, and GTK cannot be asked whether a modifier is still held. 0
# turns the safety net off and leaves the decision to Escape, Enter and Tab.
idle_commit_ms = 5000
EOF
  echo "   -> $OSD_CONFIG_DIR/switcher.conf (new)"
else
  echo "   -> $OSD_CONFIG_DIR/switcher.conf (left alone)"
fi

if [ ! -f "$OSD_CONFIG_DIR/overview.conf" ]; then
  cat > "$OSD_CONFIG_DIR/overview.conf" <<'EOF'
# hypr-osd-overview - settings for the workspace overview (hyprland_lvmgui).
# Read once at start-up:
#     pkill -f hypr-osd-overview && ~/.local/bin/hypr-osd-overview &

# How large a tile may be, in pixels. The card solves for a tile size between the
# two: windows are drawn in their own shape, so a larger tile is also a wider
# one, and the size that fits all of them on the screen at once is the one that
# gets used. The maximum is what a single window gets; the minimum is what an
# over-full desktop is allowed to shrink to.
#
tile_min_height = 84
tile_max_height = 300

# Space between tiles, and between rows of them.
gap = 18

# Application icon size inside a tile that has no picture of its own.
icon_size = 64

# Whether to capture the windows at all. Off, the card is a workspace switcher
# drawn with application icons and titles - instant, and what to set if the
# captures are too slow or grim is not installed.
thumbnails = true

# Close the card after this long with no key press and no pointer movement over
# it, in milliseconds. The card takes the keyboard while it is up, and this is
# what makes that safe: a session lock takes the keyboard away from every other
# surface, so a card that was up when the screen locked hears nothing - not even
# the Escape that was meant for it. 0 never closes by itself.
idle_close_ms = 60000
EOF
  echo "   -> $OSD_CONFIG_DIR/overview.conf (new)"
else
  echo "   -> $OSD_CONFIG_DIR/overview.conf (left alone)"
fi

if [ ! -f "$OSD_CONFIG_DIR/launcher.conf" ]; then
  cat > "$OSD_CONFIG_DIR/launcher.conf" <<'EOF'
# hypr-osd-launcher - settings for the search card (SUPER + SPACE), the element
# that replaces the Tauri/Next.js launchpad. Read once at start-up:
#     pkill -f hypr-osd-launcher && ~/.local/bin/hypr-osd-launcher &
# (`-f`, because the element's name is longer than the 15 characters that
#  `pkill -x` can match.)

# The card's width in pixels. The rows inside it are full width and their text is
# ellipsised to fit, so this is the one number that decides how much of a file
# name is readable.
width = 640

# How many results the list holds. Applications and files share the slots. The
# card always reserves room for all of them - a mapped layer surface grows with
# its content but does not shrink with it, and a card that changed size under the
# pointer would be worse anyway - so this is also how tall the card is.
max_results = 7

# Application (and file-type) icon size in pixels.
icon_size = 22

# Whether files and folders are searched as well as applications. On, the card
# walks ~/Documents, ~/Downloads, ~/Desktop, ~/Pictures, ~/Music, ~/Videos,
# ~/Projects, ~/Templates and ~/Public - no index, no watcher, just a bounded
# walk on its own thread. Off, the card is an application launcher only, and no
# filesystem work happens at all.
files = true

# How long a keystroke waits before the file walk is started, in milliseconds.
# The application half answers the keystroke immediately, so this is only about
# not walking the disk once per character while you type.
debounce_ms = 120

# Close the card after this long with no key press and no pointer movement over
# it, in milliseconds. The card takes the keyboard while it is up (a layer surface
# cannot be typed into otherwise), and this is what keeps that safe: a session
# lock takes the keyboard from every other surface, so a card that was up when
# the screen locked hears nothing - not even the Escape meant for it. 0 never
# closes by itself.
idle_close_ms = 60000
EOF
  echo "   -> $OSD_CONFIG_DIR/launcher.conf (new)"
else
  echo "   -> $OSD_CONFIG_DIR/launcher.conf (left alone)"
fi

if [ ! -f "$OSD_CONFIG_DIR/apps.conf" ]; then
  cat > "$OSD_CONFIG_DIR/apps.conf" <<'EOF'
# hypr-osd-apps - settings for the applications panel: every application
# installed on the machine, in drawers, with the search field the launcher has.
# This is the old launchpad's *main window*, rebuilt as a card that unfolds from
# the button at the bar's left end (the Tauri/Next.js window it replaces is
# gone). Read once at start-up:
#     pkill -f hypr-osd-apps && ~/.local/bin/hypr-osd-apps &
# (`-f`, because the element's name is longer than the 15 characters that
#  `pkill -x` can match.)
#
# Without a card, the verbs answer on the command line - which is also how the
# panel is tested:
#     hypr-osd-apps status        # how many applications, which drawers, open?
#     hypr-osd-apps toggle        # the bar's button does this
#     hypr-osd-apps show          # the panel, without touching the bar
#     hypr-osd-apps drawer Pinned # open on a drawer by name (or by number)
#     hypr-osd-apps apps          # one line per application: drawer, id, name

# The panel's width in pixels, and how much of it the drawer list takes. The
# tile grid gets what is left (minus the gap between the two panes), so these
# two decide how many tiles fit in a row - the arrows and the grid both count
# them the same way. `hypr-osd-apps status` prints what it worked out.
width = 760
sidebar_width = 168

# The height of the tile pane - and therefore the height the panel settles at,
# whichever drawer you are on. Make it a whole number of tile rows
# (`rows * tile_height + (rows - 1) * tile_gap`, so 4 rows of the defaults below
# = 402), or the last row sits sliced in half at the pane's edge.
grid_height = 402

# One tile, in pixels, and the application icon inside it. The name under the
# icon gets two lines and is then ellipsised.
tile_width = 100
tile_height = 96
icon_size = 40

# Space between tiles in a row and between the rows above.
tile_gap = 6

# Where the bar is, so the panel can hang below it and line its left edge up with
# the bar's. These have to match ~/.config/hypr-osd/bar.conf: the bar tells the
# panel nothing, it works the geometry out from these numbers.
bar_height = 40
bar_margin_top = 8
bar_margin_x = 12

# The distance between the bar's bottom edge and the panel's top edge.
gap = 6

# Close the panel after this long with no key press and no pointer movement over
# it, in milliseconds. The panel takes the keyboard while it is up (a layer
# surface cannot be typed into otherwise), and this is what keeps that safe: a
# session lock takes the keyboard from every other surface, so a panel that was
# up when the screen locked hears nothing - not even the Escape meant for it. 0
# never closes by itself.
idle_close_ms = 60000

# Pinned applications are not here: they are a state file, not a setting -
# ~/.config/hypr-osd/apps-pinned, one application id per line, written by a
# right click on a tile. Delete the file to unpin everything.
EOF
  echo "   -> $OSD_CONFIG_DIR/apps.conf (new)"
else
  echo "   -> $OSD_CONFIG_DIR/apps.conf (left alone)"
fi

# ---------------------------------------------------------------------------
# The screen locker
# ---------------------------------------------------------------------------
# hyprlock and hypridle are separate programs, and neither is a build dependency
# of the elements: without a locker the session card's Lock row simply greys out
# (and says why), and without hypridle nothing locks by itself. So report what is
# there - the config templates are written by the Hyprland step below, and only
# for the programs that exist.
echo "== Screen locker =="
locker=$(command -v hyprlock || command -v swaylock || command -v gtklock || true)
if [ -n "$locker" ]; then
  echo "   -> $locker (the session card can lock)"
else
  {
    echo "   ! no screen locker found (hyprlock, swaylock or gtklock)."
    echo "     The session card's Lock row stays greyed out, and its Suspend row"
    echo "     will not lock the screen first. On CachyOS/Arch:"
    echo "       sudo pacman -S hyprlock"
    echo "     A locker installed later is picked up the next time the card is"
    echo "     shown - the daemon does not need a restart."
  } >&2
fi

if command -v hypridle >/dev/null; then
  echo "   -> hypridle (locks on idle and before sleep)"
else
  {
    echo "   ! hypridle is not installed, so nothing locks by itself and a"
    echo "     suspend does not lock the screen first (unless the session card is"
    echo "     what suspends). Install it with: sudo pacman -S hypridle"
  } >&2
fi

# ---------------------------------------------------------------------------
# The hardware power key
# ---------------------------------------------------------------------------
# systemd-logind handles the power key itself unless it is told not to, and it
# may consume the event before any compositor binding runs. Report the current
# setting rather than change it: it lives in /etc and needs root.
logind_power_key() {
  value=""
  for file in /usr/lib/systemd/logind.conf /usr/lib/systemd/logind.conf.d/*.conf \
              /run/systemd/logind.conf.d/*.conf /etc/systemd/logind.conf \
              /etc/systemd/logind.conf.d/*.conf; do
    [ -f "$file" ] || continue
    line=$(grep -hE '^[[:space:]]*HandlePowerKey[[:space:]]*=' "$file" 2>/dev/null | tail -1)
    [ -n "$line" ] && value=$(printf '%s' "${line#*=}" | tr -d '[:space:]')
  done
  printf '%s' "${value:-poweroff}"
}

power_key=$(logind_power_key)
if [ "$power_key" != "ignore" ]; then
  {
    echo
    echo "   ! systemd-logind acts on the power key itself (HandlePowerKey=$power_key),"
    echo "     so the power button will '$power_key' and may never reach Hyprland."
    echo "     To hand the key to the session card, as root:"
    echo "       printf '[Login]\\nHandlePowerKey=ignore\\n' | sudo tee /etc/systemd/logind.conf.d/10-power-key.conf"
    echo "       sudo systemctl restart systemd-logind    # or just reboot"
    echo "     (SUPER + SHIFT + L shows the card either way.)"
  } >&2
fi

# ---------------------------------------------------------------------------
# Hyprland integration
# ---------------------------------------------------------------------------
hypr_installed=0

if [ "$SKIP_HYPRLAND" -eq 1 ]; then
  echo "== Skipping Hyprland integration (--no-hyprland) =="
elif [ -d "$HYPR_DIR" ]; then
  echo "== Installing Hyprland integration =="
  if [ ! -f "$HYPR_DIR/hyprland.lua" ]; then
    # Pre-0.55 config (hyprland.conf / hyprlang): the Lua API does not exist
    # there, so print the equivalent lines instead of writing a file Hyprland
    # would never load.
    echo "   ! $HYPR_DIR/hyprland.lua not found - looks like a hyprlang" >&2
    echo "     (Hyprland <= 0.54) config. Add this to it manually:" >&2
    echo "       exec-once = $BIN_DIR/hypr-osd-volume" >&2
    echo "       bindl = , XF86AudioRaiseVolume, exec, $BIN_DIR/hypr-osd-volume up" >&2
    echo "       bindl = , XF86AudioLowerVolume, exec, $BIN_DIR/hypr-osd-volume down" >&2
    echo "       bindl = , XF86AudioMute, exec, $BIN_DIR/hypr-osd-volume toggle" >&2
    echo "       layerrule = ignorealpha 0.2, hypr-osd" >&2
  else
    sed -e "s|@BAR_OSD_BIN@|$BIN_DIR/hypr-osd-bar|g" \
      -e "s|@ISLAND_OSD_BIN@|$BIN_DIR/hypr-osd-island|g" \
      -e "s|@STATS_OSD_BIN@|$BIN_DIR/hypr-osd-stats|g" \
      -e "s|@VOLUME_OSD_BIN@|$BIN_DIR/hypr-osd-volume|g" \
      -e "s|@MEDIA_OSD_BIN@|$BIN_DIR/hypr-osd-media|g" \
      -e "s|@SESSION_OSD_BIN@|$BIN_DIR/hypr-osd-session|g" \
      -e "s|@SWITCHER_OSD_BIN@|$BIN_DIR/hypr-osd-switcher|g" \
      -e "s|@OVERVIEW_OSD_BIN@|$BIN_DIR/hypr-osd-overview|g" \
      -e "s|@LAUNCHER_OSD_BIN@|$BIN_DIR/hypr-osd-launcher|g" \
      -e "s|@APPS_OSD_BIN@|$BIN_DIR/hypr-osd-apps|g" \
      scripts/hyprland/osd.lua > "$HYPR_DIR/osd.lua"
    echo "   -> $HYPR_DIR/osd.lua"

    if grep -q 'require("osd")' "$HYPR_DIR/hyprland.lua"; then
      echo "   -> require(\"osd\") already present in hyprland.lua"
    else
      cp -n "$HYPR_DIR/hyprland.lua" "$HYPR_DIR/hyprland.lua.bak" 2>/dev/null || true
      cat >> "$HYPR_DIR/hyprland.lua" <<'EOF'

-- hypr-osd: volume keys, autostart and the layer rule for the OSDs.
-- Installed by hyprland_lvmgui/scripts/install.sh; edit osd.lua instead.
require("osd")
EOF
      echo "   -> require(\"osd\") appended to hyprland.lua (backup: hyprland.lua.bak)"
    fi

    # -----------------------------------------------------------------------
    # The old bar
    # -----------------------------------------------------------------------
    # `hypr-osd-bar` replaces waybar on this desktop, and the two of them cannot
    # share the top of the screen: both reserve an exclusive zone there, both
    # draw a full-width surface, and the palette used to be read out of waybar's
    # own stylesheet. So the autostart line goes - along with the two GTK
    # processes that existed only to expand a waybar pill on hover.
    #
    # The lines are *commented out* rather than deleted, and the file is backed
    # up first: this is the user's own config, and an installer that rewrites it
    # with no way back is not one you run twice.
    if [ "$KEEP_WAYBAR" -eq 1 ]; then
      echo "   -> waybar left where it is (--keep-waybar)"
    elif grep -qE '^[[:space:]]*hl\.exec_cmd\("[^"]*waybar' "$HYPR_DIR/hyprland.lua" 2>/dev/null; then
      cp -n "$HYPR_DIR/hyprland.lua" "$HYPR_DIR/hyprland.lua.bak-waybar" 2>/dev/null || true
      sed -i -E '/^[[:space:]]*hl\.exec_cmd\("[^"]*waybar/ s|^|-- replaced by hypr-osd-bar: |' \
        "$HYPR_DIR/hyprland.lua"
      echo "   -> waybar autostart commented out (backup: hyprland.lua.bak-waybar)"
      # And stop it now, so the change is visible in this session rather than the
      # next one. The panels go with it: they watch the pointer for a pill that
      # no longer exists.
      pkill -x waybar 2>/dev/null || true
      pkill -f waybar/scripts/island_panel.py 2>/dev/null || true
      pkill -f waybar/scripts/stats_panel.py 2>/dev/null || true
      echo "   -> waybar and its two hover panels stopped"
    else
      echo "   -> no waybar autostart found in hyprland.lua"
    fi

    # -----------------------------------------------------------------------
    # The old launcher
    # -----------------------------------------------------------------------
    # `hypr-osd-launcher` takes over SUPER + SPACE, which the Tauri launcher
    # (linux-launchpad) had first. Both `require`s would bind the key, and one
    # press would open *two* search bars on top of each other - so the line is
    # commented out.
    #
    # Commented out rather than deleted, and the launcher itself is left alone:
    # this is the user's config, the app stays installed and its launchpad.lua is
    # untouched, so removing the comment is all it takes to go back.
    if [ "$KEEP_LAUNCHPAD" -eq 1 ]; then
      echo "   -> launchpad left where it is (--keep-launchpad)"
      {
        echo "   ! both launchpad.lua and osd.lua bind SUPER + SPACE now: one of"
        echo "     the two will win, and if both fire you get two search bars."
      } >&2
    elif grep -qE '^[[:space:]]*require\("launchpad"\)' "$HYPR_DIR/hyprland.lua" 2>/dev/null; then
      cp -n "$HYPR_DIR/hyprland.lua" "$HYPR_DIR/hyprland.lua.bak-launchpad" 2>/dev/null || true
      sed -i -E '/^[[:space:]]*require\("launchpad"\)/ s|^|-- replaced by hypr-osd-launcher: |' \
        "$HYPR_DIR/hyprland.lua"
      echo "   -> launchpad disabled (backup: hyprland.lua.bak-launchpad)"
      # And stop the daemon it left running, so the change is visible in this
      # session rather than the next one.
      pkill -f 'bin/launchpad --hidde[n]' 2>/dev/null || true
      echo "   -> the running launchpad daemon was stopped"
    else
      echo "   -> no require(\"launchpad\") found in hyprland.lua"
    fi

    # Launchpad's in-app "Launch on startup" toggle writes its *own* XDG autostart
    # entry, which a systemd session turns into a user unit - so the daemon can
    # come back at login even with the `require` gone. It is launchpad's own file,
    # so it is reported rather than edited.
    if [ -f "$CONFIG_HOME/autostart/launchpad.desktop" ]; then
      {
        echo "   ! $CONFIG_HOME/autostart/launchpad.desktop still starts the old"
        echo "     launcher at login (its in-app \"Launch on startup\" toggle wrote"
        echo "     it). To let the new one have the key to itself:"
        echo "       rm $CONFIG_HOME/autostart/launchpad.desktop"
      } >&2
    fi

    # The desktop's background, its lock screen and the idle timers. All three
    # configs belong to their own programs and are only written when missing -
    # they are meant to be edited, and re-running the installer must not undo
    # that.
    if command -v hyprpaper >/dev/null; then
      if [ ! -f "$HYPR_DIR/hyprpaper.conf" ]; then
        install -m644 configs/hyprpaper.conf "$HYPR_DIR/hyprpaper.conf"
        echo "   -> $HYPR_DIR/hyprpaper.conf (new)"
      else
        echo "   -> $HYPR_DIR/hyprpaper.conf (left alone)"
      fi
    fi

    if command -v hyprlock >/dev/null; then
      if [ ! -f "$HYPR_DIR/hyprlock.conf" ]; then
        install -m644 configs/hyprlock.conf "$HYPR_DIR/hyprlock.conf"
        echo "   -> $HYPR_DIR/hyprlock.conf (new)"
        # A lock screen that does not parse is worse than none: hyprlock ignores
        # bad options instead of refusing to run, so a typo would be invisible
        # until it mattered. Point it at a display that does not exist - it reads
        # the config first and prints errors with line numbers, then fails to
        # connect, so it cannot lock the session it is being checked from.
        config_errors=$(WAYLAND_DISPLAY=hypr-osd-install-check \
          hyprlock --config "$HYPR_DIR/hyprlock.conf" -v 2>&1 \
          | grep -c "Config error" || true)
        if [ "${config_errors:-0}" -gt 0 ]; then
          echo "   ! hyprlock reported ${config_errors} config error(s). Check with:" >&2
          echo "       hyprlock --config $HYPR_DIR/hyprlock.conf -v" >&2
        fi
      else
        echo "   -> $HYPR_DIR/hyprlock.conf (left alone)"
      fi
    fi

    if command -v hypridle >/dev/null; then
      if [ ! -f "$HYPR_DIR/hypridle.conf" ]; then
        install -m644 configs/hypridle.conf "$HYPR_DIR/hypridle.conf"
        echo "   -> $HYPR_DIR/hypridle.conf (new)"
      else
        echo "   -> $HYPR_DIR/hypridle.conf (left alone)"
      fi
    fi

    # The keys osd.lua adds - the session card's, the lock's and the tray's - have
    # to be free. Only report those: the volume keys are *deliberately* taken over
    # (below), while a key the user already bound would simply be replaced -
    # silently losing a binding is worth a warning.
    if grep -qE 'SHIFT \+ L"' "$HYPR_DIR/hyprland.lua" 2>/dev/null; then
      {
        echo "   ! hyprland.lua already binds SUPER + SHIFT + L (the session card's"
        echo "     key): one of the two will win. Change one of them."
      } >&2
    fi

    if grep -qE 'SUPER \+ L' "$HYPR_DIR/hyprland.lua" 2>/dev/null; then
      {
        echo "   ! hyprland.lua already binds SUPER + L (the lock key osd.lua"
        echo "     adds): one of the two will win. Change one of them."
      } >&2
    fi

    # The tray's two keys are the bar's: SUPER + H puts the focused window away
    # and SUPER + SHIFT + H brings the last one back. Same warning as above - a
    # binding that is silently replaced is worth a line on the way past.
    if grep -qE 'SUPER \+ (SHIFT \+ )?H' "$HYPR_DIR/hyprland.lua" 2>/dev/null; then
      {
        echo "   ! hyprland.lua already binds SUPER + H or SUPER + SHIFT + H (the"
        echo "     tray's put-away keys): one of the two will win. Change one."
      } >&2
    fi

    # The OSD owns the volume step, so these keys must not also run wpctl.
    # Without this, every press would move the volume twice.
    conflicts=$(grep -nE '^[[:space:]]*hl\.bind\("XF86Audio(RaiseVolume|LowerVolume|Mute)"' \
      "$HYPR_DIR/hyprland.lua" 2>/dev/null | grep 'DEFAULT_AUDIO_SINK' || true)
    if [ -n "$conflicts" ]; then
      if [ "$TAKE_OVER_KEYS" -eq 1 ]; then
        cp -n "$HYPR_DIR/hyprland.lua" "$HYPR_DIR/hyprland.lua.bak-osd" 2>/dev/null || true
        sed -i -E \
          '/^[[:space:]]*hl\.bind\("XF86Audio(RaiseVolume|LowerVolume|Mute)".*DEFAULT_AUDIO_SINK/ s/^/-- taken over by osd.lua: /' \
          "$HYPR_DIR/hyprland.lua"
        echo "   -> old volume keybinds commented out (backup: hyprland.lua.bak-osd)"
      else
        {
          echo "   ! hyprland.lua still binds the volume keys with wpctl:"
          printf '       %s\n' "$conflicts"
          echo "     osd.lua now owns them, and wpctl would step the volume a"
          echo "     second time per press. Remove those lines, or re-run with:"
          echo "       ./scripts/install.sh --take-over-keys"
        } >&2
      fi
    fi

    if [ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ] && command -v hyprctl >/dev/null; then
      echo "   -> hyprctl reload"
      hyprctl reload >/dev/null || true
    fi
    hypr_installed=1
  fi
else
  echo "== No ~/.config/hypr - skipping Hyprland integration =="
fi

# ---------------------------------------------------------------------------
# fish: make ~/.local/bin reachable
# ---------------------------------------------------------------------------
if [ "$BIN_DIR" = "$HOME/.local/bin" ] \
  && [ -f "$CONFIG_HOME/fish/config.fish" ] \
  && ! grep -q 'fish_add_path.*\.local/bin' "$CONFIG_HOME/fish/config.fish"; then
  echo "== Adding ~/.local/bin to fish's PATH =="
  cat >> "$CONFIG_HOME/fish/config.fish" <<EOF

# Added by the hypr-osd installer: the elements live in ~/.local/bin.
fish_add_path $BIN_DIR
EOF
  echo "   -> $CONFIG_HOME/fish/config.fish (open a new shell to pick it up)"
fi

echo

echo "✅ hypr-osd installed:"
for element in "${ELEMENTS[@]}"; do
  echo "   $element -> $BIN_DIR/$element"
done
if [ "$hypr_installed" -eq 1 ]; then
  echo
  echo "The palette lives in $OSD_CONFIG_DIR/theme.css, and the bar is on the top"
  echo "of your screen. Right now:"
  echo "   $BIN_DIR/hypr-osd-bar status          # what every pill currently reads"
  echo "   $BIN_DIR/hypr-osd-volume up           # a volume card"
  echo "   $BIN_DIR/hypr-osd-media &             # start watching for tracks"
  echo "   $BIN_DIR/hypr-osd-session toggle      # the session card"
  echo "   $BIN_DIR/hypr-osd-island show         # the clock's popup, without hovering"
  echo "   ALT + TAB                             # the window switcher"
  echo "   SUPER + SHIFT + TAB                   # every workspace, every window"
  echo "   SUPER + SPACE                         # search applications and files"
  echo "   the grid button at the bar's left     # every application, in drawers"
  echo "   $BIN_DIR/hypr-osd-apps show           # ... the panel, without the button"
  echo "   SUPER + L                             # lock the screen (hyprlock)"
  echo "or just log out and back in."
  echo
  echo "Hover the clock in the bar for the island popup; the tray fills in as"
  echo "indicator applications start (they register with the watcher once, when"
  echo "they start - an indicator that was already running needs a restart)."
else
  echo
  echo "Run an element with a verb, e.g. '$BIN_DIR/hypr-osd-bar status', or"
  echo "start a daemon (no verb) and bind the verbs yourself."
fi

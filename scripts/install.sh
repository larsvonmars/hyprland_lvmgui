#!/usr/bin/env bash
#
# Install the hypr-osd elements as user-level applications (no root required).
#
# Installs, per element:
#   - binary -> ~/.local/bin/<element>
#   - config -> ~/.config/hypr-osd/<element>.conf (a commented template, only
#               written when the file does not exist yet)
#
# On Hyprland it also wires up the compositor side (Wayland cannot be driven from
# inside an app):
#   - ~/.config/hypr/osd.lua            volume keys, autostart, layer rule
#   - a `require("osd")` line in ~/.config/hypr/hyprland.lua
#   - ~/.config/hypr/hyprlock.conf      the lock screen, when hyprlock is there
#   - ~/.config/hypr/hypridle.conf      idle timers, when hypridle is there
#
# Usage:
#   ./scripts/install.sh                    # build + install (+ Hyprland setup)
#   ./scripts/install.sh --no-hyprland      # skip the Hyprland integration
#   ./scripts/install.sh --take-over-keys   # comment out the old wpctl keybinds
#
set -euo pipefail

cd "$(dirname "$0")/.."

# The elements this script installs. Add an element here (and to osd.lua) when
# the collection grows.
ELEMENTS=(hypr-osd-volume hypr-osd-media hypr-osd-session hypr-osd-switcher)

SKIP_HYPRLAND=0
TAKE_OVER_KEYS=0
for arg in "$@"; do
  case "$arg" in
    --no-hyprland) SKIP_HYPRLAND=1 ;;
    --take-over-keys) TAKE_OVER_KEYS=1 ;;
    -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
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
  echo "WARNING: wpctl not found - hypr-osd-volume reads and writes the sink" >&2
  echo "         through it, so install PipeWire/WirePlumber." >&2
}

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
    sed -e "s|@VOLUME_OSD_BIN@|$BIN_DIR/hypr-osd-volume|g" \
      -e "s|@MEDIA_OSD_BIN@|$BIN_DIR/hypr-osd-media|g" \
      -e "s|@SESSION_OSD_BIN@|$BIN_DIR/hypr-osd-session|g" \
      -e "s|@SWITCHER_OSD_BIN@|$BIN_DIR/hypr-osd-switcher|g" \
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

    # The session card's keybinding has to be free. Only report these two: the
    # volume keys are *deliberately* taken over (below), while a key the user
    # already bound would simply be replaced - silently losing a binding is
    # worth a warning.
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
  echo "The volume keys are wired up, and every daemon starts at your next login."
  echo "Right now:"
  echo "   $BIN_DIR/hypr-osd-volume up        # a volume card"
  echo "   $BIN_DIR/hypr-osd-media &          # start watching for tracks"
  echo "   $BIN_DIR/hypr-osd-session toggle   # the session card"
  echo "   $BIN_DIR/hypr-osd-switcher next    # the window switcher"
  echo "   ALT + TAB                          # the same thing, with app icons"
  echo "   SUPER + L                          # lock the screen (hyprlock)"
  echo "or just log out and back in."
else
  echo
  echo "Run an element with a verb, e.g. '$BIN_DIR/hypr-osd-volume up', or"
  echo "start a daemon (no verb) and bind the verbs yourself."
fi

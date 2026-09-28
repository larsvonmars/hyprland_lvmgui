#!/bin/sh
# Re-vendor the Lucide icons this collection draws.
#
# Lucide (https://lucide.dev) is an ISC-licensed icon set of *stroke* drawings on
# a 24x24 grid. The elements used to wear Nerd Font glyphs, which made every icon
# a *character* - so a font had to be installed, the glyphs could only be one
# colour per label, and an element could not be read without knowing which
# codepoint meant "speaker". The icons are SVG in the binary instead (see
# `../src/icons.rs` and `build.rs`).
#
# Two steps, because Lucide's strokes are not what GTK draws:
#
#   1. download the icon from `lucide-static`, so what arrives is Lucide's own
#      geometry and nothing else;
#   2. **outline** it (`outline.js`) - GTK's own SVG engine fills paths and does
#      not stroke them, so a stroke drawing arrives as its own silhouette (the
#      clock as a disc, the search glass as a dot). The outlined result is the
#      same drawing expressed as fills, which the engine renders correctly. This
#      is the one transformation the icon set goes through; `README.md` explains
#      it at length.
#
# The files in `lucide/` are therefore *not* byte-identical to Lucide's: they are
# Lucide's drawings, outlined. Anything we draw ourselves lives in `local/`.
#
# Needs the network, `curl` and `node`/`npm` (the outliner is Inkscape's
# algorithm compiled to WASM). *Building* needs none of that - the results are
# committed, which is the point of committing them.
#
#   ./icons/fetch-lucide.sh
#
# The list below is the manifest: running this script is what makes a name in the
# code drawable. Bump the version to update them all.

set -eu

version=1.48.0
base="https://unpkg.com/lucide-static@${version}/icons"
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

# Every icon the collection draws. Named after the thing, not the element: the
# speaker is `volume-2` for the bar, the volume card and the system popup alike,
# which is what keeps the three from drifting apart.
icons="
audio-lines battery battery-charging battery-full battery-low battery-medium
bell bell-off bluetooth camera chevron-left chevron-right circle clapperboard
clock compass cpu download eye eye-off file folder gamepad-2 globe headphones
keyboard layout-grid link list lock log-out memory-stick monitor moon mouse
music pause pin play plus power presentation printer rotate-cw search
skip-back skip-forward sliders-horizontal smartphone speaker sun terminal
thermometer trash-2 tv unlink volume-1 volume-2 volume-x watch wifi wifi-high
wifi-low wifi-off wifi-zero x zap
"

# --- the outliner, installed here and never committed --------------------------
#
# `svg-outline-stroke` pins an old `sharp` of its own, and that copy has no native
# binary on this Node; it is dropped so the one installed beside it - which does
# load - is used instead.
outliner="$here/.outline"
if [ ! -d "$outliner/node_modules/svg-outline-stroke" ]; then
    mkdir -p "$outliner"
    printf '{"private":true}\n' > "$outliner/package.json"
    (cd "$outliner" && npm install --silent --no-audit --no-fund svg-outline-stroke sharp)
    rm -rf "$outliner/node_modules/svg-outline-stroke/node_modules/sharp"
fi

# --- download, outline, write --------------------------------------------------

raw="$outliner/raw"
rm -rf "$raw"
mkdir -p "$raw" "$here/lucide"

for icon in $icons; do
    # `-f` so a name Lucide does not have fails the run instead of writing an
    # error page into the tree.
    curl -fsSL "$base/$icon.svg" -o "$raw/$icon.svg"
    node "$here/outline.js" "$raw/$icon.svg" "$here/lucide/$icon.svg"
    printf '%s ' "$icon"
done

rm -rf "$raw"
printf '\nvendored and outlined %s icons from lucide-static %s\n' \
    "$(printf '%s' "$icons" | wc -w)" "$version"

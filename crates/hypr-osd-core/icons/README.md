# The icon set

The elements used to wear **Nerd Font glyphs**: a speaker was `\u{f028}` in a
label's text, which meant the desktop needed a patched font installed, an icon
could only be the colour of the label it sat in, and reading an element's source
required knowing which private-use codepoint meant what.

They now draw **Lucide** icons (<https://lucide.dev>, ISC licensed, see
`LICENSE`). Lucide is a set of stroke drawings on a 24x24 grid, which is what GTK
wants for a *symbolic* icon: name the file `…-symbolic.svg`, and GTK paints the
drawing in the widget's CSS `color` - so an icon follows `@accent`, `@fg_dim` and
`@crit` the way the text around it does, and a colour change in
`~/.config/hypr-osd/theme.css` reaches the icons too, with no code involved.

## The one thing that is not Lucide's: the strokes are outlined

**GTK's own SVG engine fills paths and does not draw strokes.** GTK 4.16 stopped
using librsvg, and the replacement is built for the dialect GTK itself ships
(Adwaita): filled contours. A stroke-only drawing therefore arrives as the
*silhouette* of the area the stroke covers - measured on GTK 4.22 with
`cargo run -p hypr-osd-core --example icon-sheet`:

| icon     | Lucide draws        | GTK filled it to   |
| -------- | ------------------- | ------------------ |
| `clock`  | a dial with hands   | a plain disc       |
| `search` | a magnifier         | a dot              |
| `power`  | the power mark      | a blob             |
| `sun`    | a sun with rays     | a dot              |
| `cpu`    | a chip with pins    | a rounded square   |

`fetch-lucide.sh` therefore does two things: it downloads the icon from
`lucide-static`, and it runs it through `outline.js`, which converts the strokes
into filled outlines (Inkscape's algorithm, as `svg-outline-stroke`). The result
is the *same drawing*, expressed the way the engine renders it - and the same
door Lucide's geometry keeps: every icon is still drawn from Lucide's own path
data, at Lucide's own 24x24 grid and 2-unit stroke weight.

The files in `lucide/` are consequently not byte-identical to Lucide's, and each
one says so in a comment on its first line. Re-vendoring is a `sh` script plus
`node` and the network; **building needs none of that**, which is why the
results are committed.

## What is in here

* `lucide/` - the icons, downloaded from `lucide-static` and outlined. Both steps
  are `fetch-lucide.sh`, which is also where the list of names lives; the
  outliner's install lands in `.outline/` and is not committed.
* `local/` - anything we draw ourselves. There is one: `circle-filled.svg`, a
  solid dot for the workspace you are on (Lucide draws outlines only).
* `LICENSE` - Lucide's ISC licence, which covers everything under `lucide/`.
* `outline.js` - the stroke-to-fill step, and the place to look when an icon
  arrives as a silhouette.

## How they get into the binary

`../build.rs` turns this directory into a **GResource** and links it into
`hypr-osd-core`, so the icons are *in the binary*: no icon theme to install, no
files to keep next to the executables, and an element started from a keybinding
cannot find itself without its artwork.

The layout inside the bundle is not ours to choose - GTK's icon theme only looks
in the subdirectories the *hicolor* theme declares, so every icon is placed at

```
/com/schells2/osd/icons/scalable/actions/<name>-symbolic.svg
```

and registered as a resource path on the display's icon theme at start-up (see
`hypr-osd-core/src/icons.rs`). `scalable/actions` is GTK's own naming: the size
is `scalable` because SVG has none, and the category is arbitrary - an icon is
looked up by name, not by the room it sits in.

The `-symbolic` suffix in the *name* is what makes GTK recolour the drawing. The
recolour is by *colour* and not by the file being symbolic alone, which is why
the outlined icons carry `fill="black"` - black is what every colour in a
symbolic icon is replaced with, and it keeps an unregistered resource (or a
mistake) visibly wrong rather than invisibly grey.

## Adding an icon

1. Add the name to the list in `fetch-lucide.sh` and run it (or drop an SVG into
   `local/` yourself).
2. Add it to `names` in `../src/icons.rs` - that module is the vocabulary the
   elements draw from, one constant per icon.
3. Use it with `icons::lucide(names::YOUR_ICON, size)`.

The unit test `the_bundle_ships_every_icon_the_vocabulary_names` fails if step 2
and the bundle ever disagree, so a typo is a test failure rather than a card that
draws GTK's "image missing" placeholder.


// Turn Lucide's stroke drawings into filled outlines.
//
// GTK 4.16 dropped librsvg for its own SVG engine, and that engine is built for
// the dialect GTK itself ships (Adwaita): filled contours. It does not draw
// strokes, so a stroke-only drawing arrives as the *silhouette* of the stroked
// area - Lucide's `clock` (a circle plus two hands) renders as a plain disc, and
// `search` as a solid dot. Measured with `cargo run -p hypr-osd-core --example
// icon-sheet` on GTK 4.22.
//
// So the strokes are outlined once, here, when the icons are vendored: the
// result is the same drawing expressed as fills, which the engine renders
// correctly. Inkscape's algorithm, via `svg-outline-stroke`.
//
// Usage: node outline.js <in.svg> <out.svg>

const fs = require('fs');
const path = require('path');

const module_ = require(path.join(__dirname, '.outline/node_modules/svg-outline-stroke'));
const outline = module_.default || module_;

const [input, output] = process.argv.slice(2);
if (!input || !output) {
    console.error('usage: node outline.js <in.svg> <out.svg>');
    process.exit(2);
}

outline(fs.readFileSync(input, 'utf8'))
    .then((svg) => {
        // The provenance travels *with* the file: these are no longer the bytes
        // Lucide ships, and anyone opening one should know why.
        const name = path.basename(input, '.svg');
        fs.writeFileSync(
            output,
            `<!-- Lucide's \`${name}\`, strokes outlined: see icons/README.md -->\n${svg}`
        );
    })
    .catch((error) => {
        console.error(`${input}: ${error.message}`);
        process.exit(1);
    });

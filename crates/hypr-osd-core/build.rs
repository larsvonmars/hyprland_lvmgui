//! Bake `icons/` into the binary as a GResource.
//!
//! The elements' icons are Lucide SVGs, and they travel *inside*
//! `hypr-osd-core` rather than being looked up on disk - see `icons/README.md`
//! for why, and for what the odd `scalable/actions/...-symbolic.svg` layout is
//! about (it is not ours to choose: it is the only shape GTK's icon theme reads
//! out of a resource path).
//!
//! The manifest is the directory itself. `glib-compile-resources` wants an XML
//! file listing every icon, and that file is *generated here* from whatever is
//! in `icons/lucide` and `icons/local`, so adding an icon is dropping a file in
//! and naming it in the vocabulary (`src/icons.rs`) - there is no second list to
//! keep in step, which is the one thing a hand-written manifest always becomes.
//!
//! `glib-compile-resources` ships with glib2, which is not optional for us:
//! nothing here links without GTK.

use std::collections::BTreeMap;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the icons live inside the process, once linked. The name is the
/// collection's usual app-id root, so a resource path in a stack trace or in
/// `gtk4-icon-browser` says whose it is.
const PREFIX: &str = "/com/schells2/osd/icons";

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let icons = manifest.join("icons");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));

    // Re-run when an icon is added, removed **or edited**: `rerun-if-changed` on
    // a directory only notices the directory's own mtime, so replacing the
    // *contents* of an icon - which is how one is fixed - would otherwise leave
    // a stale bundle in the binary. Every file is named, and the entries are
    // looked up below anyway.
    println!("cargo:rerun-if-changed={}", icons.display());
    println!("cargo:rerun-if-changed=build.rs");

    let entries = aliases(&icons);
    for source in entries.values() {
        println!("cargo:rerun-if-changed={}", source.display());
    }
    let xml = manifest_xml(&entries);
    let xml_path = out.join("lucide.gresource.xml");
    fs::write(&xml_path, xml).expect("write the generated manifest");

    let target = out.join("lucide.gresource");
    // Absolute paths in the XML are what makes `--sourcedir` unnecessary: the
    // manifest is generated in OUT_DIR, where the icons are not.
    let status = Command::new("glib-compile-resources")
        .arg(&xml_path)
        .arg(format!("--target={}", target.display()))
        .status();
    match status {
        Ok(status) if status.success() => {}
        Ok(status) => panic!("glib-compile-resources failed ({status})"),
        Err(error) => panic!("could not run glib-compile-resources: {error}"),
    }
}

/// Every icon, as the alias inside the bundle and the file it comes from.
///
/// Both directories are read the same way, which means the two files that make
/// up the set - the vendored ones and ours - are indistinguishable once they are
/// in the binary. That is the point: an element asks for an icon by name.
fn aliases(icons: &Path) -> BTreeMap<String, PathBuf> {
    let mut entries: BTreeMap<String, PathBuf> = BTreeMap::new();
    for dir in ["lucide", "local"] {
        let dir = icons.join(dir);
        let listing = match fs::read_dir(&dir) {
            Ok(listing) => listing,
            // A directory that is not there is a mistake in *this* file, not a
            // state to tolerate quietly.
            Err(error) => panic!("could not read {}: {error}", dir.display()),
        };
        for entry in listing.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("svg") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let alias = format!("scalable/actions/{stem}-symbolic.svg");
            // Two icons with one name would be a silent winner/loser in the
            // bundle; `local/` shadowing a Lucide icon is exactly the mistake
            // this catches.
            if let Some(other) = entries.insert(alias.clone(), path.clone()) {
                panic!(
                    "{} and {} both claim the icon {stem}",
                    other.display(),
                    path.display()
                );
            }
        }
    }
    assert!(
        !entries.is_empty(),
        "no icons found: run icons/fetch-lucide.sh"
    );
    entries
}

/// The XML `glib-compile-resources` is fed: one `<file>` per icon, in name
/// order so the bundle (and any diff of a generated copy) is stable.
fn manifest_xml(entries: &BTreeMap<String, PathBuf>) -> String {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gresources>\n");
    let _ = writeln!(xml, "  <gresource prefix=\"{PREFIX}\">");
    for (alias, source) in entries {
        let _ = writeln!(
            xml,
            "    <file alias=\"{alias}\">{}</file>",
            source.display()
        );
    }
    xml.push_str("  </gresource>\n</gresources>\n");
    xml
}

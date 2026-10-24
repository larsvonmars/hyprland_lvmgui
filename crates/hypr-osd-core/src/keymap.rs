//! The XKB names *this machine* knows: what the settings window's keyboard rows
//! offer to pick from instead of letting a name be typed in.
//!
//! A layout is a name out of a list the machine owns - `ch`, `de`, `ara`, and
//! `de` again as a *variant of* `ch` - and a name that is not in that list is not
//! a value Hyprland can reject at the door: the option is accepted, xkb then
//! fails to build the keyboard, and what the user sees is a keyboard that does
//! not type. So the window offers the list instead of a text field, and this is
//! where the list comes from.
//!
//! **Where it comes from.** `xkeyboard-config` ships the data xkb (and therefore
//! the compositor) actually uses, and its human-readable half is
//! `/usr/share/X11/xkb/rules/<rules>.lst`: one section per kind of name, each line
//! a name and the description that goes with it -
//!
//! ```text
//! ! layout
//!   ch              German (Switzerland)
//! ! variant
//!   de              ch: German (Switzerland)
//! ```
//!
//! - so this reads that file, in the rules the session's `input:kb_rules` names
//!   (empty means `evdev`, xkb's own default). `localectl
//!   list-x11-keymap-layouts` prints the same names out of the same file, but
//!   without the descriptions and only if systemd is there; the two were checked
//!   name for name against each other, and the descriptions are the whole point
//!   on a list of ninety-nine codes. A machine with no xkeyboard-config cannot
//!   have a keyboard layout at all, so an empty list is the honest answer there -
//!   the settings window falls back to the text field these rows used to be.
//!
//! The file is read once per process: it is 48 KB, a page asks for three
//! sections, and the answer cannot change under a running daemon.
//!
//! **The variants are the one list the menu is not enough for.** It is a *menu* -
//! what `localectl list-x11-keymap-variants` prints - while xkb resolves a variant
//! against `symbols/<layout>`, one `xkb_symbols "…"` block per combination, and
//! the two can differ: on this machine the menu has no `de` variant for `ch`,
//! while `symbols/ch` defines `xkb_symbols "de"` - which is *the* Swiss German
//! keyboard, `kb_layout = ch` with `kb_variant = de`. Calling that pair wrong
//! (which the menu alone would) is worse than offering nothing. So the names come
//! from the symbols file and the menu is what describes them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Where the rules files and the layouts live.
const RULES_DIR: &str = "/usr/share/X11/xkb/rules";
const SYMBOLS_DIR: &str = "/usr/share/X11/xkb/symbols";

/// Which of the four lists a row wants.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Keymap {
    /// The layouts themselves (`input:kb_layout`).
    Layout,
    /// The variants *of the layout in force* (`input:kb_variant`).
    Variant,
    /// The keyboard models (`input:kb_model`).
    Model,
    /// The options (`input:kb_options`).
    Option,
}

impl Keymap {
    /// The section of the rules file this list is read from.
    fn section(self) -> &'static str {
        match self {
            Keymap::Layout => "layout",
            Keymap::Variant => "variant",
            Keymap::Model => "model",
            Keymap::Option => "option",
        }
    }

    /// Whether the list depends on the layout in force - only the variants do,
    /// because a variant belongs to a layout.
    pub fn needs_layout(self) -> bool {
        matches!(self, Keymap::Variant)
    }
}

/// One name the machine knows, with the description that goes with it (`us`,
/// `English (US)`). The description is empty for a name the rules file does not
/// describe.
///
/// The dropdown shows `label · note`, and what is *written* when the entry is
/// picked is `name` - which are the same thing unless a caller has a friendlier
/// spelling for the same value (a monitor mode `2560x1440@59.95` reads better as
/// `2560 × 1440 · 59.95 Hz`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    pub name: String,
    /// What the dropdown shows in place of the name. The same as `name` when
    /// there is nothing friendlier to say.
    pub label: String,
    pub note: String,
}

impl Entry {
    /// A name shown and written as itself, with no description.
    pub fn plain(name: impl Into<String>) -> Self {
        let name = name.into();
        Entry {
            label: name.clone(),
            name,
            note: String::new(),
        }
    }

    /// A name with a description after it (`us`, `English (US)`).
    pub fn described(name: impl Into<String>, note: impl Into<String>) -> Self {
        let name = name.into();
        Entry {
            label: name.clone(),
            name,
            note: note.into(),
        }
    }

    /// A name written as itself but *shown* differently, with an optional
    /// description: the shape a monitor mode or a scale wants.
    pub fn labelled(
        name: impl Into<String>,
        label: impl Into<String>,
        note: impl Into<String>,
    ) -> Self {
        Entry {
            name: name.into(),
            label: label.into(),
            note: note.into(),
        }
    }

    /// The one line a dropdown draws for this entry.
    pub fn display(&self) -> String {
        if self.note.is_empty() {
            self.label.clone()
        } else {
            format!("{} \u{b7} {}", self.label, self.note)
        }
    }
}

/// The rules file a session uses: `evdev.lst` unless `input:kb_rules` names
/// another one.
///
/// The name is used to build a path, so anything that is not a plain file name
/// (a separator, a `..`) falls back to `evdev` rather than being followed: a
/// config value must not be able to point this at `/etc/passwd` - there is
/// nothing to gain from reading it, and a habit worth keeping.
pub fn rules_path(rules: &str) -> PathBuf {
    let wanted = rules.trim();
    let plain = !wanted.is_empty()
        && wanted
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    let name = if plain { wanted } else { "evdev" };
    Path::new(RULES_DIR).join(format!("{name}.lst"))
}

/// The entries of one list, in name order.
///
/// `layout` is the layout in force, which the variants are filtered by; it is
/// ignored by the other kinds. An unreadable file answers with no entries at
/// all, which is what makes the row a text field again.
pub fn entries(kind: Keymap, rules: &str, layout: &str) -> Vec<Entry> {
    if kind == Keymap::Variant {
        return variants(rules, layout);
    }
    let Some(text) = xkb_text(&rules_path(rules)) else {
        return Vec::new();
    };
    parse(&text, kind, layout)
}

/// The variants of one layout: the names xkb resolves, with the menu's own
/// descriptions of them.
fn variants(rules: &str, layout: &str) -> Vec<Entry> {
    let described = xkb_text(&rules_path(rules))
        .map(|text| parse(&text, Keymap::Variant, layout))
        .unwrap_or_default();
    merge(symbols_variants(layout), described)
}

/// A layout's variants as *xkb* has them: every `xkb_symbols "…"` block of its
/// symbols file.
fn symbols_variants(layout: &str) -> Vec<String> {
    let Some(path) = symbols_path(layout) else {
        return Vec::new();
    };
    let Some(text) = xkb_text(&path) else {
        return Vec::new();
    };
    parse_symbols(&text)
}

/// The recipes of the two lists under one roof: everything the symbols file
/// defines, with the description the menu gives that name - plus a name the menu
/// knows and the symbols file does not, which is a variant of an `include`d file
/// and therefore just as usable.
fn merge(defined: Vec<String>, described: Vec<Entry>) -> Vec<Entry> {
    if defined.is_empty() {
        return described;
    }
    let mut out: Vec<Entry> = defined
        .into_iter()
        .map(|name| {
            let note = described
                .iter()
                .find(|entry| entry.name == name)
                .map(|entry| entry.note.clone())
                .unwrap_or_default();
            Entry::described(name, note)
        })
        .collect();
    // `extend` cannot take the filter that reads `out` while it writes it, so the
    // extras are collected first.
    let extras: Vec<Entry> = described
        .into_iter()
        .filter(|entry| !out.iter().any(|known| known.name == entry.name))
        .collect();
    out.extend(extras);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// One file of xkeyboard-config's data, read once per process - see the module
/// comment.
fn xkb_text(path: &Path) -> Option<Rc<String>> {
    CACHE.with(|cache| {
        if let Some(text) = cache.borrow().get(path) {
            return Some(Rc::clone(text));
        }
        // A missing file is *not* remembered: installing xkeyboard-config later
        // should be picked up by the next page, not by a restart.
        let text = Rc::new(std::fs::read_to_string(path).ok()?);
        cache
            .borrow_mut()
            .insert(path.to_path_buf(), Rc::clone(&text));
        Some(text)
    })
}

thread_local! {
    static CACHE: RefCell<HashMap<PathBuf, Rc<String>>> = RefCell::new(HashMap::new());
}

/// The symbols file of one layout: `/usr/share/X11/xkb/symbols/<layout>`, which
/// may live in a subdirectory (`nec_vndr/jp` is a layout).
///
/// `None` for anything that is not a plain path inside that directory - the name
/// comes from a config file, and this must not be able to walk out of it.
fn symbols_path(layout: &str) -> Option<PathBuf> {
    let name = layout.trim();
    let plain = !name.is_empty()
        && name.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '+')
        });
    plain.then(|| Path::new(SYMBOLS_DIR).join(name))
}

/// One section of a rules file.
///
/// Every line is `name  description`, except in the variant section, where the
/// layout a variant belongs to sits in between: `de  ch: German (Switzerland)` is
/// the variant `de` *of* `ch`. That middle field is the whole reason this is not
/// a table lookup - it is what makes "the variants of the layout in force" a
/// question the file can answer.
///
/// The file is sorted by *description* (`al Albanian`, `et Amharic`, `am
/// Armenian`, …), so the result is sorted by name here: a list of codes is much
/// easier to scan - and to diff - that way, and it is the order
/// `localectl list-x11-keymap-layouts` prints in.
fn parse(text: &str, kind: Keymap, layout: &str) -> Vec<Entry> {
    let wanted = kind.section();
    let mut inside = false;
    let mut out: Vec<Entry> = Vec::new();

    for line in text.lines() {
        if let Some(header) = line.strip_prefix('!') {
            // `! layout` - every line up to the next `!` belongs to that section.
            inside = header.trim() == wanted;
            continue;
        }
        if !inside {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [name, rest @ ..] = fields.as_slice() else {
            continue;
        };
        let rest = if kind == Keymap::Variant {
            let [owner, note @ ..] = rest else {
                continue;
            };
            let Some((owner, _)) = owner.split_once(':') else {
                // A variant line with no layout is not an answer to "the variants
                // of <layout>".
                continue;
            };
            if !owner.eq_ignore_ascii_case(layout) {
                continue;
            }
            note
        } else {
            rest
        };
        out.push(Entry::described((*name).to_string(), rest.join(" ")));
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    // Two descriptions for one name (a layout listed twice, say) is not something
    // the file does; keeping the first is more honest than offering it twice.
    out.dedup_by(|a, b| a.name == b.name);
    out
}

/// The variant names in a symbols file, in name order.
///
/// A variant *is* its `xkb_symbols "name"` block, so these are the names xkb can
/// resolve - including the ones the rules menu does not list. Comments are
/// skipped (a commented-out block is not a variant) and the tail of the file is
/// not looked at: a block is declared at the start of a line, which is how every
/// file in `symbols/` is written.
fn parse_symbols(text: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        let Some(rest) = line.strip_prefix("xkb_symbols") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('"') else {
            continue;
        };
        let Some((name, _)) = rest.split_once('"') else {
            continue;
        };
        names.push(name.to_string());
    }
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of the real file, small enough to reason about: the sections in
    /// its order, a variant line's owner in the middle, and the file's own sort
    /// order (by description, which is *not* the order a person wants).
    const SAMPLE: &str = "\
! model
  pc105           Generic 105-key PC
  pc104           Generic 104-key PC
! layout
  al              Albanian
  et              Amharic
  am              Armenian
  us              English (US)
! variant
  ch              de: German (Switzerland)
  us              de: German (US)
  nodeadkeys      de: German (no dead keys)
  de              ch: German (Switzerland)
! option
  grp             Switching to another layout
  grp:switch      Right Alt (while pressed)
";

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    #[test]
    fn a_list_comes_out_sorted_by_name_with_its_descriptions() {
        let layouts = parse(SAMPLE, Keymap::Layout, "");
        assert_eq!(names(&layouts), ["al", "am", "et", "us"]);
        assert_eq!(layouts[0].note, "Albanian");
        assert_eq!(layouts[2].note, "Amharic");
        // A name with no description is not an error - it is an entry with none.
        assert_eq!(
            parse("! layout\n  us\n", Keymap::Model, ""),
            Vec::new(),
            "a section the file does not have has no entries"
        );
    }

    #[test]
    fn variants_belong_to_one_layout() {
        let german = parse(SAMPLE, Keymap::Variant, "de");
        assert_eq!(names(&german), ["ch", "nodeadkeys", "us"]);
        assert_eq!(german[0].note, "German (Switzerland)");
        // The same variant name under another layout is a different entry, and
        // the layout is what decides which one is offered.
        assert_eq!(names(&parse(SAMPLE, Keymap::Variant, "ch")), ["de"]);
        assert_eq!(parse(SAMPLE, Keymap::Variant, "nonsense"), Vec::new());
        assert_eq!(parse(SAMPLE, Keymap::Variant, ""), Vec::new());
    }

    #[test]
    fn the_other_sections_are_flat() {
        let models = parse(SAMPLE, Keymap::Model, "de");
        assert_eq!(names(&models), ["pc104", "pc105"]);
        assert_eq!(models[1].note, "Generic 105-key PC");
        let options = parse(SAMPLE, Keymap::Option, "");
        assert_eq!(names(&options), ["grp", "grp:switch"]);
    }

    #[test]
    fn the_rules_file_is_a_name_and_nothing_else() {
        assert_eq!(rules_path(""), Path::new(RULES_DIR).join("evdev.lst"));
        assert_eq!(rules_path("  "), Path::new(RULES_DIR).join("evdev.lst"));
        assert_eq!(rules_path("base"), Path::new(RULES_DIR).join("base.lst"));
        // A value out of a config file must not be able to walk anywhere.
        for hostile in ["../../etc/passwd", "/etc/passwd", "evdev.lst/..", "a/b"] {
            assert_eq!(
                rules_path(hostile),
                Path::new(RULES_DIR).join("evdev.lst"),
                "`{hostile}` should not be followed"
            );
        }
    }

    /// The shape of a symbols file: one block per variant, and the comments and
    /// includes around them that are not variants.
    const SYMBOLS: &str = "\
// A comment that mentions xkb_symbols \"nonsense\"
default partial alphanumeric_keys
xkb_symbols \"basic\" {
    include \"ch(basic)\"
    name[Group1] = \"Swiss German\";
};

partial alphanumeric_keys
xkb_symbols \"de\" {
    include \"de(basic)\"
    name[Group1] = \"German (Switzerland)\";
};

partial alphanumeric_keys
xkb_symbols \"legacy\" {
    include \"ch(de)\"
    include \"level3(ralt_switch)\"
};

// xkb_symbols \"commented_out\" { }
xkb_symbols \"de\" {
    include \"something(else)\"
};
";

    #[test]
    fn a_variant_is_its_xkb_symbols_block() {
        assert_eq!(parse_symbols(SYMBOLS), ["basic", "de", "legacy"]);
        assert!(parse_symbols("partial alphanumeric_keys\n").is_empty());
    }

    #[test]
    fn the_symbols_file_is_what_decides_and_the_menu_what_describes() {
        // `de` is in both: the menu's description is kept. `basic` is only in the
        // symbols file - offered without a description rather than hidden. And a
        // variant the menu knows but the file does not (it lives in an included
        // file) is a variant too.
        let merged = merge(
            vec!["basic".into(), "de".into()],
            vec![
                entry("de", "German (Switzerland)"),
                entry("fr", "French (Switzerland)"),
            ],
        );
        assert_eq!(names(&merged), ["basic", "de", "fr"]);
        assert_eq!(merged[0].note, "");
        assert_eq!(merged[1].note, "German (Switzerland)");
        assert_eq!(merged[2].note, "French (Switzerland)");
        // Nothing from the symbols file (its file was missing): the menu alone.
        assert_eq!(merge(Vec::new(), vec![entry("fr", "French")]).len(), 1);
    }

    #[test]
    fn a_layout_name_stays_inside_the_symbols_directory() {
        assert_eq!(symbols_path("ch"), Some(Path::new(SYMBOLS_DIR).join("ch")));
        // A layout may live in a subdirectory - `nec_vndr/jp` is one.
        assert_eq!(
            symbols_path("nec_vndr/jp"),
            Some(Path::new(SYMBOLS_DIR).join("nec_vndr/jp"))
        );
        for hostile in ["", "  ", "/etc/passwd", "../../etc/passwd", "a/../b", "ch/"] {
            assert_eq!(symbols_path(hostile), None, "`{hostile}` should be refused");
        }
    }

    fn entry(name: &str, note: &str) -> Entry {
        Entry::described(name, note)
    }
}

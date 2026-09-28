//! Files and folders in the user's own directories.
//!
//! The launcher's second half: `Documents`, `Downloads`, `Desktop`, `Pictures`,
//! `Music`, `Videos`, `Projects`, `Templates`, `Public` - the places a person
//! actually keeps things, and nowhere else. Nothing is indexed and nothing is
//! watched: a query walks those trees with a budget, ranks what it found and
//! throws the rest away.
//!
//! That is a deliberate difference from the project this element replaces. That
//! one kept a SQLite FTS5 index of every file it had ever seen, refreshed by
//! inotify, which is the right answer for millions of files and the wrong answer
//! for a desktop: it costs a database, a watcher and a background thread to make
//! a search that a bounded `read_dir` finishes in single-digit milliseconds
//! (measured on this machine: ~9 000 entries under `~/Documents` and
//! `~/Downloads` in ~8 ms). The walk runs off the main loop anyway - see
//! `main.rs` - so even a cold cache cannot stall a keystroke.

use std::path::{Path, PathBuf};

/// A file or folder the launcher can offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub parent: String,
    pub is_dir: bool,
}

/// Directories we never descend into: large, hidden or noisy, and none of them
/// something a person searches for by name.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    ".cache",
    ".local",
    ".config",
    ".var",
    ".rustup",
    ".cargo",
    ".npm",
    ".nvm",
    ".venv",
    "venv",
    "target",
    "build",
    "dist",
    "__pycache__",
    "snap",
    "Trash",
];

/// Upper bound on candidates collected before ranking. Only a handful are ever
/// shown, so this is what keeps a walk over a huge tree bounded.
const MAX_CANDIDATES: usize = 200;
/// Upper bound on directory entries visited in one walk.
const BUDGET: usize = 30_000;

/// The user-owned directories searched, in `$HOME`.
pub fn search_roots() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let home = PathBuf::from(home);
    [
        "Documents",
        "Downloads",
        "Desktop",
        "Pictures",
        "Music",
        "Videos",
        "Projects",
        "Templates",
        "Public",
    ]
    .iter()
    .map(|dir| home.join(dir))
    .filter(|path| path.is_dir())
    .collect()
}

/// Whether a path is one we never look at: a hidden entry, or one of the heavy
/// development/package directories in [`SKIP_DIRS`].
pub fn is_skipped_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return true;
    };
    name.starts_with('.') || (path.is_dir() && SKIP_DIRS.contains(&name))
}

/// Recursively collect entries whose name contains `query`, staying inside the
/// budget and the candidate cap.
fn collect(dir: &Path, query: &str, out: &mut Vec<FileEntry>, budget: &mut usize) {
    if *budget == 0 || out.len() >= MAX_CANDIDATES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if *budget == 0 || out.len() >= MAX_CANDIDATES {
            return;
        }
        *budget -= 1;
        let path = entry.path();
        if is_skipped_path(&path) {
            continue;
        }
        let is_dir = path.is_dir();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.to_lowercase().contains(query) {
            out.push(FileEntry {
                name,
                path: path.to_string_lossy().into_owned(),
                parent: path
                    .parent()
                    .map(|parent| parent.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                is_dir,
            });
        }
        if is_dir {
            collect(&path, query, out, budget);
        }
    }
}

/// Rank one candidate: an exact name beats a prefix, a prefix beats a substring,
/// and a shallower path beats a deeper one.
///
/// The file extension is ignored so that `report.pdf` counts as an exact match
/// for `report` - which is what the person typed, and the only reason the
/// extension is in the name at all.
pub fn rank(entry: &FileEntry, query: &str) -> u32 {
    let name = entry.name.to_lowercase();
    let stem = name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(&name);
    let score: u32 = if stem == query {
        1000
    } else if stem.starts_with(query) || name.starts_with(query) {
        700
    } else {
        300
    };
    // Depth is a tie-breaker, never enough to outrank a better match.
    score.saturating_sub(entry.path.matches('/').count() as u32 * 5)
}

/// Sort best-first and trim to `limit`.
fn finalize(matches: &mut Vec<FileEntry>, query: &str, limit: usize) {
    matches.sort_by(|a, b| {
        rank(b, query)
            .cmp(&rank(a, query))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    matches.truncate(limit);
}

/// Search the roots for `query`, best first, and answer with at most `limit`
/// entries.
///
/// Runs on its own thread (see `main.rs`): this is filesystem work, and the
/// element's main loop is drawing a card while it happens.
pub fn walk(query: &str, limit: usize) -> Vec<FileEntry> {
    let query = query.trim().to_lowercase();
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut matches = Vec::new();
    let mut budget = BUDGET;
    for root in search_roots() {
        collect(&root, &query, &mut matches, &mut budget);
        if budget == 0 {
            break;
        }
    }
    finalize(&mut matches, &query, limit);
    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A scratch tree with a `Notes` folder and a few files.
    ///
    /// The name is part of the *path*, and that matters: the tests run in
    /// parallel, and one shared directory made them delete each other's trees
    /// (three of the four failed intermittently until each had its own).
    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("hypr-osd-launcher-files-{name}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Notes/2024")).unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::create_dir_all(root.join("node_modules")).unwrap();
        fs::write(root.join("report.pdf"), b"x").unwrap();
        fs::write(root.join("Notes/report-draft.txt"), b"x").unwrap();
        fs::write(root.join("Notes/2024/report-final.txt"), b"x").unwrap();
        fs::write(root.join(".hidden/report-secret.txt"), b"x").unwrap();
        fs::write(root.join("node_modules/report-dep.js"), b"x").unwrap();
        root
    }

    /// The same walk `walk` does, rooted at a scratch tree instead of `$HOME`.
    fn search(root: &Path, query: &str, limit: usize) -> Vec<FileEntry> {
        let query = query.to_lowercase();
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }
        let mut matches = Vec::new();
        let mut budget = BUDGET;
        collect(root, &query, &mut matches, &mut budget);
        finalize(&mut matches, &query, limit);
        matches
    }

    #[test]
    fn finds_nested_matches_and_ranks_the_exact_name_first() {
        let root = scratch("nested");
        let found = search(&root, "report", 10);
        assert_eq!(found.len(), 3, "found: {found:?}");
        // `report.pdf` is the exact stem match; the drafts come after.
        assert_eq!(found[0].name, "report.pdf");
        assert!(found[1].name.starts_with("report-draft"));
        assert!(found[2].name.starts_with("report-final"));
    }

    #[test]
    fn skips_hidden_entries_and_heavy_directories() {
        let root = scratch("hidden");
        let found = search(&root, "report", 10);
        assert!(!found.iter().any(|entry| entry.path.contains(".hidden")));
        assert!(!found
            .iter()
            .any(|entry| entry.path.contains("node_modules")));
    }

    #[test]
    fn the_limit_trims_the_tail() {
        let root = scratch("limit");
        assert_eq!(search(&root, "report", 2).len(), 2);
        assert_eq!(search(&root, "report", 0).len(), 0);
        assert!(search(&root, "", 5).is_empty());
    }

    #[test]
    fn a_file_without_an_extension_can_still_be_a_prefix_match() {
        let root = scratch("extension");
        let found = search(&root, "note", 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Notes");
        assert!(found[0].is_dir);
    }
}

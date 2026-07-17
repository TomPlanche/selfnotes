//! Listing recent entries across the journal and custom folders.
//!
//! The journal lays entries out as `<root>/YYYY/MM/DD.<ext>` and each custom folder holds a flat set of files under its
//! own directory. This module scans those known locations, tags each file with where it came from, and orders the
//! results by modification time so a note can be found without opening the editor.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context as _, Result};
use glob::glob;

use crate::config::{Config, FolderConfig};
use crate::entry;

/// Source label used for built-in journal entries; also the reserved `--folder` value that selects them.
pub const JOURNAL_SOURCE: &str = "journal";

/// A note file discovered while listing.
pub struct Listing {
    /// Absolute path to the entry file.
    pub path: PathBuf,
    /// Where it came from: [`JOURNAL_SOURCE`] or a custom folder's name.
    pub source: String,
    /// Last modification time, used to order the results.
    pub modified: SystemTime,
}

/// The most recently modified entries, newest first, capped at `limit`.
///
/// With `folder` unset, journal entries and every custom folder are scanned. `Some("journal")` restricts to the
/// built-in journal; any other name restricts to that custom folder (and errors if it is not configured).
pub fn recent(config: &Config, folder: Option<&str>, limit: usize) -> Result<Vec<Listing>> {
    let root = config.resolved_journal_root()?;
    let mut listings = Vec::new();

    match folder {
        None => {
            collect_journal(&root, &mut listings);
            for folder in &config.custom_folders {
                collect_folder(config, folder, &mut listings)?;
            }
        },
        Some(JOURNAL_SOURCE) => collect_journal(&root, &mut listings),
        Some(name) => {
            let folder = config
                .folder(name)
                .with_context(|| format!("no folder `{name}` is configured"))?;
            collect_folder(config, folder, &mut listings)?;
        },
    }

    Ok(top_n(listings, limit))
}

/// Collect journal entries laid out as `<root>/YYYY/MM/DD.<ext>`.
fn collect_journal(root: &Path, out: &mut Vec<Listing>) {
    let pattern = format!("{}/[0-9][0-9][0-9][0-9]/[0-9][0-9]/*", escape_dir(root));

    collect_glob(&pattern, JOURNAL_SOURCE, out);
}

/// Collect the flat set of entries under a custom folder's directory.
fn collect_folder(config: &Config, folder: &FolderConfig, out: &mut Vec<Listing>) -> Result<()> {
    let dir = entry::folder_dir(config, folder)?;
    let pattern = format!("{}/*", escape_dir(&dir));

    collect_glob(&pattern, &folder.name, out);

    Ok(())
}

/// Push every regular file matched by `pattern` as a listing tagged with `source`, skipping directories, dotfiles, and
/// files whose modification time cannot be read. A directory that does not exist simply matches nothing.
fn collect_glob(pattern: &str, source: &str, out: &mut Vec<Listing>) {
    let Ok(paths) = glob(pattern) else {
        // An unbuildable pattern (e.g. a root whose bytes escaping cannot rescue) yields nothing rather than failing
        // the whole listing.
        return;
    };

    for path in paths.flatten() {
        if is_dotfile(&path) {
            continue;
        }

        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        // `metadata` follows symlinks, so this also drops a match that resolves to a directory.
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };

        out.push(Listing {
            path,
            source: source.to_owned(),
            modified,
        });
    }
}

/// Escape a directory path so it can be used as the literal prefix of a glob pattern.
fn escape_dir(dir: &Path) -> String {
    glob::Pattern::escape(&dir.to_string_lossy())
}

/// Whether a path's final component begins with a dot.
fn is_dotfile(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

/// Order listings by modification time (newest first) and keep at most `limit`.
fn top_n(mut listings: Vec<Listing>, limit: usize) -> Vec<Listing> {
    // Newest first; ties broken by path so the ordering is deterministic.
    listings.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.path.cmp(&b.path)));
    listings.truncate(limit);

    listings
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn listing(path: &str, modified: SystemTime) -> Listing {
        Listing {
            path: PathBuf::from(path),
            source: JOURNAL_SOURCE.to_owned(),
            modified,
        }
    }

    fn paths(listings: &[Listing]) -> Vec<String> {
        listings
            .iter()
            .map(|listing| listing.path.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn top_n_orders_newest_first_and_caps() {
        let base = SystemTime::UNIX_EPOCH;
        let old = listing("/a.md", base);
        let mid = listing("/b.md", base + Duration::from_secs(10));
        let new = listing("/c.md", base + Duration::from_secs(20));

        let top = top_n(vec![old, new, mid], 2);

        assert_eq!(paths(&top), ["/c.md", "/b.md"]);
    }

    #[test]
    fn top_n_breaks_ties_by_path() {
        let when = SystemTime::UNIX_EPOCH + Duration::from_secs(5);

        let top = top_n(vec![listing("/z.md", when), listing("/a.md", when)], 10);

        assert_eq!(paths(&top), ["/a.md", "/z.md"]);
    }

    #[test]
    fn dotfiles_are_ignored() {
        assert!(is_dotfile(Path::new("/notes/.hidden.md")));
        assert!(!is_dotfile(Path::new("/notes/visible.md")));
    }
}

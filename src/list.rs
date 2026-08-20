//! Listing recent entries across the journal and custom folders, newest first, optionally filtered by tag or status.
//!
//! Enumeration lives in [`crate::notes`]; this module orders the results by modification time and, when a `--tag` or
//! `--status` filter is given, reads each file to keep only the notes that match.

use anyhow::Result;

use crate::config::Config;
use crate::notes::{self, NoteFile};
use crate::status;

/// The most recently modified entries, newest first, capped at `limit`.
///
/// `folder` restricts the sources scanned (see [`crate::notes::walk`]). When `tags` is non-empty, only notes carrying
/// every listed tag are kept; matching is case-insensitive and a requested tag also matches its nested children, so
/// `work` matches `work/project`. When `statuses` is non-empty, only notes whose frontmatter `status` is one of them
/// are kept (see [`crate::status::matches`]).
pub fn recent(
    config: &Config,
    folder: Option<&str>,
    limit: usize,
    tags: &[String],
    statuses: &[String],
) -> Result<Vec<NoteFile>> {
    let files = notes::walk(config, folder)?;
    let files = if tags.is_empty() && statuses.is_empty() {
        files
    } else {
        filter(files, tags, statuses, config.hash_tag_min_len())
    };

    Ok(top_n(files, limit))
}

/// Keep the notes satisfying every requested tag and one of the requested statuses, reading each file to inspect it.
/// Unreadable files are dropped, matching how enumeration tolerates individual failures. `hash_min_len` is forwarded
/// to the tag parser.
fn filter(files: Vec<NoteFile>, tags: &[String], statuses: &[String], hash_min_len: usize) -> Vec<NoteFile> {
    files
        .into_iter()
        .filter(|file| {
            std::fs::read_to_string(&file.path).is_ok_and(|content| {
                let parsed = notes::parse(&content, hash_min_len);

                notes::matches_tags(&parsed.tags, tags) && status::matches(parsed.status.as_deref(), statuses)
            })
        })
        .collect()
}

/// Order notes by modification time (newest first) and keep at most `limit`.
fn top_n(mut files: Vec<NoteFile>, limit: usize) -> Vec<NoteFile> {
    // Newest first; ties broken by path so the ordering is deterministic.
    files.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.path.cmp(&b.path)));
    files.truncate(limit);

    files
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::notes::JOURNAL_SOURCE;

    fn note(path: &str, modified: SystemTime) -> NoteFile {
        NoteFile {
            path: PathBuf::from(path),
            source: JOURNAL_SOURCE.to_owned(),
            modified,
        }
    }

    fn paths(files: &[NoteFile]) -> Vec<String> {
        files
            .iter()
            .map(|file| file.path.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn top_n_orders_newest_first_and_caps() {
        let base = SystemTime::UNIX_EPOCH;
        let old = note("/a.md", base);
        let mid = note("/b.md", base + Duration::from_secs(10));
        let new = note("/c.md", base + Duration::from_secs(20));

        let top = top_n(vec![old, new, mid], 2);

        assert_eq!(paths(&top), ["/c.md", "/b.md"]);
    }

    #[test]
    fn top_n_breaks_ties_by_path() {
        let when = SystemTime::UNIX_EPOCH + Duration::from_secs(5);

        let top = top_n(vec![note("/z.md", when), note("/a.md", when)], 10);

        assert_eq!(paths(&top), ["/a.md", "/z.md"]);
    }
}

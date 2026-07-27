//! Full-text search across note bodies.
//!
//! Enumeration and parsing live in [`crate::notes`]; this module reads each walked note, applies the same `--folder`
//! and `--tag` filters that listing uses, then scans the note's body for the query. Frontmatter is skipped so a search
//! hits prose rather than metadata, and matching lines are grouped into snippets carrying the requested lines of
//! surrounding context.

use anyhow::{Result, bail};

use crate::config::Config;
use crate::notes::{self, NoteFile};

/// What to search for, and over which notes.
#[derive(Debug, Clone, Copy)]
pub struct Query<'a> {
    /// The text to look for. It is matched literally, not as a pattern.
    pub text: &'a str,
    /// Restrict to a single source (see [`crate::notes::walk`]).
    pub folder: Option<&'a str>,
    /// Only search notes carrying every one of these tags (see [`crate::notes::matches_tags`]).
    pub tags: &'a [String],
    /// Match the query's case exactly; otherwise matching is case-insensitive.
    pub case_sensitive: bool,
    /// Lines of surrounding context to keep either side of a matching line.
    pub context: usize,
    /// Maximum number of notes to return.
    pub limit: usize,
}

/// One line of a snippet.
#[derive(Debug, PartialEq, Eq)]
pub struct Line {
    /// 1-based line number within the whole note file, frontmatter included.
    pub number: usize,
    /// The line's text, with trailing whitespace removed.
    pub text: String,
    /// Whether this line matched the query, as opposed to being context around one that did.
    pub matched: bool,
}

/// A run of consecutive lines covering one or more matches. Context windows that overlap or touch are merged, so a
/// note's snippets are always disjoint and separated by at least one elided line.
#[derive(Debug, PartialEq, Eq)]
pub struct Snippet {
    /// The lines to show, in file order.
    pub lines: Vec<Line>,
}

/// A note that matched, with the snippets to show for it.
pub struct Hit {
    /// The file the match was found in.
    pub file: NoteFile,
    /// Human-readable title from the frontmatter, if any.
    pub title: Option<String>,
    /// Every snippet to show, in file order.
    pub snippets: Vec<Snippet>,
}

impl Hit {
    /// How many of the note's lines matched the query.
    pub fn matches(&self) -> usize {
        self.snippets
            .iter()
            .flat_map(|snippet| &snippet.lines)
            .filter(|line| line.matched)
            .count()
    }
}

/// Search the walked notes for `query`, newest first, capped at its `limit`.
///
/// A file that cannot be read is skipped rather than failing the whole search, mirroring how listing and indexing
/// tolerate individual unreadable entries.
pub fn search(config: &Config, query: &Query<'_>) -> Result<Vec<Hit>> {
    let needle = query.text.trim();
    if needle.is_empty() {
        bail!("search query cannot be empty");
    }

    let files = notes::walk(config, query.folder)?;
    let hash_min_len = config.hash_tag_min_len();
    let mut hits = Vec::new();

    for file in files {
        let Ok(content) = std::fs::read_to_string(&file.path) else {
            continue;
        };

        let parsed = notes::parse(&content, hash_min_len);
        if !notes::matches_tags(&parsed.tags, query.tags) {
            continue;
        }

        let (body, first_line) = notes::body(&content);
        let snippets = scan(body, first_line, needle, query);

        if !snippets.is_empty() {
            hits.push(Hit {
                file,
                title: parsed.title,
                snippets,
            });
        }
    }

    // Newest first, as `list` orders entries; ties broken by path so the ordering is deterministic.
    hits.sort_by(|a, b| {
        b.file
            .modified
            .cmp(&a.file.modified)
            .then_with(|| a.file.path.cmp(&b.file.path))
    });
    hits.truncate(query.limit);

    Ok(hits)
}

/// Find `needle` in `body` and group the matching lines, with their context, into snippets.
///
/// `first_line` is the 1-based file line the body starts on, so the numbers reported point into the whole file rather
/// than into the body alone.
fn scan(body: &str, first_line: usize, needle: &str, query: &Query<'_>) -> Vec<Snippet> {
    let lines: Vec<&str> = body.lines().collect();
    // Case-fold the needle once rather than per line.
    let needle = if query.case_sensitive {
        needle.to_owned()
    } else {
        needle.to_lowercase()
    };

    let matched: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| contains(line, &needle, query.case_sensitive))
        .map(|(index, _)| index)
        .collect();

    windows(&matched, query.context, lines.len())
        .into_iter()
        .map(|(start, end)| Snippet {
            lines: (start..=end)
                .map(|index| Line {
                    number: first_line + index,
                    text: lines[index].trim_end().to_owned(),
                    // `matched` is ascending, so it can be probed directly.
                    matched: matched.binary_search(&index).is_ok(),
                })
                .collect(),
        })
        .collect()
}

/// Whether `line` contains `needle`, which the caller has already lowercased when matching case-insensitively.
fn contains(line: &str, needle: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        line.contains(needle)
    } else {
        line.to_lowercase().contains(needle)
    }
}

/// The inclusive line-index ranges to show: every matching line widened by `context` on each side, clamped to `len`
/// lines, with overlapping or touching windows merged so no snippet elides a single line.
fn windows(matched: &[usize], context: usize, len: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();

    for &index in matched {
        let start = index.saturating_sub(context);
        let end = (index + context).min(len.saturating_sub(1));

        match out.last_mut() {
            Some(last) if start <= last.1 + 1 => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A query over `text` with everything else at its default: case-insensitive, no context, no filters.
    fn query(text: &str) -> Query<'_> {
        Query {
            text,
            folder: None,
            tags: &[],
            case_sensitive: false,
            context: 0,
            limit: 10,
        }
    }

    /// The `(number, matched, text)` triples of every line in every snippet, flattened for compact assertions.
    fn shown(snippets: &[Snippet]) -> Vec<(usize, bool, &str)> {
        snippets
            .iter()
            .flat_map(|snippet| &snippet.lines)
            .map(|line| (line.number, line.matched, line.text.as_str()))
            .collect()
    }

    #[test]
    fn matches_are_case_insensitive_by_default() {
        let body = "Nothing here.\nA Meeting about it.\n";

        assert_eq!(
            shown(&scan(body, 1, "meeting", &query("meeting"))),
            [(2, true, "A Meeting about it.")]
        );

        // With `--case-sensitive`, the differing case no longer matches.
        let exact = Query {
            case_sensitive: true,
            ..query("meeting")
        };
        assert!(scan(body, 1, "meeting", &exact).is_empty());
    }

    #[test]
    fn line_numbers_account_for_frontmatter() {
        let content = "+++\ntags = [\"work\"]\n+++\n\nthe needle is here\n";
        let (body, first_line) = notes::body(content);

        // The body starts on line 4, so the match is on line 5 of the file.
        assert_eq!(
            shown(&scan(body, first_line, "needle", &query("needle"))),
            [(5, true, "the needle is here")]
        );
    }

    #[test]
    fn context_lines_surround_the_match() {
        let body = "one\ntwo\nthree\nfour\nfive\n";
        let with_context = Query {
            context: 1,
            ..query("three")
        };

        assert_eq!(
            shown(&scan(body, 1, "three", &with_context)),
            [(2, false, "two"), (3, true, "three"), (4, false, "four"),]
        );
    }

    #[test]
    fn context_is_clamped_at_the_edges() {
        let body = "hit\nafter\n";
        let with_context = Query {
            context: 3,
            ..query("hit")
        };

        // The window cannot run past either end of the body.
        assert_eq!(
            shown(&scan(body, 1, "hit", &with_context)),
            [(1, true, "hit"), (2, false, "after")]
        );
    }

    #[test]
    fn nearby_matches_share_one_snippet_and_distant_ones_do_not() {
        // Matches on lines 1 and 3 have touching context windows; the one on line 9 stands alone.
        let body = "hit\nfiller\nhit\na\nb\nc\nd\ne\nhit\n";
        let with_context = Query {
            context: 1,
            ..query("hit")
        };

        let snippets = scan(body, 1, "hit", &with_context);

        assert_eq!(snippets.len(), 2);
        assert_eq!(
            shown(&snippets[..1]),
            [
                (1, true, "hit"),
                (2, false, "filler"),
                (3, true, "hit"),
                (4, false, "a"),
            ]
        );
        assert_eq!(shown(&snippets[1..]), [(8, false, "e"), (9, true, "hit")]);
    }

    #[test]
    fn a_line_matching_twice_is_reported_once() {
        let hit = Hit {
            file: NoteFile {
                path: std::path::PathBuf::from("/n/a.md"),
                source: notes::JOURNAL_SOURCE.to_owned(),
                modified: std::time::SystemTime::UNIX_EPOCH,
            },
            title: None,
            snippets: scan(
                "needle and needle again\nplain\nneedle\n",
                1,
                "needle",
                &query("needle"),
            ),
        };

        // Counting is per line, not per occurrence.
        assert_eq!(hit.matches(), 2);
    }

    #[test]
    fn trailing_whitespace_is_trimmed_from_shown_lines() {
        assert_eq!(
            shown(&scan("needle here   \n", 1, "needle", &query("needle"))),
            [(1, true, "needle here")]
        );
    }
}

//! Enumerating note files and extracting their tags and wikilinks.
//!
//! Notes are plain text on disk, so tags and links are text conventions embedded in each file rather than a separate
//! database: `#tag` hashtags in the body, an optional `+++`-delimited TOML frontmatter carrying `tags = [...]` (plus a
//! human `title` and `aliases` a note can be addressed by), and `[[note-name]]` wikilinks between notes. This module
//! walks the same journal and custom-folder locations that listing does, then parses those conventions so the `tags`,
//! `links`, and `open` commands (and `list --tag`) can query them.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context as _, Result};
use glob::glob;
use serde::Deserialize;

use crate::config::{Config, FolderConfig};
use crate::entry;

/// Source label used for built-in journal entries; also the reserved `--folder` value that selects them.
pub const JOURNAL_SOURCE: &str = "journal";

/// A note file discovered while walking the journal and custom folders.
pub struct NoteFile {
    /// Absolute path to the entry file.
    pub path: PathBuf,
    /// Where it came from: [`JOURNAL_SOURCE`] or a custom folder's name.
    pub source: String,
    /// Last modification time, used to order listings.
    pub modified: SystemTime,
}

/// A `[[wikilink]]` found in a note body.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Link {
    /// The link target as written: a note name, optionally `folder/name`, optionally with an extension.
    pub target: String,
    /// Display text from a `[[target|display]]` alias, if given.
    pub display: Option<String>,
}

/// A note together with the tags and links parsed from its contents.
pub struct IndexedNote {
    /// The file this note was read from.
    pub file: NoteFile,
    /// Tags gathered from the frontmatter and the body, de-duplicated in first-seen order.
    pub tags: Vec<String>,
    /// Outbound wikilinks, de-duplicated in first-seen order.
    pub links: Vec<Link>,
    /// Human-readable title from the frontmatter `title`, if any. A note can be linked to (and opened) by this.
    pub title: Option<String>,
    /// Alternative names from the frontmatter `aliases`, each also usable to link to (and open) the note.
    pub aliases: Vec<String>,
}

/// An in-memory view of every scanned note, backing the tag, link, and open queries.
pub struct Index {
    /// Every note that could be read, in walk order.
    pub notes: Vec<IndexedNote>,
}

/// Walk the note files under the journal root, optionally restricted to one source.
///
/// With `folder` unset, journal entries and every custom folder are scanned. `Some("journal")` restricts to the
/// built-in journal; any other name restricts to that custom folder (and errors if it is not configured).
pub fn walk(config: &Config, folder: Option<&str>) -> Result<Vec<NoteFile>> {
    let root = config.resolved_journal_root()?;
    let mut out = Vec::new();

    match folder {
        None => {
            collect_journal(&root, &mut out);
            for folder in &config.custom_folders {
                collect_folder(config, folder, &mut out)?;
            }
        },
        Some(JOURNAL_SOURCE) => collect_journal(&root, &mut out),
        Some(name) => {
            let folder = config
                .folder(name)
                .with_context(|| format!("no folder `{name}` is configured"))?;
            collect_folder(config, folder, &mut out)?;
        },
    }

    Ok(out)
}

/// Build an [`Index`] by walking the notes and parsing each file's tags and links.
///
/// A file that cannot be read is skipped rather than failing the whole query, mirroring how listing tolerates
/// individual unreadable entries.
pub fn build_index(config: &Config, folder: Option<&str>) -> Result<Index> {
    let files = walk(config, folder)?;
    let hash_min_len = config.hash_tag_min_len();
    let mut notes = Vec::with_capacity(files.len());

    for file in files {
        let Ok(content) = std::fs::read_to_string(&file.path) else {
            continue;
        };

        let parsed = parse(&content, hash_min_len);
        notes.push(IndexedNote {
            file,
            tags: parsed.tags,
            links: parsed.links,
            title: parsed.title,
            aliases: parsed.aliases,
        });
    }

    Ok(Index { notes })
}

impl Index {
    /// The number of notes using each tag, keyed by tag name (alphabetical via the map).
    pub fn tag_counts(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();

        for note in &self.notes {
            for tag in &note.tags {
                *counts.entry(tag.clone()).or_default() += 1;
            }
        }

        counts.into_iter().collect()
    }

    /// Every note a `[[target]]` could refer to. A note matches (case-insensitively) on its filename stem, its
    /// frontmatter `title`, or any of its `aliases`; when the target is `folder/name`, the source must match too.
    pub fn resolve(&self, target: &str) -> Vec<&IndexedNote> {
        let (folder, name) = split_target(target);

        self.notes
            .iter()
            .filter(|note| note_matches(note, folder.as_deref(), &name))
            .collect()
    }

    /// Notes that link to the file at `target`, i.e. its backlinks.
    pub fn backlinks(&self, target: &Path) -> Vec<&IndexedNote> {
        self.notes
            .iter()
            .filter(|note| {
                note.file.path != target
                    && note
                        .links
                        .iter()
                        .any(|link| self.resolve(&link.target).iter().any(|hit| hit.file.path == target))
            })
            .collect()
    }
}

/// Collect journal entries laid out as `<root>/YYYY/MM/DD.<ext>`.
fn collect_journal(root: &Path, out: &mut Vec<NoteFile>) {
    let pattern = format!("{}/[0-9][0-9][0-9][0-9]/[0-9][0-9]/*", escape_dir(root));

    collect_glob(&pattern, JOURNAL_SOURCE, out);
}

/// Collect the flat set of entries under a custom folder's directory.
fn collect_folder(config: &Config, folder: &FolderConfig, out: &mut Vec<NoteFile>) -> Result<()> {
    let dir = entry::folder_dir(config, folder)?;
    let pattern = format!("{}/*", escape_dir(&dir));

    collect_glob(&pattern, &folder.name, out);

    Ok(())
}

/// Push every regular file matched by `pattern` as a note tagged with `source`, skipping directories, dotfiles, and
/// files whose modification time cannot be read. A directory that does not exist simply matches nothing.
fn collect_glob(pattern: &str, source: &str, out: &mut Vec<NoteFile>) {
    let Ok(paths) = glob(pattern) else {
        // An unbuildable pattern (e.g. a root whose bytes cannot be escaped) yields nothing rather than failing the
        // whole walk.
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

        out.push(NoteFile {
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

/// Everything parsed out of a single note.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    /// Tags from the frontmatter `tags` array and inline `#tag`s, de-duplicated in first-seen order.
    pub tags: Vec<String>,
    /// Outbound wikilinks, de-duplicated in first-seen order.
    pub links: Vec<Link>,
    /// Frontmatter `title`, trimmed; `None` when unset or blank.
    pub title: Option<String>,
    /// Frontmatter `aliases`, trimmed, with blanks dropped.
    pub aliases: Vec<String>,
}

/// Parse a note's tags, wikilinks, and frontmatter title/aliases.
///
/// `hash_min_len` is the length at or above which an all-hexadecimal inline `#token` is treated as a git hash and
/// skipped rather than collected as a tag; `0` disables that heuristic. It never affects frontmatter tags, which are
/// always explicit.
pub fn parse(content: &str, hash_min_len: usize) -> Parsed {
    let (frontmatter, body) = split_frontmatter(content);
    let meta = frontmatter.map(frontmatter_meta).unwrap_or_default();

    let mut tags = meta.tags;
    let mut links = Vec::new();
    scan_body(body, hash_min_len, &mut tags, &mut links);

    dedup(&mut tags);
    dedup(&mut links);

    Parsed {
        tags,
        links,
        title: meta.title,
        aliases: meta.aliases,
    }
}

/// All tags from a note: the frontmatter `tags` array plus every inline `#tag`, de-duplicated. See [`parse`] for
/// `hash_min_len`.
pub fn extract_tags(content: &str, hash_min_len: usize) -> Vec<String> {
    parse(content, hash_min_len).tags
}

/// Split a leading `+++`-delimited TOML frontmatter block from the body.
///
/// The frontmatter must open on the very first line (`+++`) and close on a later line that is exactly `+++`. Anything
/// else (no fence, or an unterminated one) is treated as having no frontmatter, and the whole input is the body.
fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    let mut lines = content.split_inclusive('\n');

    let Some(first) = lines.next() else {
        return (None, content);
    };
    if first.trim_end() != "+++" {
        return (None, content);
    }

    let fm_start = first.len();
    let mut offset = fm_start;

    for line in lines {
        if line.trim_end() == "+++" {
            let fm = &content[fm_start..offset];
            let body = &content[offset + line.len()..];

            return (Some(fm), body);
        }

        offset += line.len();
    }

    // Opening fence with no close: not valid frontmatter, so keep everything as the body.
    (None, content)
}

/// The frontmatter fields the index cares about: tags, title, and aliases.
#[derive(Default)]
struct FrontMatterMeta {
    tags: Vec<String>,
    title: Option<String>,
    aliases: Vec<String>,
}

/// Read `tags`, `title`, and `aliases` out of a TOML frontmatter block, ignoring other keys and tolerating a parse
/// error (returns defaults).
fn frontmatter_meta(fm: &str) -> FrontMatterMeta {
    #[derive(Deserialize)]
    struct FrontMatter {
        #[serde(default)]
        tags: Vec<String>,
        title: Option<String>,
        #[serde(default)]
        aliases: Vec<String>,
    }

    let Ok(parsed) = toml::from_str::<FrontMatter>(fm) else {
        return FrontMatterMeta::default();
    };

    FrontMatterMeta {
        tags: parsed.tags.iter().filter_map(|tag| normalize_tag(tag)).collect(),
        title: parsed.title.and_then(|title| normalize_name(&title)),
        aliases: parsed
            .aliases
            .iter()
            .filter_map(|alias| normalize_name(alias))
            .collect(),
    }
}

/// Trim a title or alias, returning `None` when nothing is left. Unlike a tag, it keeps its inner punctuation and case.
fn normalize_name(name: &str) -> Option<String> {
    let name = name.trim();

    (!name.is_empty()).then(|| name.to_owned())
}

/// Normalize a written tag: trim it, drop a leading `#` and any trailing `/`, and discard it if nothing remains.
fn normalize_tag(tag: &str) -> Option<String> {
    let tag = tag.trim().trim_start_matches('#').trim_end_matches('/');

    (!tag.is_empty()).then(|| tag.to_string())
}

/// Whether `tag` looks like a git commit hash: entirely hexadecimal and at least `min_len` characters (with `min_len`
/// of `0` disabling the check). A nested tag keeps its `/`, so it is never all-hex and never treated as a hash.
fn is_hash_like(tag: &str, min_len: usize) -> bool {
    min_len != 0 && tag.len() >= min_len && tag.chars().all(|c| c.is_ascii_hexdigit())
}

/// Ensure the note `content` carries every tag in `tags` in a leading `+++` TOML frontmatter block.
///
/// Applied to the raw template before rendering, so any `{{cursor}}` position downstream stays correct. With no tags,
/// the content is returned unchanged. When the content already opens with a parseable frontmatter, the tags are merged
/// into its `tags` array (existing entries first, new ones appended); otherwise a fresh frontmatter block is prepended.
/// A leading frontmatter that does not parse as TOML is left untouched rather than risk corrupting it.
pub fn ensure_frontmatter_tags(content: &str, tags: &[String]) -> String {
    if tags.is_empty() {
        return content.to_owned();
    }

    match split_frontmatter(content) {
        (Some(frontmatter), body) => {
            merge_frontmatter_tags(frontmatter, body, tags).unwrap_or_else(|| content.to_owned())
        },
        (None, _) => prepend_frontmatter_tags(content, tags),
    }
}

/// Serialize a `tags` array as a one-line TOML assignment, e.g. `tags = ["a", "b"]\n`.
fn serialize_tags(tags: &[String]) -> String {
    #[derive(serde::Serialize)]
    struct FrontMatter<'a> {
        tags: &'a [String],
    }

    // A plain string array cannot fail to serialize; fall back to an empty block rather than panicking if it ever does.
    toml::to_string(&FrontMatter { tags }).unwrap_or_default()
}

/// Prepend a fresh frontmatter block carrying `tags` to `content`.
fn prepend_frontmatter_tags(content: &str, tags: &[String]) -> String {
    format!("+++\n{}+++\n\n{content}", serialize_tags(tags))
}

/// Merge `tags` into an existing frontmatter's `tags` array and rebuild the note, or `None` if the block is not TOML.
fn merge_frontmatter_tags(frontmatter: &str, body: &str, tags: &[String]) -> Option<String> {
    let mut table: toml::Table = frontmatter.parse().ok()?;

    let mut merged: Vec<String> = table
        .get("tags")
        .and_then(toml::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();

    for tag in tags {
        if !merged.contains(tag) {
            merged.push(tag.clone());
        }
    }

    table.insert(
        "tags".to_owned(),
        toml::Value::Array(merged.into_iter().map(toml::Value::String).collect()),
    );

    let serialized = toml::to_string(&table).ok()?;

    Some(format!("+++\n{serialized}+++\n{body}"))
}

/// Scan a note body line by line, collecting inline `#tag`s and `[[links]]` while skipping fenced code blocks.
fn scan_body(body: &str, hash_min_len: usize, tags: &mut Vec<String>, links: &mut Vec<Link>) {
    let mut in_fence = false;

    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;

            continue;
        }
        if in_fence {
            continue;
        }

        scan_line(line, hash_min_len, tags, links);
    }
}

/// Scan one line for tags and links, treating backtick-delimited inline code spans as opaque.
fn scan_line(line: &str, hash_min_len: usize, tags: &mut Vec<String>, links: &mut Vec<Link>) {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        // Inline code span: a run of N backticks is opaque until the next run of exactly N backticks.
        if c == '`' {
            let start = i;
            while i < chars.len() && chars[i] == '`' {
                i += 1;
            }

            if let Some(close) = backtick_close(&chars, i, i - start) {
                i = close;
            }
            // No closing run: the backticks were literal, so just carry on past them.
            continue;
        }

        // Wikilink `[[target]]` or `[[target|display]]`.
        if c == '['
            && chars.get(i + 1) == Some(&'[')
            && let Some(end) = find_pair(&chars, i + 2, ']')
        {
            let inner: String = chars[i + 2..end].iter().collect();
            if let Some(link) = parse_link(&inner) {
                links.push(link);
            }

            i = end + 2;
            continue;
        }

        // Inline `#tag`: must sit at a word boundary (start of line or after whitespace) and start with a letter or
        // underscore, so markdown headings (`# Title`) and URL fragments (`page#anchor`) are not mistaken for tags.
        if c == '#' {
            let boundary = i == 0 || chars[i - 1].is_whitespace();

            if boundary && chars.get(i + 1).is_some_and(|&next| is_tag_start(next)) {
                let mut j = i + 1;
                while j < chars.len() && is_tag_continue(chars[j]) {
                    j += 1;
                }

                let raw: String = chars[i + 1..j].iter().collect();
                // Skip git hashes: an all-hex token at or above the configured length is a commit code, not a tag.
                if let Some(tag) = normalize_tag(&raw)
                    && !is_hash_like(&tag, hash_min_len)
                {
                    tags.push(tag);
                }

                i = j;
                continue;
            }
        }

        i += 1;
    }
}

/// From just past an opening run of `n` backticks, find the index right after the next run of exactly `n` backticks.
const fn backtick_close(chars: &[char], mut i: usize, n: usize) -> Option<usize> {
    while i < chars.len() {
        if chars[i] == '`' {
            let start = i;
            while i < chars.len() && chars[i] == '`' {
                i += 1;
            }

            if i - start == n {
                return Some(i);
            }
        } else {
            i += 1;
        }
    }

    None
}

/// Find the start index of the first `cc` pair at or after `from`.
fn find_pair(chars: &[char], from: usize, c: char) -> Option<usize> {
    (from..chars.len().saturating_sub(1)).find(|&i| chars[i] == c && chars[i + 1] == c)
}

/// Parse the inside of a `[[...]]` into a [`Link`], returning `None` when the target is empty.
fn parse_link(inner: &str) -> Option<Link> {
    let mut parts = inner.splitn(2, '|');

    let target = parts.next()?.trim().to_string();
    if target.is_empty() {
        return None;
    }

    let display = parts
        .next()
        .map(|display| display.trim().to_string())
        .filter(|display| !display.is_empty());

    Some(Link { target, display })
}

/// Whether `c` may begin a tag (after the `#`).
fn is_tag_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

/// Whether `c` may continue a tag.
fn is_tag_continue(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '/')
}

/// Split a link target into an optional (lowercased) folder prefix and the trailing name to match on.
///
/// Extension stripping is deferred to [`note_matches`], since it only applies to the filename comparison, not to a
/// `title`/`alias` (which may legitimately contain dots).
fn split_target(target: &str) -> (Option<String>, String) {
    let target = target.trim();

    match target.rsplit_once('/') {
        Some((folder, name)) => (Some(folder.trim().to_lowercase()), name.trim().to_owned()),
        None => (None, target.to_owned()),
    }
}

/// Whether a note matches a resolved target: its filename stem, frontmatter `title`, or any alias equals `name`
/// (case-insensitively), and, when a folder is given, its source matches too.
fn note_matches(note: &IndexedNote, folder: Option<&str>, name: &str) -> bool {
    if let Some(folder) = folder
        && note.file.source.to_lowercase() != folder
    {
        return false;
    }

    let name = name.to_lowercase();

    let file_stem = note
        .file
        .path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_lowercase();
    // Strip an extension from the target only for the filename comparison, so `[[note.md]]` still matches `note`.
    let name_stem = Path::new(&name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(&name);

    file_stem == name_stem
        || note.title.as_deref().is_some_and(|title| title.to_lowercase() == name)
        || note.aliases.iter().any(|alias| alias.to_lowercase() == name)
}

/// Drop later duplicates, keeping the first occurrence of each value.
fn dedup<T: Clone + Eq + std::hash::Hash>(items: &mut Vec<T>) {
    let mut seen = HashSet::new();

    items.retain(|item| seen.insert(item.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Default hash-tag threshold, so parsing tests exercise the shipped behaviour.
    const HASH_MIN: usize = crate::config::DEFAULT_HASH_TAG_MIN_LEN;

    #[test]
    fn extracts_inline_and_nested_tags() {
        let tags = extract_tags("Fixed it today. #work #bug/auth and #work again.\n", HASH_MIN);

        // `#work` is de-duplicated; the nested tag keeps its full path.
        assert_eq!(tags, ["work", "bug/auth"]);
    }

    #[test]
    fn heading_and_fragment_are_not_tags() {
        // `# Heading` has a space after `#`; `page#anchor` has no word boundary before it.
        assert!(extract_tags("# Heading\n\nSee https://example.com/page#anchor here.\n", HASH_MIN).is_empty());
    }

    #[test]
    fn tags_inside_code_are_ignored() {
        let content = "Before #real\n\n```\n# not a heading, #notatag\n```\n\nInline `#alsoskipped` here.\n";

        assert_eq!(extract_tags(content, HASH_MIN), ["real"]);
    }

    #[test]
    fn git_hashes_are_not_tags() {
        // Abbreviated (starting a-f, so they reach the tag scanner) and full-length all-hex tokens are skipped, while a
        // real word that merely starts hex-like, and a nested tag with a hex segment, are kept.
        let content = "See #deadbeef and #a1b2c3d and #cafebabecafebabecafebabecafebabecafebabe.\n#decadent thoughts about #work/beef.\n";

        assert_eq!(extract_tags(content, HASH_MIN), ["decadent", "work/beef"]);
    }

    #[test]
    fn hash_heuristic_can_be_disabled() {
        // With the threshold at 0, an all-hex token is a perfectly ordinary tag again.
        assert_eq!(extract_tags("#deadbeef\n", 0), ["deadbeef"]);
    }

    #[test]
    fn frontmatter_tags_merge_with_inline() {
        let content = "+++\ntags = [\"work\", \"#bug/auth\"]\ntitle = \"x\"\n+++\n\nBody with #idea and #work.\n";

        // Frontmatter first (with the leading `#` stripped), then new inline tags; `work` is not repeated. Frontmatter
        // tags are explicit, so the hash heuristic never touches them.
        assert_eq!(extract_tags(content, HASH_MIN), ["work", "bug/auth", "idea"]);
    }

    #[test]
    fn no_frontmatter_when_fence_is_absent_or_unterminated() {
        assert_eq!(split_frontmatter("# Title\n#tag\n").0, None);
        assert_eq!(split_frontmatter("+++\ntags = []\n\nbody without a close\n").0, None);
    }

    #[test]
    fn extracts_links_with_alias_and_folder() {
        let links = parse(
            "See [[login-bug]] and [[ticket/PROJ-1|the ticket]] plus [[login-bug]] again.\n",
            HASH_MIN,
        )
        .links;

        assert_eq!(
            links,
            [
                Link {
                    target: "login-bug".into(),
                    display: None,
                },
                Link {
                    target: "ticket/PROJ-1".into(),
                    display: Some("the ticket".into()),
                },
            ]
        );
    }

    #[test]
    fn links_inside_code_are_ignored() {
        let content = "Real [[note-a]].\n\n```\n[[not-a-link]]\n```\n\nInline `[[skip]]` too.\n";

        assert_eq!(
            parse(content, HASH_MIN).links,
            [Link {
                target: "note-a".into(),
                display: None,
            }]
        );
    }

    #[test]
    fn empty_link_target_is_ignored() {
        assert!(parse("[[]] and [[  |only display]]\n", HASH_MIN).links.is_empty());
    }

    #[test]
    fn parses_frontmatter_title_and_aliases() {
        let content =
            "+++\ntitle = \"Login bug investigation\"\naliases = [\"login-bug\", \" PROJ-1 \", \"\"]\n+++\n\nbody\n";

        let parsed = parse(content, HASH_MIN);

        // Title is trimmed; aliases are trimmed and blanks dropped.
        assert_eq!(parsed.title.as_deref(), Some("Login bug investigation"));
        assert_eq!(parsed.aliases, ["login-bug", "PROJ-1"]);
    }

    #[test]
    fn ensure_tags_is_a_no_op_without_defaults() {
        let content = "# {{date}}\n\nbody\n";

        assert_eq!(ensure_frontmatter_tags(content, &[]), content);
    }

    #[test]
    fn ensure_tags_prepends_frontmatter_when_absent() {
        let out = ensure_frontmatter_tags("# {{date}}\n\nbody\n", &["daily".into()]);

        assert_eq!(out, "+++\ntags = [\"daily\"]\n+++\n\n# {{date}}\n\nbody\n");
        // The seeded tag round-trips through the parser.
        assert_eq!(extract_tags(&out, HASH_MIN), ["daily"]);
    }

    #[test]
    fn ensure_tags_merges_into_existing_frontmatter() {
        let content = "+++\ntitle = \"scrum\"\ntags = [\"standup\"]\n+++\n\n# notes\n";

        let out = ensure_frontmatter_tags(content, &["daily".into(), "standup".into()]);

        // The existing tag and key survive, `daily` is added once, and the result still parses.
        let tags = extract_tags(&out, HASH_MIN);
        assert_eq!(tags, ["standup", "daily"]);
        assert!(out.contains("title = \"scrum\""));
        assert!(out.trim_start().starts_with("+++"));
    }

    fn indexed(path: &str, source: &str, links: &[&str]) -> IndexedNote {
        IndexedNote {
            file: NoteFile {
                path: PathBuf::from(path),
                source: source.to_owned(),
                modified: SystemTime::UNIX_EPOCH,
            },
            tags: Vec::new(),
            links: links
                .iter()
                .map(|target| Link {
                    target: (*target).to_string(),
                    display: None,
                })
                .collect(),
            title: None,
            aliases: Vec::new(),
        }
    }

    #[test]
    fn resolve_matches_by_stem_case_insensitively() {
        let index = Index {
            notes: vec![
                indexed("/n/ideas/Login-Bug.md", "ideas", &[]),
                indexed("/n/tickets/login-bug.md", "tickets", &[]),
            ],
        };

        // Bare name is ambiguous across folders.
        assert_eq!(index.resolve("login-bug").len(), 2);
        // A folder qualifier (and an extension) narrows it to one.
        let hits = index.resolve("tickets/login-bug.md");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file.source, "tickets");
    }

    #[test]
    fn resolve_matches_by_title_and_alias() {
        let mut note = indexed("/n/tickets/PROJ-1.md", "tickets", &[]);
        note.title = Some("Login bug investigation".into());
        note.aliases = vec!["login-bug".into()];
        let index = Index { notes: vec![note] };

        // Filename stem still resolves.
        assert_eq!(index.resolve("proj-1").len(), 1);
        // Title matches case-insensitively despite spaces.
        assert_eq!(index.resolve("login bug INVESTIGATION").len(), 1);
        // An alias matches, including with a folder qualifier.
        assert_eq!(index.resolve("login-bug").len(), 1);
        assert_eq!(index.resolve("tickets/login-bug").len(), 1);
        // A different folder does not match.
        assert!(index.resolve("ideas/login-bug").is_empty());
        // An unrelated name matches nothing.
        assert!(index.resolve("nope").is_empty());
    }

    #[test]
    fn backlinks_find_notes_pointing_at_a_file() {
        let target = PathBuf::from("/n/ideas/login-bug.md");
        let index = Index {
            notes: vec![
                indexed("/n/ideas/login-bug.md", "ideas", &[]),
                indexed("/n/ideas/other.md", "ideas", &["login-bug"]),
                indexed("/n/ideas/unrelated.md", "ideas", &["something-else"]),
            ],
        };

        let backlinks = index.backlinks(&target);

        assert_eq!(backlinks.len(), 1);
        assert_eq!(backlinks[0].file.path, PathBuf::from("/n/ideas/other.md"));
    }
}

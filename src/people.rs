//! The people roster behind `@handle` mentions.
//!
//! Like tags and wikilinks, a mention is a text convention rather than a database: notes carry plain `@handle` text,
//! and this module holds the roster that gives those handles a name, a team and an address. The roster lives in a
//! `people.toml` next to the global config, or wherever `people_file` points, so a path-scoped `[[overrides]]` entry
//! can swap in a different roster per project. The `lsp` command reads it to complete and describe mentions as you
//! type them.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::config::{self, Config};

/// File name of the roster resolved next to the global config when `people_file` is unset.
pub const PEOPLE_FILE_NAME: &str = "people.toml";

/// Seed written by `selfnotes people open` when no roster exists yet.
pub const TEMPLATE: &str = "\
# People you mention as `@handle` in your notes.
#
# `selfnotes lsp` serves these as completions after an `@`, so a handle is what
# you type: no spaces, no `@`. Everything but `handle` is optional and only
# feeds the completion's description.

# [[people]]
# handle  = \"jdoe\"
# name    = \"Jane Doe\"
# email   = \"jane.doe@example.com\"
# team    = \"backend\"
# role    = \"Tech lead\"
# aliases = [\"jane\"]
";

/// A named link attached to a person: their chat thread, their profile page, whatever you want one keystroke away.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Link {
    /// Label shown in the hover popup. Defaults to the URL's host.
    pub name: Option<String>,
    pub url: String,
}

impl Link {
    /// What to call this link on screen: its name, else the host it points at, else the whole URL.
    pub fn label(&self) -> &str {
        if let Some(name) = self.name.as_deref() {
            return name;
        }

        host(&self.url).unwrap_or(&self.url)
    }
}

/// The host part of a URL, without pulling in a URL parser for what is only ever a label.
fn host(url: &str) -> Option<&str> {
    let after_scheme = url.split_once("://")?.1;
    let host = after_scheme.split(['/', '?', '#']).next()?;

    (!host.is_empty()).then_some(host)
}

/// One person in the roster.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Person {
    /// The handle typed after the `@`, written without it.
    pub handle: String,
    /// Full name, shown alongside the handle while completing.
    pub name: Option<String>,
    pub email: Option<String>,
    pub team: Option<String>,
    pub role: Option<String>,
    /// Extra handles that resolve to this person, and that completion matches on too.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Links to this person elsewhere, in the order you wrote them. All of them show up on hover; the first is the
    /// one an editor opens on a modifier-click over the mention.
    #[serde(default)]
    pub links: Vec<Link>,
}

impl Person {
    /// The person's name, falling back to the handle when none is set.
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.handle)
    }

    /// One-line summary shown next to a completion: the name, then the role and team in parentheses.
    pub fn detail(&self) -> String {
        let mut detail = self.display_name().to_owned();

        let context: Vec<&str> = [self.role.as_deref(), self.team.as_deref()]
            .into_iter()
            .flatten()
            .collect();

        if !context.is_empty() {
            detail.push_str(" (");
            detail.push_str(&context.join(", "));
            detail.push(')');
        }

        detail
    }

    /// Markdown description shown in a completion's documentation popup and on hover.
    pub fn describe(&self) -> String {
        let mut lines = vec![format!("**{}** `@{}`", self.display_name(), self.handle)];

        if let Some(role) = &self.role {
            lines.push(role.clone());
        }

        if let Some(team) = &self.team {
            lines.push(format!("Team: {team}"));
        }

        if let Some(email) = &self.email {
            lines.push(format!("<{email}>"));
        }

        if !self.aliases.is_empty() {
            let aliases: Vec<String> = self.aliases.iter().map(|alias| format!("`@{alias}`")).collect();
            lines.push(format!("Also: {}", aliases.join(", ")));
        }

        if !self.links.is_empty() {
            let bullets: Vec<String> = self
                .links
                .iter()
                .map(|link| format!("- [{}]({})", link.label(), link.url))
                .collect();

            lines.push(bullets.join("\n"));
        }

        lines.join("\n\n")
    }

    /// Text a client fuzzy-matches the typed `@prefix` against.
    ///
    /// It leads with `@handle` because the replaced range starts at the `@`, so the client's query carries it too;
    /// the name, team and aliases follow so typing a name or a team also finds the person.
    pub fn filter_text(&self) -> String {
        let mut text = format!("@{}", self.handle);

        for extra in [self.name.as_deref(), self.team.as_deref(), self.role.as_deref()]
            .into_iter()
            .flatten()
        {
            text.push(' ');
            text.push_str(extra);
        }

        for alias in &self.aliases {
            text.push(' ');
            text.push_str(alias);
        }

        text
    }

    /// Whether the handle can actually be typed after an `@`, and so be completed and resolved.
    pub fn has_usable_handle(&self) -> bool {
        !self.handle.is_empty() && self.handle.chars().all(is_handle_char)
    }

    /// Whether `prefix` matches this person, and how well. `None` when it does not match at all.
    fn match_kind(&self, prefix: &str) -> Option<MatchKind> {
        if prefix.is_empty() {
            return Some(MatchKind::Handle);
        }

        if starts_with_fold(&self.handle, prefix) {
            return Some(MatchKind::Handle);
        }

        if self.aliases.iter().any(|alias| starts_with_fold(alias, prefix)) {
            return Some(MatchKind::Alias);
        }

        // Any word of the name, so `@doe` finds "Jane Doe" just as `@jane` does.
        if self.name.as_deref().is_some_and(|name| {
            name.split(|c: char| !c.is_alphanumeric())
                .any(|word| starts_with_fold(word, prefix))
        }) {
            return Some(MatchKind::Name);
        }

        if self
            .email
            .as_deref()
            .is_some_and(|email| starts_with_fold(email.split('@').next().unwrap_or(email), prefix))
        {
            return Some(MatchKind::Email);
        }

        None
    }
}

/// How a person matched a typed prefix. Declaration order is the completion ranking, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    /// The handle itself starts with the prefix.
    Handle,
    /// One of the aliases starts with the prefix.
    Alias,
    /// A word of the full name starts with the prefix.
    Name,
    /// The local part of the email address starts with the prefix.
    Email,
}

/// A person matched against a typed prefix, together with what matched.
pub struct Match<'a> {
    pub person: &'a Person,
    pub kind: MatchKind,
}

/// The roster, as parsed from `people.toml`.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Directory {
    #[serde(default)]
    pub people: Vec<Person>,
}

impl Directory {
    /// Everyone matching `prefix` (the text typed after an `@`), best match first.
    ///
    /// Ranking is by how the person matched, then alphabetically by handle, so the order is stable between keystrokes.
    /// People whose handle could never be typed after an `@` are left out.
    pub fn matches(&self, prefix: &str) -> Vec<Match<'_>> {
        let mut matches: Vec<Match<'_>> = self
            .people
            .iter()
            .filter(|person| person.has_usable_handle())
            .filter_map(|person| person.match_kind(prefix).map(|kind| Match { person, kind }))
            .collect();

        matches.sort_by(|a, b| {
            a.kind
                .cmp(&b.kind)
                .then_with(|| cmp_fold(&a.person.handle, &b.person.handle))
        });

        matches
    }

    /// The person a written `@handle` refers to, matched on the handle or any alias, ignoring case.
    pub fn resolve(&self, handle: &str) -> Option<&Person> {
        self.people
            .iter()
            .find(|person| eq_fold(&person.handle, handle) || person.aliases.iter().any(|alias| eq_fold(alias, handle)))
    }
}

/// A `@mention` span found in a line, as byte offsets into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mention<'a> {
    /// Byte offset of the `@`.
    pub start: usize,
    /// Byte offset just past the handle text this span covers.
    pub end: usize,
    /// The handle text between the `@` and `end`, written without the `@`.
    pub text: &'a str,
}

/// The mention being typed at byte offset `col`, spanning the `@` up to the cursor.
///
/// Returns `None` unless the cursor sits in a handle run introduced by an `@` that opens a mention: an `@` preceded by
/// another handle character is the one in an email address, not the start of a mention.
pub fn mention_before(line: &str, col: usize) -> Option<Mention<'_>> {
    let head = line.get(..col)?;

    // Walk back over the handle characters already typed; `start` ends up at the first of them, or at the cursor when
    // nothing has been typed since the `@`.
    let mut start = col;
    for (index, character) in head.char_indices().rev() {
        if !is_handle_char(character) {
            break;
        }

        start = index;
    }

    let before = &head[..start];
    if !before.ends_with('@') {
        return None;
    }

    let at = start - '@'.len_utf8();
    // `tom@example` and `@@` are not mentions being typed.
    if line[..at]
        .chars()
        .next_back()
        .is_some_and(|previous| is_handle_char(previous) || previous == '@')
    {
        return None;
    }

    Some(Mention {
        start: at,
        end: col,
        text: &line[start..col],
    })
}

/// The whole mention under byte offset `col`, extended past the cursor to the end of the handle.
///
/// The `@` itself counts as being inside the mention it opens, so pointing at the sigil answers the same as pointing
/// at the handle.
pub fn mention_at(line: &str, col: usize) -> Option<Mention<'_>> {
    let typed = mention_before(line, col).or_else(|| mention_before(line, col + '@'.len_utf8()))?;
    let tail = &line[typed.end..];
    let end = typed.end + tail.find(|c: char| !is_handle_char(c)).unwrap_or(tail.len());

    Some(Mention {
        start: typed.start,
        end,
        text: &line[typed.start + '@'.len_utf8()..end],
    })
}

/// Every mention written on a line, left to right.
///
/// Unlike the cursor-driven lookups above this one needs no cursor, so it is what an editor's whole-buffer passes
/// (document links, and anything else that decorates mentions) are built on.
pub fn mentions(line: &str) -> Vec<Mention<'_>> {
    line.match_indices('@')
        // A cursor sitting on one `@` can still resolve to a mention opened by an earlier one, so keep only the
        // mention this `@` actually starts. A bare `@` with nothing after it names nobody.
        .filter_map(|(at, _)| mention_at(line, at).filter(|mention| mention.start == at && !mention.text.is_empty()))
        .collect()
}

/// Whether a character can appear in a handle. Alphanumerics are Unicode-wide so accented names work unchanged.
fn is_handle_char(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '.' | '_' | '-')
}

/// Case-insensitive `starts_with`.
///
/// Compares lowercased character streams rather than byte slices, so an accented name never splits a character.
fn starts_with_fold(haystack: &str, prefix: &str) -> bool {
    let mut haystack = haystack.chars().flat_map(char::to_lowercase);

    prefix
        .chars()
        .flat_map(char::to_lowercase)
        .all(|wanted| haystack.next() == Some(wanted))
}

/// Case-insensitive equality.
fn eq_fold(left: &str, right: &str) -> bool {
    cmp_fold(left, right) == Ordering::Equal
}

/// Case-insensitive ordering, used to keep the completion list stable between keystrokes.
fn cmp_fold(left: &str, right: &str) -> Ordering {
    left.chars()
        .flat_map(char::to_lowercase)
        .cmp(right.chars().flat_map(char::to_lowercase))
}

/// Path to the roster: `people_file` if set, otherwise a `people.toml` beside the global config.
pub fn path(config: &Config) -> Option<PathBuf> {
    if let Some(configured) = config.people_file.as_deref() {
        return Some(config::expand_tilde(configured));
    }

    config::global_config_path().map(|path| path.with_file_name(PEOPLE_FILE_NAME))
}

/// Read and parse a roster file, returning an empty roster if it does not exist.
pub fn read(path: &Path) -> Result<Directory> {
    if !path.exists() {
        return Ok(Directory::default());
    }

    let contents = std::fs::read_to_string(path).with_context(|| format!("reading people file {}", path.display()))?;

    toml::from_str(&contents).with_context(|| format!("parsing people file {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(handle: &str, name: &str) -> Person {
        Person {
            handle: handle.into(),
            name: Some(name.into()),
            ..Person::default()
        }
    }

    fn directory() -> Directory {
        Directory {
            people: vec![
                Person {
                    aliases: vec!["jane".into()],
                    email: Some("jane.doe@example.com".into()),
                    team: Some("backend".into()),
                    ..person("jdoe", "Jane Doe")
                },
                person("jsmith", "John Smith"),
                Person {
                    email: Some("chloe.martin@example.com".into()),
                    ..person("cmartin", "Chloé Martin")
                },
            ],
        }
    }

    #[test]
    fn mention_before_covers_the_at_and_what_was_typed() {
        let line = "ping @jdo";
        let mention = mention_before(line, line.len()).unwrap();

        assert_eq!(mention.start, 5);
        assert_eq!(mention.end, 9);
        assert_eq!(mention.text, "jdo");
    }

    #[test]
    fn mention_before_matches_a_bare_at() {
        let mention = mention_before("ping @", 6).unwrap();

        assert_eq!(mention.start, 5);
        assert_eq!(mention.text, "");
    }

    #[test]
    fn mention_before_ignores_email_addresses_and_doubled_ats() {
        assert!(mention_before("jane@example", 12).is_none());
        assert!(mention_before("mail me at jane@ex", 18).is_none());
        assert!(mention_before("@@jdoe", 6).is_none());
    }

    #[test]
    fn mention_before_accepts_punctuation_and_line_start_before_the_at() {
        assert!(mention_before("@jdoe", 5).is_some());
        assert!(mention_before("- [ ] ask (@jdoe", 16).is_some());
        assert!(mention_before("**@jdoe", 7).is_some());
    }

    #[test]
    fn mention_before_stops_at_the_cursor() {
        // The cursor sits between `jd` and `oe`; only what is left of it has been typed.
        let mention = mention_before("@jdoe", 3).unwrap();

        assert_eq!(mention.text, "jd");
        assert_eq!(mention.end, 3);
    }

    #[test]
    fn mention_at_answers_on_the_at_sign_itself() {
        let mention = mention_at("ping @jdoe", 5).unwrap();

        assert_eq!(mention.start, 5);
        assert_eq!(mention.text, "jdoe");
    }

    #[test]
    fn mention_at_extends_past_the_cursor() {
        let mention = mention_at("ping @jdoe today", 8).unwrap();

        assert_eq!(mention.start, 5);
        assert_eq!(mention.end, 10);
        assert_eq!(mention.text, "jdoe");
    }

    #[test]
    fn mention_handles_multibyte_text() {
        let line = "réunion @cmartin";
        let mention = mention_before(line, line.len()).unwrap();

        assert_eq!(mention.text, "cmartin");
        assert_eq!(&line[mention.start..mention.end], "@cmartin");
    }

    #[test]
    fn mentions_finds_every_one_on_a_line() {
        let line = "- [ ] @jdoe et @cmartin, pas jane@example.com";
        let found = mentions(line);

        let text: Vec<&str> = found.iter().map(|mention| mention.text).collect();
        assert_eq!(text, ["jdoe", "cmartin"]);
        assert_eq!(&line[found[1].start..found[1].end], "@cmartin");
    }

    #[test]
    fn mentions_skips_a_bare_at_and_never_repeats_a_span() {
        assert!(mentions("email me @ noon").is_empty());
        // The second `@` resolves back into the first mention; it must not be reported twice.
        assert_eq!(mentions("@jd@oe").len(), 1);
    }

    #[test]
    fn a_link_falls_back_to_its_host_for_a_label() {
        let named = Link {
            name: Some("Chat".into()),
            url: "https://mail.google.com/chat/u/0/#chat/dm/AAAA".into(),
        };
        assert_eq!(named.label(), "Chat");

        let bare = Link {
            name: None,
            url: "https://gitlab.affluences.com/luis.valdez".into(),
        };
        assert_eq!(bare.label(), "gitlab.affluences.com");

        let odd = Link {
            name: None,
            url: "mailto:jane@example.com".into(),
        };
        assert_eq!(odd.label(), "mailto:jane@example.com");
    }

    #[test]
    fn describe_lists_the_links_as_markdown() {
        let person = Person {
            links: vec![Link {
                name: Some("Chat".into()),
                url: "https://chat.example.com/dm/1".into(),
            }],
            ..person("jdoe", "Jane Doe")
        };

        assert!(person.describe().contains("- [Chat](https://chat.example.com/dm/1)"));
    }

    #[test]
    fn matches_rank_handles_above_names() {
        let directory = directory();
        let matches = directory.matches("j");

        // Both handles start with `j`, and they sort alphabetically among themselves.
        let handles: Vec<&str> = matches.iter().map(|m| m.person.handle.as_str()).collect();
        assert_eq!(handles, ["jdoe", "jsmith"]);
        assert!(matches.iter().all(|m| m.kind == MatchKind::Handle));
    }

    #[test]
    fn matches_find_aliases_names_and_emails() {
        let directory = directory();

        assert_eq!(directory.matches("jane")[0].kind, MatchKind::Alias);
        // "Doe" is the second word of the name.
        let doe = &directory.matches("doe")[0];
        assert_eq!(doe.person.handle, "jdoe");
        assert_eq!(doe.kind, MatchKind::Name);
        // `chloe.` only appears in the email's local part.
        assert_eq!(directory.matches("chloe.")[0].person.handle, "cmartin");
    }

    #[test]
    fn matches_ignore_case_and_an_empty_prefix_lists_everyone() {
        let directory = directory();

        assert_eq!(directory.matches("JD")[0].person.handle, "jdoe");
        assert_eq!(directory.matches("").len(), 3);
    }

    #[test]
    fn matches_skip_untypable_handles() {
        let directory = Directory {
            people: vec![person("jane doe", "Jane Doe"), person("jdoe", "Jane Doe")],
        };

        let handles: Vec<&str> = directory.matches("").iter().map(|m| m.person.handle.as_str()).collect();
        assert_eq!(handles, ["jdoe"]);
    }

    #[test]
    fn resolve_matches_handles_and_aliases_ignoring_case() {
        let directory = directory();

        assert_eq!(directory.resolve("JDoe").unwrap().handle, "jdoe");
        assert_eq!(directory.resolve("jane").unwrap().handle, "jdoe");
        assert!(directory.resolve("nobody").is_none());
    }

    #[test]
    fn detail_lists_the_role_and_team() {
        let lead = Person {
            role: Some("Tech lead".into()),
            team: Some("backend".into()),
            ..person("jdoe", "Jane Doe")
        };

        assert_eq!(lead.detail(), "Jane Doe (Tech lead, backend)");
        assert_eq!(person("jdoe", "Jane Doe").detail(), "Jane Doe");
    }
}

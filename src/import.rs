//! Filling the people roster from an external directory export.
//!
//! Something else already knows who works here: a GitLab group, a Slack workspace, an HR export. This module takes
//! whatever that thing prints on standard input and folds it into `people.toml`, so the roster stays current without
//! anyone retyping it. Fetching and authentication stay outside: `selfnotes` reads a stream, it never calls an API.
//!
//! The merge only ever adds. A person already in the roster is left exactly as written, because the `team`, `role`
//! and `aliases` you filled in by hand are things no export knows about, and re-running the import must not undo
//! them. People the source no longer lists are reported and, with `--prune`, removed.
//!
//! Edits are made to the file's text rather than by re-serializing a parsed roster, so comments, key alignment and
//! entry order all survive. The result is parsed again before it is written, and a mismatch aborts the write.

use std::collections::HashSet;
use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use crate::people::Directory;

/// Object keys read as a person's handle, in order of preference. `username` covers GitLab and Slack, `login` GitHub.
const HANDLE_KEYS: [&str; 4] = ["handle", "username", "login", "nickname"];

/// Object keys read as a person's full name.
const NAME_KEYS: [&str; 4] = ["name", "full_name", "real_name", "display_name"];

/// Object keys read as a person's email address.
const EMAIL_KEYS: [&str; 3] = ["email", "public_email", "mail"];

/// Object keys read as a person's team.
const TEAM_KEYS: [&str; 3] = ["team", "department", "division"];

/// Object keys read as a person's role.
const ROLE_KEYS: [&str; 3] = ["role", "title", "job_title"];

/// Shape of the text arriving on standard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// One or more JSON arrays of objects, or a stream of bare objects.
    Json,
    /// Tab-separated rows, with or without a header row.
    Tsv,
    /// Comma-separated rows, with or without a header row.
    Csv,
}

/// One person as an external source describes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub handle: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub team: Option<String>,
    pub role: Option<String>,
}

impl Record {
    /// The record rendered as a `[[people]]` block, ready to append to a roster file.
    fn render(&self) -> String {
        let mut block = String::from("[[people]]\n");

        for (key, value) in [
            ("handle", Some(self.handle.as_str())),
            ("name", self.name.as_deref()),
            ("email", self.email.as_deref()),
            ("team", self.team.as_deref()),
            ("role", self.role.as_deref()),
        ] {
            if let Some(value) = value {
                // `toml::Value`'s own rendering handles the quoting, so an apostrophe or a backslash in a name cannot
                // produce a file that no longer parses.
                let _ = writeln!(block, "{key} = {}", toml::Value::String(value.to_owned()));
            }
        }

        block
    }
}

/// What an import would do to a roster.
pub struct Plan {
    /// Records not yet in the roster, in source order.
    pub added: Vec<Record>,
    /// Handles the source lists that the roster already has, left untouched.
    pub present: Vec<String>,
    /// Handles in the roster that the source does not list.
    pub missing: Vec<String>,
    /// Source records that carried no usable handle, or that were not active accounts.
    pub skipped: usize,
}

impl Plan {
    /// Whether applying this plan would change the file at all.
    pub const fn is_empty(&self, prune: bool) -> bool {
        self.added.is_empty() && (!prune || self.missing.is_empty())
    }
}

/// Work out what `records` would change in `directory`, without touching anything.
///
/// A record matches an existing person by handle or by one of their aliases, ignoring case, so renaming someone
/// upstream after recording the old name as an alias does not create a duplicate.
pub fn plan(directory: &Directory, records: &[Record], skipped: usize) -> Plan {
    let mut added = Vec::new();
    let mut present = Vec::new();
    let mut matched: HashSet<String> = HashSet::new();

    for record in records {
        match directory.resolve(&record.handle) {
            Some(person) => {
                matched.insert(person.handle.to_lowercase());
                present.push(person.handle.clone());
            },
            // Guard against the same person appearing twice in one export.
            None if added
                .iter()
                .any(|earlier: &Record| earlier.handle.eq_ignore_ascii_case(&record.handle)) => {},
            None => added.push(record.clone()),
        }
    }

    let missing = directory
        .people
        .iter()
        .filter(|person| !matched.contains(&person.handle.to_lowercase()))
        .map(|person| person.handle.clone())
        .collect();

    Plan {
        added,
        present,
        missing,
        skipped,
    }
}

/// Apply `plan` to the text of a roster file, returning the new text.
///
/// New entries are appended and, with `prune`, the blocks of people the source no longer lists are cut out. Nothing
/// else in the file is rewritten. The result is parsed before it is returned, and an unexpected roster aborts rather
/// than overwriting a file with something the edit did not intend.
pub fn apply(text: &str, plan: &Plan, prune: bool) -> Result<String> {
    let mut updated = if prune && !plan.missing.is_empty() {
        let dropped: HashSet<String> = plan.missing.iter().map(|handle| handle.to_lowercase()).collect();

        remove_blocks(text, &dropped)
    } else {
        text.to_owned()
    };

    for record in &plan.added {
        // A file that does not end in a newline would otherwise swallow the appended header, and entries read better
        // with a blank line between them. Neither applies to the first entry of an empty file.
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }

        if !updated.is_empty() && !updated.ends_with("\n\n") {
            updated.push('\n');
        }

        updated.push_str(&record.render());
    }

    verify(&updated, text, plan, prune)?;

    Ok(updated)
}

/// Check that the edited text parses, and holds exactly the people it should.
fn verify(updated: &str, original: &str, plan: &Plan, prune: bool) -> Result<()> {
    let before: Directory = toml::from_str(original).context("re-reading the roster before the edit")?;
    let after: Directory = toml::from_str(updated).context("the edited roster is no longer valid TOML")?;

    let dropped: HashSet<String> = if prune {
        plan.missing.iter().map(|handle| handle.to_lowercase()).collect()
    } else {
        HashSet::new()
    };

    let mut expected: Vec<String> = before
        .people
        .iter()
        .map(|person| person.handle.to_lowercase())
        .filter(|handle| !dropped.contains(handle))
        .collect();
    expected.extend(plan.added.iter().map(|record| record.handle.to_lowercase()));

    let found: Vec<String> = after.people.iter().map(|person| person.handle.to_lowercase()).collect();

    if found != expected {
        bail!(
            "the edit would have left the roster holding {} people instead of {}; nothing was written",
            found.len(),
            expected.len()
        );
    }

    Ok(())
}

/// Cut the `[[people]]` blocks whose handle is in `dropped` out of a roster file's text.
///
/// A block runs from its `[[people]]` header to the line before the next table header, so the blank lines and the
/// comments written under an entry go with it.
fn remove_blocks(text: &str, dropped: &HashSet<String>) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;

    while index < lines.len() {
        if lines[index].trim() != "[[people]]" {
            out.push_str(lines[index]);
            index += 1;

            continue;
        }

        let start = index;
        let mut end = index + 1;
        while end < lines.len() && !lines[end].trim_start().starts_with('[') {
            end += 1;
        }

        let block = &lines[start..end];
        // Parsing the block on its own is how its handle is read, so an unusual but valid spelling still matches.
        let handle = toml::from_str::<Directory>(&block.concat())
            .ok()
            .and_then(|parsed| parsed.people.into_iter().next())
            .map(|person| person.handle.to_lowercase());

        if !handle.is_some_and(|handle| dropped.contains(&handle)) {
            for line in block {
                out.push_str(line);
            }
        }

        index = end;
    }

    out
}

/// Read the records an external source printed, in `format`.
///
/// Returns the usable records and how many rows had to be dropped, either for carrying no handle or for describing an
/// account the source itself marks as inactive.
pub fn parse(input: &str, format: Format) -> Result<(Vec<Record>, usize)> {
    match format {
        Format::Json => parse_json(input),
        Format::Tsv => Ok(parse_rows(input, '\t')),
        Format::Csv => Ok(parse_rows(input, ',')),
    }
}

/// Read a JSON export: one array, several arrays back to back (what a paginated fetch prints), or bare objects.
fn parse_json(input: &str) -> Result<(Vec<Record>, usize)> {
    let mut records = Vec::new();
    let mut skipped = 0;

    for value in serde_json::Deserializer::from_str(input).into_iter::<Value>() {
        let value = value.context("reading JSON from standard input")?;

        match value {
            Value::Array(entries) => {
                for entry in entries {
                    push_json(&entry, &mut records, &mut skipped);
                }
            },
            entry => push_json(&entry, &mut records, &mut skipped),
        }
    }

    Ok((records, skipped))
}

/// Turn one JSON object into a record, counting it as skipped when it cannot be used.
fn push_json(entry: &Value, records: &mut Vec<Record>, skipped: &mut usize) {
    // A source that reports account state (GitLab, Slack) should not put blocked or deactivated people in a
    // completion popup.
    let inactive = entry
        .get("state")
        .and_then(Value::as_str)
        .is_some_and(|state| state != "active")
        || entry.get("deleted").and_then(Value::as_bool) == Some(true);

    let handle = HANDLE_KEYS
        .iter()
        .find_map(|key| entry.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|handle| !handle.is_empty());

    match handle {
        Some(handle) if !inactive => records.push(Record {
            handle: handle.to_owned(),
            name: json_field(entry, &NAME_KEYS),
            email: json_field(entry, &EMAIL_KEYS),
            team: json_field(entry, &TEAM_KEYS),
            role: json_field(entry, &ROLE_KEYS),
        }),
        _ => *skipped += 1,
    }
}

/// The first of `keys` present on `entry` as a non-empty string.
fn json_field(entry: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| entry.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Read a delimited export, using a header row to name the columns when there is one.
///
/// Without a header the columns are taken as handle then name, which is what a bare
/// `jq -r '.[] | "\(.username)\t\(.name)"'` prints.
fn parse_rows(input: &str, delimiter: char) -> (Vec<Record>, usize) {
    let mut rows = input
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| split_row(line, delimiter))
        .peekable();

    let header = rows
        .peek()
        .filter(|first| {
            first
                .first()
                .is_some_and(|cell| HANDLE_KEYS.contains(&cell.to_lowercase().as_str()))
        })
        .cloned();

    if header.is_some() {
        rows.next();
    }

    let column = |names: &[&str]| -> Option<usize> {
        header.as_ref().and_then(|header| {
            header
                .iter()
                .position(|cell| names.contains(&cell.to_lowercase().as_str()))
        })
    };

    let (handle_at, name_at) = match &header {
        Some(_) => (column(&HANDLE_KEYS).unwrap_or(0), column(&NAME_KEYS)),
        None => (0, Some(1)),
    };
    let (email_at, team_at, role_at) = (column(&EMAIL_KEYS), column(&TEAM_KEYS), column(&ROLE_KEYS));

    let mut records = Vec::new();
    let mut skipped = 0;

    for row in rows {
        let cell = |at: Option<usize>| -> Option<String> {
            at.and_then(|at| row.get(at))
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };

        match cell(Some(handle_at)) {
            Some(handle) => records.push(Record {
                handle,
                name: cell(name_at),
                email: cell(email_at),
                team: cell(team_at),
                role: cell(role_at),
            }),
            None => skipped += 1,
        }
    }

    (records, skipped)
}

/// Split one delimited row, honouring the quoting `jq`'s `@csv` and `@tsv` produce.
fn split_row(line: &str, delimiter: char) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut characters = line.chars().peekable();
    let mut quoted = false;

    while let Some(character) = characters.next() {
        match character {
            // A doubled quote inside a quoted cell is a literal one; `@csv` writes them that way.
            '"' if quoted && characters.peek() == Some(&'"') => {
                characters.next();
                cell.push('"');
            },
            '"' => quoted = !quoted,
            // `@tsv` cannot put a raw tab or newline in a cell, so it escapes them instead.
            '\\' if delimiter == '\t' => match characters.next() {
                Some('t') => cell.push('\t'),
                Some('n') => cell.push('\n'),
                Some('r') => cell.push('\r'),
                Some(other) => {
                    cell.push('\\');

                    if other != '\\' {
                        cell.push(other);
                    }
                },
                None => cell.push('\\'),
            },
            character if character == delimiter && !quoted => cells.push(std::mem::take(&mut cell)),
            character => cell.push(character),
        }
    }

    cells.push(cell);

    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(handle: &str, name: &str) -> Record {
        Record {
            handle: handle.into(),
            name: Some(name.into()),
            email: None,
            team: None,
            role: None,
        }
    }

    fn roster(text: &str) -> Directory {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn json_reads_gitlab_members() {
        let input = r#"[
            {"username": "luis.valdez", "name": "Luis Valdez", "state": "active", "access_level": 30},
            {"username": "blocked.user", "name": "Blocked User", "state": "blocked"}
        ]"#;

        let (records, skipped) = parse(input, Format::Json).unwrap();

        assert_eq!(records, vec![record("luis.valdez", "Luis Valdez")]);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn json_reads_pages_printed_back_to_back() {
        // What a paginated fetch prints when it does not merge the pages itself.
        let input = r#"[{"username": "a", "name": "A"}] [{"username": "b", "name": "B"}]"#;

        let (records, _) = parse(input, Format::Json).unwrap();

        assert_eq!(records.len(), 2);
        assert_eq!(records[1].handle, "b");
    }

    #[test]
    fn json_takes_the_other_field_names_sources_use() {
        let input = r#"[{"login": "octocat", "full_name": "The Octocat", "public_email": "cat@example.com"}]"#;

        let (records, _) = parse(input, Format::Json).unwrap();

        assert_eq!(records[0].handle, "octocat");
        assert_eq!(records[0].name.as_deref(), Some("The Octocat"));
        assert_eq!(records[0].email.as_deref(), Some("cat@example.com"));
    }

    #[test]
    fn tsv_without_a_header_is_handle_then_name() {
        let (records, _) = parse("luis.valdez\tLuis Valdez\njdoe\tJane Doe\n", Format::Tsv).unwrap();

        assert_eq!(
            records,
            vec![record("luis.valdez", "Luis Valdez"), record("jdoe", "Jane Doe")]
        );
    }

    #[test]
    fn csv_with_a_header_maps_columns_by_name() {
        let input = "username,name,access\n\"jdoe\",\"Doe, Jane\",\"Developer\"\n";

        let (records, _) = parse(input, Format::Csv).unwrap();

        assert_eq!(records[0].handle, "jdoe");
        // The quoted comma stays inside the cell rather than starting a new column.
        assert_eq!(records[0].name.as_deref(), Some("Doe, Jane"));
    }

    #[test]
    fn tsv_unescapes_what_jq_escaped() {
        let (records, _) = parse("jdoe\tJane\\tDoe\n", Format::Tsv).unwrap();

        assert_eq!(records[0].name.as_deref(), Some("Jane\tDoe"));
    }

    #[test]
    fn planning_separates_new_from_known_and_missing() {
        let directory = roster(
            r#"
            [[people]]
            handle = "jdoe"
            aliases = ["jane"]

            [[people]]
            handle = "gone"
            "#,
        );

        let records = vec![record("jdoe", "Jane Doe"), record("newbie", "New Bie")];
        let plan = plan(&directory, &records, 0);

        assert_eq!(plan.added, vec![record("newbie", "New Bie")]);
        assert_eq!(plan.present, vec!["jdoe"]);
        assert_eq!(plan.missing, vec!["gone"]);
    }

    #[test]
    fn planning_matches_an_alias_so_a_rename_is_not_a_duplicate() {
        let directory = roster("[[people]]\nhandle = \"jdoe\"\naliases = [\"j.doe\"]\n");
        let plan = plan(&directory, &[record("j.doe", "Jane Doe")], 0);

        assert!(plan.added.is_empty());
        assert_eq!(plan.present, vec!["jdoe"]);
    }

    #[test]
    fn applying_appends_without_disturbing_what_is_there() {
        let text = "# my colleagues\n\n[[people]]\nhandle  = \"jdoe\"\nteam    = \"backend\"\naliases = [\"jane\"]\n";
        let plan = plan(
            &roster(text),
            &[record("jdoe", "Renamed Upstream"), record("newbie", "New Bie")],
            0,
        );

        let updated = apply(text, &plan, false).unwrap();

        // The comment, the key alignment and the hand-written fields are all still there, verbatim.
        assert!(updated.starts_with(text), "{updated}");
        assert!(updated.contains("handle = \"newbie\""), "{updated}");
        // The existing entry keeps its own name, because no export knows better than the roster does.
        assert!(!updated.contains("Renamed Upstream"), "{updated}");
    }

    #[test]
    fn applying_quotes_values_that_would_break_the_file() {
        let plan = Plan {
            added: vec![record("oreilly", "Tim O\"Reilly\\")],
            present: Vec::new(),
            missing: Vec::new(),
            skipped: 0,
        };

        let updated = apply("", &plan, false).unwrap();

        assert_eq!(roster(&updated).people[0].name.as_deref(), Some("Tim O\"Reilly\\"));
    }

    #[test]
    fn pruning_cuts_only_the_blocks_it_should() {
        let text = "\
# roster
[[people]]
handle = \"stays\"

[[people]]
handle = \"goes\"
name = \"Gone Away\"

[[people]]
handle = \"also-stays\"
";
        let plan = plan(&roster(text), &[record("stays", "S"), record("also-stays", "A")], 0);
        assert_eq!(plan.missing, vec!["goes"]);

        let updated = apply(text, &plan, true).unwrap();

        assert!(updated.starts_with("# roster\n"), "{updated}");
        assert!(!updated.contains("goes"), "{updated}");
        let handles: Vec<String> = roster(&updated).people.into_iter().map(|p| p.handle).collect();
        assert_eq!(handles, ["stays", "also-stays"]);
    }

    #[test]
    fn a_file_without_a_trailing_newline_still_gains_a_readable_entry() {
        let text = "[[people]]\nhandle = \"jdoe\"";
        let plan = plan(&roster(text), &[record("newbie", "New Bie")], 0);

        let updated = apply(text, &plan, false).unwrap();

        assert_eq!(roster(&updated).people.len(), 2);
    }

    #[test]
    fn an_export_listing_the_same_person_twice_adds_them_once() {
        let plan = plan(
            &Directory::default(),
            &[record("jdoe", "Jane"), record("JDOE", "Jane")],
            0,
        );

        assert_eq!(plan.added.len(), 1);
    }
}

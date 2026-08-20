//! Note statuses: the ordered workflow a note moves through, and the frontmatter key recording where it currently is.
//!
//! A status is not a tag. A tag is an open, multi-valued set, while a status is single-valued, exclusive and mutable:
//! a note is in exactly one of `backlog`, `todo`, `doing`, ... at a time, and moving it means replacing that value
//! rather than adding another. So it lives in its own `status` key of the `+++` frontmatter, and the values it may
//! take are declared per folder (see [`crate::config::FolderConfig::statuses`]) rather than fixed by this crate: a
//! ticket folder ends at `prod`, an ideas folder ends at `dropped`, and neither ladder belongs in the binary.
//!
//! Writing a status edits a file the user wrote, so [`set`] replaces (or inserts) a single line and leaves every other
//! byte of the frontmatter alone. A `toml::Table` round-trip would be shorter but would also reorder the keys and
//! discard the comments, which is not an acceptable thing to do to someone's note.

use anyhow::{Result, bail};

use crate::config::{Config, FolderConfig};
use crate::notes;

/// Frontmatter key a note's status is written to and read from.
pub const KEY: &str = "status";

/// The ordered set of statuses notes from one source may take, with which of them is the default and which end the
/// line.
///
/// Borrowed from the config rather than owned, since it is only ever read for the length of one command.
#[derive(Debug, Clone, Copy)]
pub struct Workflow<'a> {
    /// Every status, in the order they were declared, which is the order a board shows them in and the order
    /// [`Workflow::next_after`] walks.
    statuses: &'a [String],
    /// The configured `default_status`, which is only honored when it is one of `statuses`.
    configured_default: Option<&'a str>,
    /// Statuses that close a note, hidden from a board unless it is asked for everything.
    terminal: &'a [String],
}

impl<'a> Workflow<'a> {
    /// The workflow for entries of `folder`, falling back to the global keys for whichever of the three the folder
    /// leaves unset.
    pub fn for_folder(config: &'a Config, folder: &'a FolderConfig) -> Self {
        Self {
            statuses: config.folder_statuses(folder),
            configured_default: config.folder_default_status(folder),
            terminal: config.folder_terminal_statuses(folder),
        }
    }

    /// The workflow for a note's source label, which is a custom folder's.
    ///
    /// The journal has none, and neither has any name that is not a configured folder: a dated log entry is not a
    /// ticket, and a top-level `statuses` is a default *for folders* rather than a workflow every note is dragged
    /// into. Without this a board would bury every column under a year of journal entries carrying no status.
    pub fn for_source(config: &'a Config, source: &str) -> Self {
        config
            .folder(source)
            .map_or_else(Self::none, |folder| Self::for_folder(config, folder))
    }

    /// A workflow with no statuses at all, i.e. a source that does not track them.
    pub const fn none() -> Self {
        Self {
            statuses: &[],
            configured_default: None,
            terminal: &[],
        }
    }

    /// Whether no statuses are declared at all, i.e. this source does not track statuses.
    pub const fn is_empty(&self) -> bool {
        self.statuses.is_empty()
    }

    /// Every status, in declaration order.
    pub const fn statuses(&self) -> &'a [String] {
        self.statuses
    }

    /// The position of `value` in the workflow, matched case-insensitively.
    pub fn index(&self, value: &str) -> Option<usize> {
        let value = normalize(value);

        self.statuses.iter().position(|status| normalize(status) == value)
    }

    /// `value` as the workflow spells it, or `None` when it is not one of the declared statuses.
    ///
    /// This is what makes `selfnotes status <note> DOING` write `doing`: what lands in the file is the configured
    /// spelling, never the one that happened to be typed.
    pub fn resolve(&self, value: &str) -> Option<&'a str> {
        self.index(value).map(|index| self.statuses[index].as_str())
    }

    /// The status a new note starts in: the configured `default_status` when it is part of the workflow, and the first
    /// declared status otherwise.
    ///
    /// Falling back rather than failing keeps a typo in `default_status` from blocking note creation; `selfnotes
    /// config validate` is where it is reported.
    pub fn default_status(&self) -> Option<&'a str> {
        self.configured_default
            .and_then(|configured| self.resolve(configured))
            .or_else(|| self.statuses.first().map(String::as_str))
    }

    /// Whether `value` closes a note, so a board leaves it out by default.
    pub fn is_terminal(&self, value: &str) -> bool {
        let value = normalize(value);

        self.terminal.iter().any(|status| normalize(status) == value)
    }

    /// The status one step further along than `value`, or `None` when `value` is unknown or already the last one.
    pub fn next_after(&self, value: &str) -> Option<&'a str> {
        let index = self.index(value)?;

        self.statuses.get(index + 1).map(String::as_str)
    }

    /// The statuses as a backticked, comma-separated list, ready to drop into a message.
    pub fn names(&self) -> String {
        self.statuses
            .iter()
            .map(|status| format!("`{status}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A note's status as written in its frontmatter, trimmed; `None` when the key is absent or blank.
pub fn read(content: &str) -> Option<String> {
    notes::extract_status(content)
}

/// Whether a note's status satisfies a `--status` filter.
///
/// Unlike a tag filter, which requires *every* requested tag, several statuses are alternatives: a note holds exactly
/// one, so `--status todo --status doing` can only sensibly mean either. An empty filter matches every note, and a
/// note with no status matches only an empty filter.
pub fn matches(note_status: Option<&str>, wanted: &[String]) -> bool {
    if wanted.is_empty() {
        return true;
    }

    let Some(note_status) = note_status else {
        return false;
    };
    let note_status = normalize(note_status);

    wanted.iter().any(|want| normalize(want) == note_status)
}

/// Rewrite `content` so its frontmatter records `status`, returning the new file contents.
///
/// An existing `status` line is replaced in place, keeping its indentation and its position among the other keys;
/// otherwise the assignment is inserted after the last top-level key, before any `[table]` and the comments that
/// introduce it. Everything else, comments and key order included, is left byte for byte as it was. A note with no
/// frontmatter gets a fresh block.
///
/// Errors rather than guessing when the frontmatter cannot be edited safely: an unterminated `+++` fence, a block that
/// is not valid TOML, or an edit whose result does not read back as the status just written.
pub fn set(content: &str, status: &str) -> Result<String> {
    match notes::split_frontmatter(content) {
        (Some(frontmatter), body) => match replace_or_insert(frontmatter, status) {
            Some(updated) => Ok(format!("+++\n{updated}+++\n{body}")),
            None => bail!("the note's `+++` frontmatter is not valid TOML; fix it and try again"),
        },
        (None, _) if opens_a_fence(content) => {
            bail!("the note opens a `+++` frontmatter that is never closed; fix it and try again")
        },
        (None, _) => Ok(format!("+++\n{}\n+++\n\n{content}", assignment(status))),
    }
}

/// Give `content` a `status` only if it does not already carry one, for seeding a folder's `default_status` into a new
/// entry.
///
/// Never overwrites: a template that sets its own `status` (from a custom field, say) has already said what the entry
/// starts as. A frontmatter that cannot be edited is left untouched rather than failing the creation, mirroring how
/// [`crate::notes::ensure_frontmatter_tags`] tolerates one it cannot parse.
pub fn seed(content: &str, status: Option<&str>) -> String {
    let Some(status) = status else {
        return content.to_owned();
    };

    if read(content).is_some() {
        return content.to_owned();
    }

    set(content, status).unwrap_or_else(|_| content.to_owned())
}

/// Whether the first line of `content` opens a frontmatter fence, used to tell "no frontmatter" from "a frontmatter
/// that was never closed".
fn opens_a_fence(content: &str) -> bool {
    content.lines().next().is_some_and(|line| line.trim_end() == "+++")
}

/// Rewrite one frontmatter block so it assigns `status`, or `None` when it is not TOML we can safely edit.
///
/// Works on lines rather than on a parsed table so that comments, key order and formatting survive. The result is
/// parsed back and checked to actually hold `status`, which catches the shapes this line-level edit cannot see
/// through, such as a quoted `"status"` key that would otherwise end up duplicated.
fn replace_or_insert(frontmatter: &str, status: &str) -> Option<String> {
    frontmatter.parse::<toml::Table>().ok()?;

    let mut lines: Vec<String> = frontmatter.split_inclusive('\n').map(str::to_owned).collect();
    // A top-level key must precede the first `[table]` header, so that header bounds the region to edit.
    let top_level = lines
        .iter()
        .position(|line| line.trim_start().starts_with('['))
        .unwrap_or(lines.len());

    let assignment = assignment(status);

    if let Some(index) = lines[..top_level].iter().position(|line| assigns_status(line)) {
        let indent: String = lines[index].chars().take_while(|c| *c == ' ' || *c == '\t').collect();

        lines[index] = format!("{indent}{assignment}\n");
    } else {
        // After the last real key, so the assignment does not land between a table's leading comments and the table
        // itself.
        let at = lines[..top_level]
            .iter()
            .rposition(|line| {
                let trimmed = line.trim();

                !trimmed.is_empty() && !trimmed.starts_with('#')
            })
            .map_or(0, |index| index + 1);

        lines.insert(at, format!("{assignment}\n"));
    }

    let updated = lines.concat();
    let written = updated
        .parse::<toml::Table>()
        .ok()?
        .get(KEY)
        .and_then(toml::Value::as_str)?
        .to_owned();

    (written == status).then_some(updated)
}

/// Whether a frontmatter line assigns the top-level `status` key.
fn assigns_status(line: &str) -> bool {
    line.trim_start()
        .strip_prefix(KEY)
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// The `status = "..."` assignment, with the value escaped as TOML rather than pasted between quotes.
fn assignment(status: &str) -> String {
    format!("{KEY} = {}", toml::Value::String(status.to_owned()))
}

/// Trim and case-fold a status for comparison, so `Doing`, `doing` and ` doing ` are one value.
fn normalize(status: &str) -> String {
    status.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workflow(statuses: &[String], default: Option<&str>, terminal: &[String]) -> Workflow<'static> {
        // The test data outlives the assertions; leaking keeps `Workflow`'s borrowed shape without a config fixture.
        Workflow {
            statuses: Box::leak(statuses.to_vec().into_boxed_slice()),
            configured_default: default.map(|value| &*Box::leak(value.to_owned().into_boxed_str())),
            terminal: Box::leak(terminal.to_vec().into_boxed_slice()),
        }
    }

    fn ladder() -> Vec<String> {
        ["backlog", "todo", "doing", "done"]
            .iter()
            .map(|status| (*status).to_owned())
            .collect()
    }

    #[test]
    fn next_after_walks_the_declared_order_and_stops_at_the_end() {
        let flow = workflow(&ladder(), None, &[]);

        assert_eq!(flow.next_after("backlog"), Some("todo"));
        assert_eq!(flow.next_after("doing"), Some("done"));
        assert_eq!(flow.next_after("done"), None);
        assert_eq!(flow.next_after("nonsense"), None);
    }

    #[test]
    fn statuses_are_matched_case_insensitively_and_answer_in_the_configured_spelling() {
        let flow = workflow(&ladder(), None, &[]);

        assert_eq!(flow.resolve("  DOING "), Some("doing"));
        assert_eq!(flow.resolve("shipped"), None);
    }

    #[test]
    fn default_status_falls_back_to_the_first_when_unset_or_unknown() {
        assert_eq!(workflow(&ladder(), None, &[]).default_status(), Some("backlog"));
        assert_eq!(workflow(&ladder(), Some("todo"), &[]).default_status(), Some("todo"));
        // A `default_status` that is not part of the workflow is reported by `config validate`, not enforced here.
        assert_eq!(
            workflow(&ladder(), Some("bogus"), &[]).default_status(),
            Some("backlog")
        );
        assert_eq!(workflow(&[], None, &[]).default_status(), None);
    }

    #[test]
    fn a_status_filter_takes_any_of_the_requested_values() {
        let wanted = vec!["todo".to_owned(), "doing".to_owned()];

        assert!(matches(Some("doing"), &wanted));
        assert!(matches(Some("TODO"), &wanted));
        assert!(!matches(Some("done"), &wanted));
        assert!(!matches(None, &wanted));
        // An empty filter is no filter at all.
        assert!(matches(None, &[]));
    }

    #[test]
    fn setting_a_status_replaces_the_existing_line_and_keeps_everything_else() {
        let note =
            "+++\n# what this note is\ntags = [\"idea\"]\nstatus = \"todo\"\ntitle = \"Login bug\"\n+++\n\n# body\n";

        let updated = set(note, "doing").unwrap();

        assert_eq!(
            updated,
            "+++\n# what this note is\ntags = [\"idea\"]\nstatus = \"doing\"\ntitle = \"Login bug\"\n+++\n\n# body\n"
        );
    }

    #[test]
    fn setting_a_status_inserts_after_the_last_key_when_absent() {
        let note = "+++\ntags = [\"idea\"]\n+++\n\n# body\n";

        assert_eq!(
            set(note, "backlog").unwrap(),
            "+++\ntags = [\"idea\"]\nstatus = \"backlog\"\n+++\n\n# body\n"
        );
    }

    #[test]
    fn an_inserted_status_stays_out_of_a_table_and_its_leading_comments() {
        let note = "+++\ntags = [\"idea\"]\n\n# who owns this\n[meta]\nowner = \"tom\"\n+++\n\nbody\n";

        assert_eq!(
            set(note, "todo").unwrap(),
            "+++\ntags = [\"idea\"]\nstatus = \"todo\"\n\n# who owns this\n[meta]\nowner = \"tom\"\n+++\n\nbody\n"
        );
    }

    #[test]
    fn a_note_without_frontmatter_gets_one() {
        assert_eq!(
            set("# body\n", "todo").unwrap(),
            "+++\nstatus = \"todo\"\n+++\n\n# body\n"
        );
    }

    #[test]
    fn an_unparseable_or_unterminated_frontmatter_is_refused() {
        assert!(set("+++\ntags = [\n+++\n\nbody\n", "todo").is_err());
        assert!(set("+++\ntags = []\n\nbody\n", "todo").is_err());
    }

    #[test]
    fn a_status_value_is_escaped_rather_than_pasted() {
        let updated = set("# body\n", "in \"review\"").unwrap();

        assert_eq!(read(&updated).as_deref(), Some("in \"review\""));
    }

    #[test]
    fn seeding_never_overwrites_a_status_the_template_already_set() {
        let note = "+++\nstatus = \"doing\"\n+++\n\nbody\n";

        assert_eq!(seed(note, Some("backlog")), note);
        assert_eq!(seed("body\n", None), "body\n");
        assert_eq!(
            seed("body\n", Some("backlog")),
            "+++\nstatus = \"backlog\"\n+++\n\nbody\n"
        );
    }
}

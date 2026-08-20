//! Grouping notes by status into the columns of a board.
//!
//! Enumeration and parsing live in [`crate::notes`] and the workflows in [`crate::status`]; this module puts the two
//! together, bucketing every note under the status it carries and ordering the buckets the way the config declares
//! them. A closed note is left out unless asked for, since a board is meant to show what is still moving.
//!
//! Columns are built from the workflows themselves rather than from the statuses notes happen to carry, so a stage
//! nobody is in still shows up (empty), which is the point of having declared it.

use anyhow::Result;

use crate::config::Config;
use crate::notes::{self, NoteFile};
use crate::status::Workflow;

/// Heading shown over the notes that carry no status yet.
pub const UNTRACKED: &str = "(no status)";

/// One note on the board.
pub struct Card {
    /// The file it was read from, carrying its source and modification time.
    pub file: NoteFile,
    /// Human-readable title from the frontmatter, if any.
    pub title: Option<String>,
}

/// One status, with the notes currently in it.
pub struct Column {
    /// The status, spelled as the config declares it.
    pub status: String,
    /// The notes in it, newest first.
    pub cards: Vec<Card>,
    /// How many were dropped by the per-column `limit`.
    pub truncated: usize,
}

/// Every column, plus the notes that fall outside the workflow.
pub struct Board {
    /// The declared statuses, in workflow order, minus the closed ones unless `all` was asked for.
    pub columns: Vec<Column>,
    /// Notes in a source that has a workflow but carrying no `status` of their own: what still needs triaging.
    pub untracked: Column,
    /// Notes whose `status` is not one their source declares, which is almost always a typo.
    pub unknown: Vec<Column>,
    /// How many notes were hidden for sitting in a closed status.
    pub closed: usize,
}

impl Board {
    /// Whether nothing at all was collected, so the caller can say so rather than print a wall of empty columns.
    pub fn is_empty(&self) -> bool {
        self.closed == 0
            && self.untracked.cards.is_empty()
            && self.unknown.is_empty()
            && self.columns.iter().all(|column| column.cards.is_empty())
    }
}

/// Build the board for one source, or for every source at once.
///
/// `folder` restricts the sources scanned (see [`crate::notes::walk`]) and, with it, the workflow the columns come
/// from; without it the columns are every workflow's statuses in turn, de-duplicated. `tags` filters the notes exactly
/// as `list --tag` does. `all` keeps the closed statuses, and `limit` caps each column (`0` for no cap).
pub fn build(config: &Config, folder: Option<&str>, tags: &[String], all: bool, limit: usize) -> Result<Board> {
    let files = notes::walk(config, folder)?;
    let hash_min_len = config.hash_tag_min_len();

    let mut columns: Vec<Column> = column_order(config, folder)
        .into_iter()
        // A closed stage is not a stage anyone is working through, so its column goes with its cards. Should a note
        // sit there under a workflow that does not call it closed, `push` puts the column back.
        .filter(|status| all || !closes_a_note(config, folder, status))
        .map(|status| Column::named(&status))
        .collect();
    let mut untracked = Column::named(UNTRACKED);
    let mut unknown: Vec<Column> = Vec::new();
    let mut closed = 0;

    for file in files {
        let Ok(content) = std::fs::read_to_string(&file.path) else {
            continue;
        };

        let parsed = notes::parse(&content, hash_min_len);
        if !notes::matches_tags(&parsed.tags, tags) {
            continue;
        }

        let workflow = Workflow::for_source(config, &file.source);
        // A source that tracks no statuses has nothing to do with a board; leaving its notes out keeps the journal
        // from filling the untracked column every single day.
        if workflow.is_empty() {
            continue;
        }

        let card = Card {
            file,
            title: parsed.title,
        };
        let Some(written) = parsed.status else {
            untracked.cards.push(card);

            continue;
        };

        let Some(status) = workflow.resolve(&written) else {
            push(&mut unknown, &written, card);

            continue;
        };

        if !all && workflow.is_terminal(status) {
            closed += 1;

            continue;
        }

        push(&mut columns, status, card);
    }

    for column in columns
        .iter_mut()
        .chain(unknown.iter_mut())
        .chain(std::iter::once(&mut untracked))
    {
        column.sort_and_cap(limit);
    }

    unknown.sort_by(|a, b| a.status.cmp(&b.status));

    Ok(Board {
        columns,
        untracked,
        unknown,
        closed,
    })
}

impl Column {
    /// An empty column under `status`.
    fn named(status: &str) -> Self {
        Self {
            status: status.to_owned(),
            cards: Vec::new(),
            truncated: 0,
        }
    }

    /// Order the column newest first (ties broken by path, so the ordering is deterministic) and cap it at `limit`
    /// (`0` for no cap), remembering how many that left out.
    fn sort_and_cap(&mut self, limit: usize) {
        self.cards.sort_by(|a, b| {
            b.file
                .modified
                .cmp(&a.file.modified)
                .then_with(|| a.file.path.cmp(&b.file.path))
        });

        if limit > 0 && self.cards.len() > limit {
            self.truncated = self.cards.len() - limit;
            self.cards.truncate(limit);
        }
    }
}

/// The statuses to build columns from, in the order to show them.
///
/// For one source that is simply its workflow. For all of them it is every folder's workflow in turn, in
/// configuration order and de-duplicated, so folders sharing a ladder share its columns and a folder with its own
/// ladder adds its steps after.
fn column_order(config: &Config, folder: Option<&str>) -> Vec<String> {
    if let Some(folder) = folder {
        return Workflow::for_source(config, folder).statuses().to_vec();
    }

    let mut order: Vec<String> = Vec::new();

    for folder in &config.custom_folders {
        for status in Workflow::for_folder(config, folder).statuses() {
            if !order.contains(status) {
                order.push(status.clone());
            }
        }
    }

    order
}

/// Whether `status` closes a note in the workflow the board is showing: the requested folder's, or any folder's when
/// the board spans them all.
///
/// Erring towards hiding keeps the common case right, where the folders sharing a status also agree on whether it
/// closes a note; a folder that disagrees still gets its column back through [`push`].
fn closes_a_note(config: &Config, folder: Option<&str>, status: &str) -> bool {
    if let Some(folder) = folder {
        return Workflow::for_source(config, folder).is_terminal(status);
    }

    config
        .custom_folders
        .iter()
        .any(|folder| Workflow::for_folder(config, folder).is_terminal(status))
}

/// File `card` under `status`, creating the column when it is one no workflow declared.
fn push(columns: &mut Vec<Column>, status: &str, card: Card) {
    if let Some(column) = columns.iter_mut().find(|column| column.status == status) {
        column.cards.push(card);

        return;
    }

    let mut column = Column::named(status);
    column.cards.push(card);
    columns.push(column);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FolderConfig;

    /// A folder with its own workflow.
    fn folder(name: &str, statuses: &[&str], terminal: &[&str]) -> FolderConfig {
        FolderConfig {
            name: name.to_owned(),
            statuses: statuses.iter().map(|status| (*status).to_owned()).collect(),
            terminal_statuses: terminal.iter().map(|status| (*status).to_owned()).collect(),
            ..FolderConfig::default()
        }
    }

    /// Two folders whose ladders overlap, which is what makes the union order worth checking.
    fn config() -> Config {
        Config {
            custom_folders: vec![
                folder("idea", &["backlog", "todo", "done"], &["done"]),
                folder("ticket", &["backlog", "todo", "staging", "prod"], &["prod"]),
            ],
            ..Config::default()
        }
    }

    #[test]
    fn columns_for_one_folder_are_its_own_workflow() {
        assert_eq!(
            column_order(&config(), Some("ticket")),
            ["backlog", "todo", "staging", "prod"]
        );
    }

    #[test]
    fn columns_for_every_folder_are_the_union_in_declaration_order() {
        assert_eq!(
            column_order(&config(), None),
            ["backlog", "todo", "done", "staging", "prod"]
        );
    }

    #[test]
    fn the_journal_has_no_columns_of_its_own() {
        assert!(column_order(&config(), Some(notes::JOURNAL_SOURCE)).is_empty());
    }

    #[test]
    fn a_status_closes_a_note_when_any_folder_showing_it_says_so() {
        let config = config();

        assert!(closes_a_note(&config, None, "done"));
        assert!(closes_a_note(&config, None, "prod"));
        assert!(!closes_a_note(&config, None, "todo"));
        // Scoped to one folder, only that folder's own terminal statuses count.
        assert!(!closes_a_note(&config, Some("ticket"), "done"));
        assert!(closes_a_note(&config, Some("ticket"), "prod"));
    }
}

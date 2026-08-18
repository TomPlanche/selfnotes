//! Carrying the previous day's checklist forward into a new journal entry.
//!
//! A day rarely ends with everything ticked off, so a new entry starts from the last one: a journal template pulls its
//! checkboxes in with `{{last_day.tasks}}` (every item, ticked or not) and `{{last_day.todo}}` (only what is still
//! unfinished), and names the day they came from with `{{last_day.weekday}}` and `{{last_day.date}}`. The checkboxes
//! are read from one section of the most recent entry before the new day, named by the `journal.carry_over_section`
//! config key.

use std::path::{Path, PathBuf};

use chrono::NaiveDate;

/// Prefix the carried values are exposed under, so a template reaches them as `{{last_day.<key>}}`.
pub const PREFIX: &str = "last_day";

/// Indentation added per nesting level when rendering a carried list.
const INDENT: &str = "  ";

/// Width a tab counts for when measuring a source line's indentation.
const TAB_WIDTH: usize = 4;

/// Rendered when nothing is carried over, so the section keeps the empty bullet an untouched template would leave.
const EMPTY_LIST: &str = "-";

/// Rendered by `{{last_day.weekday}}` when there is no previous entry to name, so the section still has a heading.
const NO_PREVIOUS_DAY: &str = "Last day";

/// A checkbox item read out of a journal entry.
#[derive(Debug, PartialEq, Eq)]
struct Task {
    /// Indentation width of the source line, which is what the nesting is rebuilt from.
    indent: usize,
    /// Whether the box was ticked.
    checked: bool,
    /// Everything after the checkbox.
    text: String,
}

/// The values a journal template can reach as `{{last_day.<key>}}`, taken from the most recent entry before `before`.
///
/// `tasks` is the whole checklist of its `section` and `todo` only the unfinished items, so a template can show what
/// the last day held alongside what it left behind. `weekday` and `date` name that day, since a section headed
/// `Wednesday` should say which entry it was actually filled from.
pub fn last_day_fields(root: &Path, format: &str, section: &str, before: NaiveDate) -> Vec<(String, String)> {
    let previous = previous_entry(root, format, before);
    let tasks = previous
        .as_ref()
        .map(|(_, path)| tasks_in(path, section))
        .unwrap_or_default();

    vec![
        (
            "weekday".to_string(),
            previous.as_ref().map_or_else(
                || NO_PREVIOUS_DAY.to_string(),
                |(date, _)| date.format("%A").to_string(),
            ),
        ),
        (
            "date".to_string(),
            previous
                .as_ref()
                .map_or_else(String::new, |(date, _)| date.format("%Y-%m-%d").to_string()),
        ),
        ("tasks".to_string(), render(tasks.iter())),
        ("todo".to_string(), render(tasks.iter().filter(|task| !task.checked))),
    ]
}

/// Tasks in the `section` of the entry at `path`.
///
/// An unreadable entry yields no tasks rather than an error: the new day is still worth creating.
fn tasks_in(path: &Path, section: &str) -> Vec<Task> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let (body, _) = crate::notes::body(&content);

    tasks_in_section(body, section)
}

/// The most recent journal entry strictly before `before`, with the day it is for.
///
/// Entries live at `<root>/YYYY/MM/DD.<format>`, so the search walks those directories highest-first and stops at the
/// first day it can use, instead of guessing how far back the last entry is: a Monday picks up the Friday before it,
/// and a gap of any length is crossed in one step. The date comes back with the path because it names the day the
/// carried checklist belongs to, which is the entry that exists rather than the calendar day before.
fn previous_entry(root: &Path, format: &str, before: NaiveDate) -> Option<(NaiveDate, PathBuf)> {
    for (year, year_dir) in numbered_dirs(root) {
        let Ok(year) = i32::try_from(year) else {
            continue;
        };

        for (month, month_dir) in numbered_dirs(&year_dir) {
            for (day, path) in numbered_files(&month_dir, format) {
                if let Some(date) = NaiveDate::from_ymd_opt(year, month, day)
                    && date < before
                {
                    return Some((date, path));
                }
            }
        }
    }

    None
}

/// Sub-directories of `dir` whose whole name is a number, highest first.
fn numbered_dirs(dir: &Path) -> Vec<(u32, PathBuf)> {
    let mut found: Vec<(u32, PathBuf)> = children(dir)
        .filter(|path| path.is_dir())
        .filter_map(|path| Some((path.file_name()?.to_str()?.parse().ok()?, path)))
        .collect();

    found.sort_by(|(a, _), (b, _)| b.cmp(a));

    found
}

/// Files in `dir` named `<number>.<format>`, highest first.
fn numbered_files(dir: &Path, format: &str) -> Vec<(u32, PathBuf)> {
    let mut found: Vec<(u32, PathBuf)> = children(dir)
        .filter(|path| path.extension().and_then(std::ffi::OsStr::to_str) == Some(format))
        .filter_map(|path| Some((path.file_stem()?.to_str()?.parse().ok()?, path)))
        .collect();

    found.sort_by(|(a, _), (b, _)| b.cmp(a));

    found
}

/// The paths directly under `dir`; an unreadable or missing directory simply has none.
fn children(dir: &Path) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
}

/// Every checkbox item under the `section` header of `body`, in source order.
///
/// The header is matched on its text alone, so both a `## Today` heading and a `- Today:` list item name the same
/// section. Nothing outside it is read, which is what keeps a template's own "last day" section from being carried
/// forward a second time.
fn tasks_in_section(body: &str, section: &str) -> Vec<Task> {
    let section = section.trim().trim_end_matches(':');
    if section.is_empty() {
        return Vec::new();
    }

    let lines: Vec<&str> = body.lines().collect();
    let Some(start) = lines.iter().position(|line| is_header(line, section)) else {
        return Vec::new();
    };
    let header = lines[start];

    lines[start + 1..]
        .iter()
        .take_while(|line| !ends_section(line, header))
        .filter_map(|line| parse_task(line))
        .collect()
}

/// Whether `line` is the header of the `section` (already trimmed of its own trailing colon).
fn is_header(line: &str, section: &str) -> bool {
    header_text(line)
        .trim_end_matches(':')
        .trim()
        .eq_ignore_ascii_case(section)
}

/// A line's text with any leading heading marker or list bullet removed.
fn header_text(line: &str) -> &str {
    let trimmed = line.trim();

    if let Some(level) = heading_level(trimmed) {
        return trimmed[level..].trim();
    }

    trimmed.strip_prefix(['-', '*', '+']).unwrap_or(trimmed).trim()
}

/// Whether `line` closes the section opened by `header`.
fn ends_section(line: &str, header: &str) -> bool {
    // A heading's section runs until the next heading of the same or a higher level.
    if let Some(level) = heading_level(header) {
        return heading_level(line).is_some_and(|next| next <= level);
    }

    // A list item's section is whatever is indented under it, blank lines included.
    !line.trim().is_empty() && indent_width(line) <= indent_width(header)
}

/// The ATX heading level of `line` (`## Today` is 2), or `None` when it is not a heading.
///
/// The `#`s must be followed by whitespace, so an inline `#tag` at the start of a line stays a tag.
fn heading_level(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|&c| c == '#').count();

    if !(1..=6).contains(&level) {
        return None;
    }

    let rest = &trimmed[level..];

    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then_some(level)
}

/// Parse a checkbox line such as `- [ ] write it up`, or `None` for any other line.
fn parse_task(line: &str) -> Option<Task> {
    let after_bullet = line.trim_start().strip_prefix(['-', '*', '+'])?;
    // A bullet needs whitespace after it, so `-[x]` and a stray `--` are not tasks.
    let (mark, text) = after_bullet
        .strip_prefix([' ', '\t'])?
        .trim_start()
        .strip_prefix('[')?
        .split_once(']')?;

    let checked = match mark.trim() {
        "" => false,
        // Any other single-character mark counts as finished: `[x]`, and conventions like `[-]` for cancelled. It
        // stays in the day's list but is not carried into the new one. Anything longer is a `[link]`, not a checkbox.
        other if other.chars().count() == 1 => true,
        _ => return None,
    };

    Some(Task {
        indent: indent_width(line),
        checked,
        text: text.trim().to_string(),
    })
}

/// Indentation width of `line`, counting a tab as [`TAB_WIDTH`] spaces.
fn indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { TAB_WIDTH } else { 1 })
        .sum()
}

/// Render `tasks` as a checklist indented by [`INDENT`] per level, or [`EMPTY_LIST`] when there is nothing to render.
///
/// Nesting follows the source indentation, but only among the tasks actually rendered: dropping a finished parent
/// promotes its unfinished children to the parent's level instead of leaving them dangling under nothing.
fn render<'a>(tasks: impl Iterator<Item = &'a Task>) -> String {
    // Source indentation of each rendered ancestor, so a task's depth is a count of kept parents rather than raw
    // whitespace.
    let mut ancestors: Vec<usize> = Vec::new();
    let mut lines: Vec<String> = Vec::new();

    for task in tasks {
        while ancestors.last().is_some_and(|&indent| indent >= task.indent) {
            ancestors.pop();
        }

        let mark = if task.checked { "x" } else { " " };
        let mut line = format!("{}- [{mark}]", INDENT.repeat(ancestors.len()));

        if !task.text.is_empty() {
            line.push(' ');
            line.push_str(&task.text);
        }

        lines.push(line);
        ancestors.push(task.indent);
    }

    if lines.is_empty() {
        return EMPTY_LIST.to_string();
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of a journal entry written from the default template.
    const ENTRY: &str = "\
# Daily 2026-08-17

- Last day:
  - [x] older thing

- Today:
  - [x] `appBooking`
    - [x] `/grouped` prod
    - [ ] `probes` staging
  - [ ] `CallExternalPusher` probes.
  - a plain note, not a task
";

    fn task(indent: usize, checked: bool, text: &str) -> Task {
        Task {
            indent,
            checked,
            text: text.to_string(),
        }
    }

    #[test]
    fn parses_checkbox_lines() {
        assert_eq!(parse_task("- [ ] open"), Some(task(0, false, "open")));
        assert_eq!(parse_task("  - [x] done"), Some(task(2, true, "done")));
        // Any bullet, any case, and an empty `[]` counts as unfinished.
        assert_eq!(parse_task("* [X] done"), Some(task(0, true, "done")));
        assert_eq!(parse_task("\t+ [] open"), Some(task(4, false, "open")));
        // A `[-]` cancelled item is finished for carry-over purposes.
        assert_eq!(parse_task("- [-] dropped"), Some(task(0, true, "dropped")));
    }

    #[test]
    fn ignores_lines_that_are_not_tasks() {
        for line in [
            "",
            "  plain text",
            "- plain bullet",
            "-[x] no space",
            "- [[note]] a link",
            "# Today",
        ] {
            assert_eq!(parse_task(line), None, "expected `{line}` not to parse as a task");
        }
    }

    #[test]
    fn reads_only_the_named_section() {
        let tasks = tasks_in_section(ENTRY, "Today");

        assert_eq!(
            tasks,
            vec![
                task(2, true, "`appBooking`"),
                task(4, true, "`/grouped` prod"),
                task(4, false, "`probes` staging"),
                task(2, false, "`CallExternalPusher` probes."),
            ]
        );
    }

    #[test]
    fn stops_at_the_next_section() {
        // `Last day` comes first, so its single task is all that belongs to it.
        assert_eq!(tasks_in_section(ENTRY, "Last day"), vec![task(2, true, "older thing")]);
    }

    #[test]
    fn matches_a_heading_section_and_stops_at_the_next_heading() {
        let body = "## Today\n\n- [ ] one\n\n### Notes\n\n- [ ] two\n\n## Tomorrow\n\n- [ ] three\n";

        // The `### Notes` sub-heading stays inside the section; the sibling `## Tomorrow` closes it.
        assert_eq!(
            tasks_in_section(body, "Today"),
            vec![task(0, false, "one"), task(0, false, "two")]
        );
    }

    #[test]
    fn unknown_or_empty_section_yields_nothing() {
        assert!(tasks_in_section(ENTRY, "Tomorrow").is_empty());
        assert!(tasks_in_section(ENTRY, "   ").is_empty());
    }

    #[test]
    fn section_name_ignores_case_and_a_trailing_colon() {
        assert_eq!(tasks_in_section(ENTRY, "today:").len(), 4);
    }

    #[test]
    fn renders_the_whole_list_with_its_nesting() {
        let tasks = tasks_in_section(ENTRY, "Today");

        assert_eq!(
            render(tasks.iter()),
            "- [x] `appBooking`\n  - [x] `/grouped` prod\n  - [ ] `probes` staging\n- [ ] `CallExternalPusher` probes."
        );
    }

    #[test]
    fn unfinished_children_are_promoted_past_a_finished_parent() {
        let tasks = tasks_in_section(ENTRY, "Today");

        // `probes` staging outlived its ticked parent, so it carries over at the top level.
        assert_eq!(
            render(tasks.iter().filter(|task| !task.checked)),
            "- [ ] `probes` staging\n- [ ] `CallExternalPusher` probes."
        );
    }

    #[test]
    fn nesting_survives_when_a_parent_is_kept() {
        let body = "- Today:\n  - [ ] parent\n    - [ ] child\n    - [x] done child\n";
        let tasks = tasks_in_section(body, "Today");

        assert_eq!(
            render(tasks.iter().filter(|task| !task.checked)),
            "- [ ] parent\n  - [ ] child"
        );
    }

    #[test]
    fn nothing_to_carry_renders_an_empty_bullet() {
        assert_eq!(render(std::iter::empty()), EMPTY_LIST);
    }

    /// A journal tree that is deleted when the test ends.
    struct TempJournal(PathBuf);

    impl TempJournal {
        fn new(label: &str) -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!("selfnotes-{label}-{}-{stamp}", std::process::id()));

            std::fs::create_dir_all(&root).unwrap();

            Self(root)
        }

        /// Write an entry at `<root>/YYYY/MM/DD.md`.
        fn write(&self, year: i32, month: u32, day: u32, content: &str) {
            let dir = self.0.join(format!("{year:04}")).join(format!("{month:02}"));

            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{day:02}.md")), content).unwrap();
        }
    }

    impl Drop for TempJournal {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn previous_entry_is_the_most_recent_earlier_day() {
        let journal = TempJournal::new("previous");

        journal.write(2025, 12, 31, "");
        journal.write(2026, 1, 5, "");
        journal.write(2026, 1, 9, "");
        // A future entry, which no earlier day should ever pick up.
        journal.write(2026, 2, 2, "");

        // The 9th is skipped for its own day and found from the next one, across a month and a year boundary.
        assert_eq!(
            previous_entry(&journal.0, "md", date(2026, 1, 9)),
            Some((date(2026, 1, 5), journal.0.join("2026/01/05.md")))
        );
        assert_eq!(
            previous_entry(&journal.0, "md", date(2026, 1, 20)),
            Some((date(2026, 1, 9), journal.0.join("2026/01/09.md")))
        );
        assert_eq!(
            previous_entry(&journal.0, "md", date(2026, 1, 3)),
            Some((date(2025, 12, 31), journal.0.join("2025/12/31.md")))
        );
        // Nothing precedes the very first entry.
        assert_eq!(previous_entry(&journal.0, "md", date(2025, 12, 31)), None);
    }

    #[test]
    fn previous_entry_ignores_other_formats_and_stray_names() {
        let journal = TempJournal::new("formats");

        journal.write(2026, 1, 5, "");
        std::fs::write(journal.0.join("2026/01/06.txt"), "").unwrap();
        std::fs::write(journal.0.join("2026/01/notes.md"), "").unwrap();

        assert_eq!(
            previous_entry(&journal.0, "md", date(2026, 1, 20)),
            Some((date(2026, 1, 5), journal.0.join("2026/01/05.md")))
        );
    }

    #[test]
    fn fields_carry_the_last_entry_forward() {
        let journal = TempJournal::new("fields");

        journal.write(2026, 8, 17, &format!("+++\ntags = [\"daily\"]\n+++\n\n{ENTRY}"));

        let fields = last_day_fields(&journal.0, "md", "Today", date(2026, 8, 18));
        let value = |key: &str| {
            fields
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
                .unwrap()
        };

        assert!(value("tasks").starts_with("- [x] `appBooking`"));
        assert_eq!(
            value("todo"),
            "- [ ] `probes` staging\n- [ ] `CallExternalPusher` probes."
        );
        // The day named is the entry the checklist came from, not the calendar day before.
        assert_eq!(value("weekday"), "Monday");
        assert_eq!(value("date"), "2026-08-17");
    }

    #[test]
    fn the_named_day_skips_over_a_gap() {
        let journal = TempJournal::new("gap");

        journal.write(2026, 8, 6, ENTRY);

        // Nothing was written between the 6th (a Thursday) and the 13th, so that is the day the list comes from.
        let fields = last_day_fields(&journal.0, "md", "Today", date(2026, 8, 13));

        assert!(fields.contains(&("weekday".to_string(), "Thursday".to_string())));
        assert!(fields.contains(&("date".to_string(), "2026-08-06".to_string())));
    }

    #[test]
    fn a_journal_with_no_earlier_entry_carries_nothing() {
        let journal = TempJournal::new("empty");

        let fields = last_day_fields(&journal.0, "md", "Today", date(2026, 8, 18));

        assert_eq!(
            fields,
            vec![
                ("weekday".to_string(), NO_PREVIOUS_DAY.to_string()),
                ("date".to_string(), String::new()),
                ("tasks".to_string(), EMPTY_LIST.to_string()),
                ("todo".to_string(), EMPTY_LIST.to_string()),
            ]
        );
    }
}

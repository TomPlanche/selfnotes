//! `selfnotes`: a CLI that manages a journal-style notes filesystem.
//! `selfnotes -h` for full usage information.

mod carryover;
mod cli;
mod config;
mod date;
mod entry;
mod import;
mod list;
mod lsp;
mod notes;
mod people;
mod search;
mod template;

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use chrono::{DateTime, Local, NaiveDate};
use clap::Parser;
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Input, Select};

use cli::{Cli, Command, ConfigAction, ConfigScope, ImportFormat, PeopleAction, TagSort};
use config::{Config, FolderConfig};
use entry::Entry;
use notes::{Index, IndexedNote};

fn main() -> Result<()> {
    let args = Cli::parse();
    // A bare invocation creates today's journal entry.
    let command = args.command.unwrap_or(Command::Journal {
        date: None,
        no_open: false,
    });

    match command {
        Command::Journal { date, no_open } => {
            let config = config::load()?;
            let date = journal_date(date.as_deref())?;
            let entry = entry::create_journal(&config, date)?;

            report(&entry);
            maybe_open(&config, &entry, no_open);
        },
        Command::New { folder, name, no_open } => {
            let config = config::load()?;
            if config.custom_folders.is_empty() {
                bail!("no custom folders are configured; add a [[custom_folders]] entry to your config");
            }

            let folder_config = match folder {
                Some(folder) => config
                    .folder(&folder)
                    .with_context(|| format!("no folder `{folder}` is configured"))?
                    .clone(),
                None => select_folder(&config)?,
            };

            // Validate the target directory (which does not depend on the entry name) before prompting, so a
            // misconfigured folder fails immediately instead of after the user fills everything in.
            entry::folder_dir(&config, &folder_config)?;

            let name = match name {
                Some(name) => name,
                None => prompt_name()?,
            };
            let name = name.trim();

            if name.is_empty() {
                bail!("entry name cannot be empty");
            }
            // Reject a traversal-laden name before prompting for the folder's fields.
            entry::validate_entry_name(name)?;

            let fields = prompt_fields(&folder_config)?;

            let entry = entry::create_folder_entry(&config, &folder_config, name, fields)?;
            report(&entry);
            maybe_open(&config, &entry, no_open);
        },
        Command::List { limit, folder, tags } => {
            let config = config::load()?;
            let listings = list::recent(&config, folder.as_deref(), limit, &tags)?;

            print_listings(&config, &listings)?;
        },
        Command::Search {
            query,
            limit,
            folder,
            tags,
            context,
            case_sensitive,
            files,
        } => {
            let config = config::load()?;
            let hits = search::search(
                &config,
                &search::Query {
                    text: &query,
                    folder: folder.as_deref(),
                    tags: &tags,
                    case_sensitive,
                    context,
                    limit,
                },
            )?;

            print_search(&config, &hits, files)?;
        },
        Command::Tags { folder, sort } => {
            let config = config::load()?;
            let index = notes::build_index(&config, folder.as_deref())?;

            print_tags(&index, sort);
        },
        Command::Links { name } => {
            let config = config::load()?;
            let index = notes::build_index(&config, None)?;

            print_links(&config, &index, &name)?;
        },
        Command::Open { name } => {
            let config = config::load()?;
            let index = notes::build_index(&config, None)?;

            open_note(&config, &index, &name)?;
        },
        Command::People { action } => run_people(action)?,
        Command::Lsp => lsp::run()?,
        Command::Config { action } => run_config(action)?,
    }

    Ok(())
}

/// Resolve the requested journal date against today's, defaulting to today when `--date` was not given.
fn journal_date(spec: Option<&str>) -> Result<NaiveDate> {
    let today = Local::now().date_naive();

    spec.map_or(Ok(today), |spec| date::parse(spec, today))
}

/// Print recent entries as `<modified>  <source>  <relative-path>`, newest first.
///
/// Paths are shown relative to the journal root so the list stays compact; the source column is padded so the paths
/// line up.
fn print_listings(config: &Config, listings: &[notes::NoteFile]) -> Result<()> {
    if listings.is_empty() {
        println!("No entries found.");

        return Ok(());
    }

    let root = config.resolved_journal_root()?;
    let width = listings.iter().map(|listing| listing.source.len()).max().unwrap_or(0);

    for listing in listings {
        let when = DateTime::<Local>::from(listing.modified).format("%Y-%m-%d %H:%M");
        let shown = listing.path.strip_prefix(&root).unwrap_or(&listing.path).display();

        println!("{when}  {:<width$}  {shown}", listing.source);
    }

    Ok(())
}

/// Print search hits: a header per note, then its matching lines.
///
/// Each line is prefixed with its file line number and a marker, `:` for a match and `-` for context, so the two are
/// distinguishable when several lines are shown. `files_only` reduces the output to bare paths, one per line, for
/// piping into other tools.
fn print_search(config: &Config, hits: &[search::Hit], files_only: bool) -> Result<()> {
    if hits.is_empty() {
        println!("No matches found.");

        return Ok(());
    }

    let root = config.resolved_journal_root()?;
    let rel = |path: &Path| path.strip_prefix(&root).unwrap_or(path).display().to_string();

    if files_only {
        for hit in hits {
            println!("{}", rel(&hit.file.path));
        }

        return Ok(());
    }

    for (index, hit) in hits.iter().enumerate() {
        // Blank line between notes, so the headers stand out from the lines beneath them.
        if index > 0 {
            println!();
        }

        let count = hit.matches();
        let lines = if count == 1 { "line" } else { "lines" };
        let title = hit
            .title
            .as_ref()
            .map_or_else(String::new, |title| format!("  ({title})"));

        println!("{}  {}{title}  [{count} {lines}]", hit.file.source, rel(&hit.file.path));

        for (nth, snippet) in hit.snippets.iter().enumerate() {
            // Mark the gap between snippets, which skips at least one line.
            if nth > 0 {
                println!("  ...");
            }

            for line in &snippet.lines {
                let marker = if line.matched { ':' } else { '-' };

                // A blank line prints bare, so the output carries no trailing whitespace.
                if line.text.is_empty() {
                    println!("  {:>5}{marker}", line.number);
                } else {
                    println!("  {:>5}{marker} {}", line.number, line.text);
                }
            }
        }
    }

    Ok(())
}

/// Print every tag with the number of notes using it, ordered per `sort`.
fn print_tags(index: &Index, sort: TagSort) {
    let mut counts = index.tag_counts();

    if counts.is_empty() {
        println!("No tags found.");

        return;
    }

    // `tag_counts` is already alphabetical; only the count ordering needs a re-sort (ties fall back to the name).
    if matches!(sort, TagSort::Count) {
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    }

    let width = counts
        .iter()
        .map(|(_, count)| count.to_string().len())
        .max()
        .unwrap_or(0);

    for (tag, count) in counts {
        println!("{count:>width$}  #{tag}");
    }
}

/// Print a note's outbound links (with where each resolves) and its backlinks, paths shown relative to the root.
fn print_links(config: &Config, index: &Index, name: &str) -> Result<()> {
    let note = resolve_one(index, name)?;
    let root = config.resolved_journal_root()?;
    let rel = |path: &Path| path.strip_prefix(&root).unwrap_or(path).display().to_string();

    match &note.title {
        Some(title) => println!("{}  ({title})", rel(&note.file.path)),
        None => println!("{}", rel(&note.file.path)),
    }

    println!();
    println!("Outbound links:");
    if note.links.is_empty() {
        println!("  (none)");
    } else {
        for link in &note.links {
            let status = match index.resolve(&link.target).as_slice() {
                [] => "unresolved".to_string(),
                [one] => rel(&one.file.path),
                many => format!("ambiguous ({} matches)", many.len()),
            };

            println!("  [[{}]] -> {status}", link.target);
        }
    }

    println!();
    println!("Backlinks:");
    let backlinks = index.backlinks(&note.file.path);
    if backlinks.is_empty() {
        println!("  (none)");
    } else {
        for backlink in backlinks {
            println!("  {}", rel(&backlink.file.path));
        }
    }

    Ok(())
}

/// Resolve `name` to a single note and open it in the editor.
fn open_note(config: &Config, index: &Index, name: &str) -> Result<()> {
    let note = resolve_one(index, name)?;
    let root = config.resolved_journal_root()?;

    println!("Opening {}", note.file.path.display());
    entry::open_in_editor(config, &root, &note.file.path, None)
}

/// Resolve a note name to exactly one note, erroring (and listing candidates) when it is missing or ambiguous.
fn resolve_one<'a>(index: &'a Index, name: &str) -> Result<&'a IndexedNote> {
    match index.resolve(name).as_slice() {
        [] => bail!("no note matches `{name}`"),
        [one] => Ok(one),
        many => {
            let candidates = many
                .iter()
                .map(|note| {
                    let path = note.file.path.display();

                    note.title
                        .as_ref()
                        .map_or_else(|| format!("  {path}"), |title| format!("  {path}  ({title})"))
                })
                .collect::<Vec<_>>()
                .join("\n");

            bail!("`{name}` is ambiguous ({} matches):\n{candidates}", many.len())
        },
    }
}

/// Print a message describing what happened to an entry.
fn report(entry: &Entry) {
    let verb = if entry.created { "Created" } else { "Already exists" };

    println!("{verb}: {}", entry.path.display());
}

/// Open the entry in an editor unless the user opted out.
///
/// The entry file is already created and reported, so a failure here (missing editor, no terminal) is surfaced as a
/// warning rather than failing the run.
fn maybe_open(config: &Config, entry: &Entry, no_open: bool) {
    if no_open {
        return;
    }

    let root = match config.resolved_journal_root() {
        Ok(root) => root,
        Err(err) => {
            eprintln!("warning: could not open editor: {err:#}");

            return;
        },
    };

    if let Err(err) = entry::open_in_editor(config, &root, &entry.path, entry.cursor.as_ref()) {
        eprintln!("warning: could not open editor: {err:#}");
    }
}

/// Interactively pick one of the configured folders.
fn select_folder(config: &Config) -> Result<FolderConfig> {
    let names: Vec<&str> = config
        .custom_folders
        .iter()
        .map(|folder| folder.name.as_str())
        .collect();
    let choice = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Folder")
        .items(&names)
        .default(0)
        .interact()
        .context("selecting a folder")?;

    Ok(config.custom_folders[choice].clone())
}

/// Interactively prompt for an entry name.
fn prompt_name() -> Result<String> {
    Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Entry name")
        .interact_text()
        .context("reading entry name")
}

/// Prompt for each of a folder's custom fields, returning `(name, value)` pairs ready to hand to the template context.
fn prompt_fields(folder: &FolderConfig) -> Result<Vec<(String, String)>> {
    let mut values = Vec::with_capacity(folder.fields.len());
    let theme = ColorfulTheme::default();

    for field in folder.ordered_fields() {
        let prompt = field.prompt.as_deref().unwrap_or(&field.name);
        let input = Input::<String>::with_theme(&theme)
            .with_prompt(prompt)
            .allow_empty(true);
        let input = match &field.default {
            Some(default) => input.default(default.clone()),
            None => input,
        };

        let value = input
            .interact_text()
            .with_context(|| format!("reading field `{}`", field.name))?;

        values.push((field.name.clone(), value));
    }

    Ok(values)
}

/// Handle the `people` subcommand.
fn run_people(action: Option<PeopleAction>) -> Result<()> {
    let config = config::load()?;
    let path = people::path(&config).context("could not determine where the people file lives")?;

    match action {
        Some(PeopleAction::Path) => println!("{}", path.display()),
        Some(PeopleAction::Open) => {
            if !path.exists() {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("creating people directory {}", parent.display()))?;
                }

                std::fs::write(&path, people::TEMPLATE)
                    .with_context(|| format!("writing people file {}", path.display()))?;
                println!("Created {}", path.display());
            }

            return entry::open_paths_in_editor(&config, &[&path]);
        },
        Some(PeopleAction::Import { format, prune, dry_run }) => return import_people(&path, format, prune, dry_run),
        None => print_people(&people::read(&path)?, &path),
    }

    Ok(())
}

/// Handle `people import`: fold a directory export arriving on standard input into the roster.
fn import_people(path: &Path, format: ImportFormat, prune: bool, dry_run: bool) -> Result<()> {
    let format = match format {
        ImportFormat::Json => import::Format::Json,
        ImportFormat::Tsv => import::Format::Tsv,
        ImportFormat::Csv => import::Format::Csv,
    };

    let input = std::io::read_to_string(std::io::stdin()).context("reading standard input")?;
    if input.trim().is_empty() {
        bail!("nothing on standard input; pipe a directory export into `selfnotes people import`");
    }

    let (records, skipped) = import::parse(&input, format)?;
    // The file is read as text, not through `people::read`, because the merge edits that text in place.
    let text = if path.exists() {
        std::fs::read_to_string(path).with_context(|| format!("reading people file {}", path.display()))?
    } else {
        String::new()
    };
    let directory = toml::from_str(&text).with_context(|| format!("parsing people file {}", path.display()))?;

    let plan = import::plan(&directory, &records, skipped);
    report_import(&plan, prune);

    if plan.is_empty(prune) {
        println!(
            "
Nothing to do; {} is unchanged.",
            path.display()
        );

        return Ok(());
    }

    // Build the new text even for a dry run: it is what proves the edit is sound before anyone relies on the report.
    let updated = import::apply(&text, &plan, prune)?;

    if dry_run {
        println!(
            "
Dry run; {} was not written.",
            path.display()
        );

        return Ok(());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating people directory {}", parent.display()))?;
    }

    std::fs::write(path, updated).with_context(|| format!("writing people file {}", path.display()))?;
    println!(
        "
Updated {}",
        path.display()
    );

    Ok(())
}

/// Print what an import found, before anything is written.
fn report_import(plan: &import::Plan, prune: bool) {
    let read = plan.added.len() + plan.present.len();
    match plan.skipped {
        0 => println!("{read} people read from standard input."),
        skipped => println!("{read} people read from standard input, {skipped} skipped (no handle, or not active)."),
    }

    if !plan.added.is_empty() {
        println!(
            "  + {} added: {}",
            plan.added.len(),
            handle_list(plan.added.iter().map(|record| &record.handle))
        );
    }

    if !plan.present.is_empty() {
        println!("    {} already in the roster, left untouched", plan.present.len());
    }

    if !plan.missing.is_empty() {
        let verb = if prune { "removed" } else { "not in the source" };

        println!(
            "  - {} {verb}: {}",
            plan.missing.len(),
            handle_list(plan.missing.iter())
        );

        if !prune {
            println!("    (remove them with --prune)");
        }
    }
}

/// Render handles as a comma-separated `@name` list, abbreviated once it gets long.
fn handle_list<'a>(handles: impl ExactSizeIterator<Item = &'a String>) -> String {
    const SHOWN: usize = 8;

    let total = handles.len();
    let mut names: Vec<String> = handles.take(SHOWN).map(|handle| format!("@{handle}")).collect();

    if total > SHOWN {
        names.push(format!("and {} more", total - SHOWN));
    }

    names.join(", ")
}

/// Print the roster, flagging any handle that could never be typed after an `@`.
fn print_people(directory: &people::Directory, path: &Path) {
    if directory.people.is_empty() {
        println!("No people in {}", path.display());
        println!("Add some with `selfnotes people open`.");

        return;
    }

    let width = directory
        .people
        .iter()
        .map(|person| person.handle.chars().count())
        .max()
        .unwrap_or(0);

    for person in &directory.people {
        println!("@{:<width$}  {}", person.handle, person.detail());

        if !person.has_usable_handle() {
            println!(
                "  warning: `{}` cannot be typed after an `@`, so it is never completed",
                person.handle
            );
        }
    }
}

/// Handle the `config` subcommand.
fn run_config(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::New => return new_config(),
        ConfigAction::Open { scope } => return open_config(scope),
        ConfigAction::Validate => return validate_config(),
        ConfigAction::Path => {
            match config::global_config_path() {
                Some(path) => println!("global: {}", path.display()),
                None => println!("global: <unavailable>"),
            }
            match config::find_local_config(&std::env::current_dir()?) {
                Some(path) => println!("local:  {}", path.display()),
                None => println!("local:  <none>"),
            }

            let merged = config::load()?;

            println!();
            println!("effective values:");
            println!(
                "  journal-root      = {}",
                merged.journal_root.as_deref().unwrap_or("<unset>")
            );
            println!("  format            = {}", merged.journal_format());
            println!(
                "  editor            = {}",
                merged.editor.as_deref().unwrap_or("<unset>")
            );
            println!("  cursor-format     = {}", merged.cursor_format());
            // Quoted, since the empty string is a meaningful value here and would otherwise read as unset.
            println!(
                "  space-replacement = {}",
                merged
                    .space_replacement
                    .as_deref()
                    .map_or_else(|| "<unset>".to_owned(), |value| format!("\"{value}\""))
            );
            println!(
                "  people-file       = {}",
                people::path(&merged).map_or_else(|| "<unavailable>".to_owned(), |path| path.display().to_string())
            );
        },
        ConfigAction::Get { key } => {
            let config = config::load()?;

            let value = match key.as_str() {
                "journal-root" => config.journal_root,
                "format" => Some(config.journal_format().to_string()),
                "editor" => config.editor,
                "cursor-format" => Some(config.cursor_format().to_string()),
                "space-replacement" => config.space_replacement,
                "hash-tag-min-len" => Some(config.hash_tag_min_len().to_string()),
                "people-file" => people::path(&config).map(|path| path.display().to_string()),
                other => bail!("unknown config key `{other}`"),
            };

            match value {
                Some(value) => println!("{value}"),
                None => println!("<unset>"),
            }
        },
        ConfigAction::Set { key, value } => {
            let mut config = config::load_global()?;

            match key.as_str() {
                "journal-root" => config.journal_root = Some(value),
                "format" => config.format = Some(value),
                "editor" => config.editor = Some(value),
                "cursor-format" => config.cursor_format = Some(value),
                "space-replacement" => config.space_replacement = Some(value),
                "people-file" => config.people_file = Some(value),
                "hash-tag-min-len" => {
                    config.hash_tag_min_len = Some(
                        value
                            .parse()
                            .context("hash-tag-min-len must be a non-negative integer")?,
                    );
                },
                other => bail!("unknown config key `{other}`"),
            }

            let path = config::save_global(&config)?;
            println!("Updated {}", path.display());
        },
    }
    Ok(())
}

/// Severity of a configuration problem.
enum Severity {
    Error,
    Warning,
}

/// A single configuration problem, attributed to the config file (or the effective configuration) it came from.
struct Problem {
    severity: Severity,
    /// The `source` a problem is attributed to: a config file's path, or [`EFFECTIVE`] for cross-file issues.
    source: String,
    message: String,
}

/// Attribution used for problems that belong to the merged configuration rather than any single file.
const EFFECTIVE: &str = "(effective configuration)";

#[derive(Default)]
struct Problems {
    items: Vec<Problem>,
}

impl Problems {
    fn error(&mut self, source: &str, message: String) {
        self.items.push(Problem {
            severity: Severity::Error,
            source: source.to_owned(),
            message,
        });
    }

    fn warn(&mut self, source: &str, message: String) {
        self.items.push(Problem {
            severity: Severity::Warning,
            source: source.to_owned(),
            message,
        });
    }

    /// Number of problems attributed to `source` at each severity, as `(errors, warnings)`.
    fn counts_for(&self, source: &str) -> (usize, usize) {
        self.items
            .iter()
            .filter(|problem| problem.source == source)
            .fold((0, 0), |(errors, warnings), problem| match problem.severity {
                Severity::Error => (errors + 1, warnings),
                Severity::Warning => (errors, warnings + 1),
            })
    }

    fn error_count(&self) -> usize {
        self.items
            .iter()
            .filter(|problem| matches!(problem.severity, Severity::Error))
            .count()
    }
}

/// A config file that participates in the effective configuration.
struct Layer {
    /// The file's path, used as the problem source and shown in the per-file summary.
    label: String,
    /// Whether this is a local `.selfnotes.toml` (overrides declared here are ignored at runtime).
    is_local: bool,
    config: Config,
}

/// Validate every config file that contributes to the effective configuration and report a verdict per file.
///
/// Each file is checked in isolation (folder names/directories, templates, and its `[[overrides]]` wiring), but folder
/// directories are resolved against the *effective* journal root so a layer that does not set its own root is still
/// checked correctly. A folder or template shadowed by a higher-priority layer is skipped, so only what actually takes
/// effect is judged. Errors make the command exit non-zero; softer issues are warnings.
fn validate_config() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let layers = participating_layers(&cwd)?;
    let mut problems = Problems::default();

    // Folder directories are resolved against the effective root, so resolve it once from the merged config.
    let root = match config::load()?.resolved_journal_root() {
        Ok(root) => Some(root),
        Err(err) => {
            problems.error(EFFECTIVE, format!("{err:#}"));

            None
        },
    };

    for (index, layer) in layers.iter().enumerate() {
        let later = &layers[index + 1..];

        // Checked wherever it is written, shadowed or not: a separator here is a mistake in this file either way.
        check_space_replacement(
            layer.config.space_replacement.as_deref(),
            "space_replacement",
            &layer.label,
            &mut problems,
        );

        // Only the highest-priority layer that sets a journal template is effective.
        let template_shadowed = later.iter().any(|layer| {
            layer
                .config
                .journal
                .as_ref()
                .and_then(|journal| journal.template_file.as_ref())
                .is_some()
        });
        if !template_shadowed {
            check_journal_template(&layer.config, &layer.label, &mut problems);
        }

        for folder in &layer.config.custom_folders {
            // A folder replaced by a same-named folder in a higher-priority layer does not take effect; skip it.
            let shadowed = later.iter().any(|layer| {
                layer
                    .config
                    .custom_folders
                    .iter()
                    .any(|other| other.name == folder.name)
            });
            if !shadowed {
                check_folder(folder, root.as_deref(), &layer.label, &mut problems);
            }
        }

        for over in &layer.config.overrides {
            check_override(over, layer.is_local, &layer.label, &cwd, &mut problems);
        }
    }

    report_problems(&layers, &problems)
}

/// Reconstruct, in precedence order (lowest first), the config files that `config::load` merges for `cwd`: the global
/// config, then each matching global override's referenced config, then the nearest local `.selfnotes.toml`. Missing
/// files are skipped.
fn participating_layers(cwd: &Path) -> Result<Vec<Layer>> {
    let mut layers = Vec::new();

    if let Some(path) = config::global_config_path()
        && let Some(config) = config::read_config_file(&path)?
    {
        let overrides = config.overrides.clone();
        layers.push(Layer {
            label: path.display().to_string(),
            is_local: false,
            config,
        });

        // Global overrides that match the current directory are layered between the global and local configs.
        for over in &overrides {
            if config::override_matches(over, cwd).unwrap_or(false) {
                let referenced = config::expand_tilde(&over.config);

                if let Some(config) = config::read_config_file(&referenced)? {
                    layers.push(Layer {
                        label: referenced.display().to_string(),
                        is_local: false,
                        config,
                    });
                }
            }
        }
    }

    if let Some(path) = config::find_local_config(cwd)
        && let Some(config) = config::read_config_file(&path)?
    {
        layers.push(Layer {
            label: path.display().to_string(),
            is_local: true,
            config,
        });
    }

    Ok(layers)
}

/// Print a verdict for each participating file, then the details, and return an error when any file is invalid.
fn report_problems(layers: &[Layer], problems: &Problems) -> Result<()> {
    let files = if layers.len() == 1 { "file" } else { "files" };
    println!("Checked {} config {files}:", layers.len());

    for layer in layers {
        let (errors, warnings) = problems.counts_for(&layer.label);
        let verdict = if errors > 0 { "INVALID" } else { "valid" };

        println!("  {verdict:>7}  {}{}", layer.label, count_suffix(errors, warnings));
    }

    let (effective_errors, effective_warnings) = problems.counts_for(EFFECTIVE);
    if effective_errors + effective_warnings > 0 {
        println!(
            "  {:>7}  {EFFECTIVE}{}",
            "",
            count_suffix(effective_errors, effective_warnings)
        );
    }

    if !problems.items.is_empty() {
        println!();

        for problem in &problems.items {
            let severity = match problem.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };

            println!("{severity}: {}: {}", problem.source, problem.message);
        }
    }

    let errors = problems.error_count();
    if errors > 0 {
        bail!("configuration has {errors} problem(s)");
    }

    println!();
    match problems.items.len() {
        0 => println!("All configs valid."),
        n => println!("All configs valid ({n} warning(s))."),
    }

    Ok(())
}

/// Format a `(errors, warnings)` count as a parenthesised suffix, e.g. ` (1 error, 2 warnings)`, or empty when both
/// are zero.
fn count_suffix(errors: usize, warnings: usize) -> String {
    let noun = |count: usize, singular: &str| {
        if count == 1 {
            format!("{count} {singular}")
        } else {
            format!("{count} {singular}s")
        }
    };

    match (errors, warnings) {
        (0, 0) => String::new(),
        (errors, 0) => format!(" ({})", noun(errors, "error")),
        (0, warnings) => format!(" ({})", noun(warnings, "warning")),
        (errors, warnings) => format!(" ({}, {})", noun(errors, "error"), noun(warnings, "warning")),
    }
}

/// Check a config's journal `template_file` (if any) exists, attributing any problem to `source`.
fn check_journal_template(config: &Config, source: &str, problems: &mut Problems) {
    if let Some(template) = config
        .journal
        .as_ref()
        .and_then(|journal| journal.template_file.as_deref())
        && !config::expand_tilde(template).exists()
    {
        problems.error(source, format!("journal: template file not found: {template}"));
    }
}

/// Check a single folder: no separator in its name, its directory stays under `root` (when known), its template
/// exists, and `field_order` names a declared field. Problems are attributed to `source`.
fn check_folder(folder: &FolderConfig, root: Option<&Path>, source: &str, problems: &mut Problems) {
    let name = &folder.name;

    if folder.name.contains('/') || folder.name.contains('\\') {
        problems.error(
            source,
            format!("folder `{name}`: name must not contain a path separator"),
        );
    }

    // Skip when the root is unresolved, otherwise this just repeats the root error for every folder.
    if let Some(root) = root
        && let Err(err) = entry::folder_dir_in(root, folder)
    {
        problems.error(source, format!("folder `{name}`: {err:#}"));
    }

    if let Some(template) = folder.template_file.as_deref()
        && !config::expand_tilde(template).exists()
    {
        problems.error(source, format!("folder `{name}`: template file not found: {template}"));
    }

    check_space_replacement(
        folder.space_replacement.as_deref(),
        &format!("folder `{name}`: space_replacement"),
        source,
        problems,
    );

    for field in &folder.field_order {
        if !folder.fields.iter().any(|declared| &declared.name == field) {
            problems.warn(
                source,
                format!("folder `{name}`: field_order references unknown field `{field}` (ignored)"),
            );
        }
    }
}

/// Check a `space_replacement`, described by `label`, cannot push an entry out of its folder.
///
/// The value is substituted into the file name, so a path separator in it turns `selfnotes new ticket "login bug"`
/// into a write one directory over. Creation rejects that, but only once a spaced name has been typed, so report the
/// value itself here.
fn check_space_replacement(value: Option<&str>, label: &str, source: &str, problems: &mut Problems) {
    if let Some(value) = value
        && (value.contains('/') || value.contains('\\'))
    {
        problems.error(source, format!("{label}: must not contain a path separator"));
    }
}

/// Check one `[[overrides]]` entry declared in the config file `source`.
///
/// An override in a local config is only reported (it is ignored at runtime). For a global override we validate the
/// glob, report whether it matches `cwd`, and check the referenced config file exists. The referenced config's own
/// contents are validated separately as a participating layer.
fn check_override(over: &config::Override, is_local: bool, source: &str, cwd: &Path, problems: &mut Problems) {
    if over.path.as_slice().is_empty() {
        problems.warn(
            source,
            format!(
                "override -> {} declares no glob, so it never applies",
                config::expand_tilde(&over.config).display()
            ),
        );

        return;
    }

    let matches = config::override_matches(over, cwd);

    if is_local {
        // Only the global config's overrides are applied, so surface the misplacement and whether the glob would even
        // match here, which is the usual reason a local override looks like it does nothing.
        let note = match matches {
            Ok(true) => "its glob matches the current directory",
            Ok(false) => "its glob does not match the current directory either",
            Err(_) => "its glob is invalid",
        };

        problems.warn(
            source,
            format!(
                "override for {} is ignored (overrides are only applied from the global config); {note}",
                over.path
            ),
        );

        return;
    }

    let matches = match matches {
        Ok(matches) => matches,
        Err(err) => {
            problems.error(source, format!("{err:#}"));

            return;
        },
    };

    let referenced = config::expand_tilde(&over.config);
    if !referenced.exists() {
        let message = format!(
            "override for {} -> config file not found: {}",
            over.path,
            referenced.display()
        );

        // A missing file the override actually selects here is a hard error; otherwise just a heads-up.
        if matches {
            problems.error(source, message);
        } else {
            problems.warn(source, message);
        }
    }
}

/// Handle `config new`: write a `.selfnotes.toml` in the current directory.
///
/// The local layer needs no registering anywhere: a run looks for this file by walking up from where it started, so
/// dropping it at the root of a tree is all it takes to configure that tree. An `[[overrides]]` entry is only worth
/// the indirection when the config has to live somewhere other than the root of what it configures, which is rare
/// enough to write by hand.
///
/// An existing file is left as written, so re-running the command changes nothing.
fn new_config() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let target = cwd.join(config::LOCAL_CONFIG_NAME);

    if target.exists() {
        println!("Kept {} (already there)", target.display());

        return Ok(());
    }

    // The nearest config wins outright rather than merging, so a new one silently replaces whatever an ancestor was
    // contributing here. Say so at creation time, when it is still cheap to move the file instead.
    let shadowed = config::find_local_config(&cwd);

    std::fs::write(&target, config::LOCAL_CONFIG_TEMPLATE)
        .with_context(|| format!("writing config file {}", target.display()))?;
    println!("Created {}", target.display());

    if let Some(shadowed) = shadowed {
        println!("Note: it replaces {} here and below.", shadowed.display());
    }

    Ok(())
}

/// Open the global or local config file in the editor, creating it if it does not exist.
///
/// `scope` selects which file; when omitted, the user is prompted to pick one.
fn open_config(scope: Option<ConfigScope>) -> Result<()> {
    let scope = match scope {
        Some(scope) => scope,
        None => prompt_config_scope()?,
    };

    let path = match scope {
        // Global: fixed path; create with current global values if missing.
        ConfigScope::Global => {
            let path = config::global_config_path().context("could not determine a config directory")?;
            if !path.exists() {
                config::save_global(&config::load_global()?)?;
                println!("Created {}", path.display());
            }
            path
        },
        // Local: reuse an existing `.selfnotes.toml` up the tree, else create one in the current directory.
        ConfigScope::Local => {
            if let Some(path) = config::find_local_config(&std::env::current_dir()?) {
                path
            } else {
                let path = std::env::current_dir()?.join(config::LOCAL_CONFIG_NAME);

                std::fs::write(&path, config::LOCAL_CONFIG_TEMPLATE)
                    .with_context(|| format!("writing config file {}", path.display()))?;
                println!("Created {}", path.display());

                path
            }
        },
    };

    let config = config::load()?;
    entry::open_paths_in_editor(&config, &[&path])
}

/// Interactively pick the global or local config scope.
fn prompt_config_scope() -> Result<ConfigScope> {
    let choice = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Which config?")
        .items(["global", "local"])
        .default(0)
        .interact()
        .context("selecting a config scope")?;

    Ok(if choice == 0 {
        ConfigScope::Global
    } else {
        ConfigScope::Local
    })
}

//! Building paths and creating journal / folder entries.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use chrono::Datelike;

use crate::config::{self, Config, FolderConfig};
use crate::template::{self, Context, Cursor};

/// Outcome of an entry request, so callers can report accurately.
pub struct Entry {
    /// Absolute path of the entry file.
    pub path: PathBuf,
    /// Whether the file was newly created (vs. already existed).
    pub created: bool,
    /// Cursor position from a `{{cursor}}` marker in the rendered template, if any.
    pub cursor: Option<Cursor>,
}

/// Create today's journal entry: `<root>/YYYY/MM/DD.<format>`.
pub fn create_journal(config: &Config) -> Result<Entry> {
    let root = config.resolved_journal_root()?;
    let ctx = Context::now();
    let now = ctx.now;

    let dir = root
        .join(format!("{:04}", now.year()))
        .join(format!("{:02}", now.month()));
    let file_name = format!("{:02}.{}", now.day(), config.journal_format());
    let path = dir.join(file_name);

    let template = config
        .journal
        .as_ref()
        .and_then(|journal| journal.template_file.as_deref());
    let default = "# {{date}}\n\n";

    write_entry(&path, template, default, &ctx)
}

/// Create an entry in a custom folder: `<root>/<folder.path>/<name>.<format>`.
///
/// `fields` are the resolved custom-field values, exposed to the template as
/// `{{<folder-name>.<field>}}`.
pub fn create_folder_entry(
    config: &Config,
    folder: &FolderConfig,
    name: &str,
    fields: Vec<(String, String)>,
) -> Result<Entry> {
    let root = config.resolved_journal_root()?;
    let sub = folder.path.as_deref().unwrap_or(&folder.name);
    let dir = root.join(sub);
    let file_name = format!("{}.{}", name, config.folder_format(folder));
    let path = dir.join(file_name);

    let ctx = Context::now().with_name(name).with_fields(folder.name.as_str(), fields);
    let template = folder.template_file.as_deref();
    let default = "# {{name}}\n\n_Created {{datetime}}_\n\n";

    write_entry(&path, template, default, &ctx)
}

/// Write an entry file if it does not already exist.
///
/// `template_file` is an optional path to a template; when absent, `default` is rendered instead. Both go through
/// placeholder substitution.
fn write_entry(path: &Path, template_file: Option<&str>, default: &str, ctx: &Context) -> Result<Entry> {
    if path.exists() {
        return Ok(Entry {
            path: path.to_path_buf(),
            created: false,
            cursor: None,
        });
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating directory {}", parent.display()))?;
    }

    let raw = match template_file {
        Some(file) => {
            let template_path = config::expand_tilde(file);

            std::fs::read_to_string(&template_path)
                .with_context(|| format!("reading template {}", template_path.display()))?
        },
        None => default.to_string(),
    };

    let rendered = template::render(&raw, ctx);

    std::fs::write(path, &rendered.content).with_context(|| format!("writing entry {}", path.display()))?;

    Ok(Entry {
        path: path.to_path_buf(),
        created: true,
        cursor: rendered.cursor,
    })
}

/// Open an entry `path` in the configured editor (falling back to `$EDITOR`).
///
/// Both `root` (the journal root) and the entry are passed as arguments, so an editor like `zed` opens the notes
/// workspace and focuses the file: `zed <root> <path>`. When `cursor` is set, the entry argument is formatted with the
/// editor's cursor syntax (see `Config::cursor_format`), e.g. `zed <root> <path>:12:5`.
pub fn open_in_editor(config: &Config, root: &Path, path: &Path, cursor: Option<&Cursor>) -> Result<()> {
    let editor = editor_command(config)?;
    let mut parts = editor.split_whitespace();
    let program = parts.next().context("editor command is empty")?;

    let status = std::process::Command::new(program)
        .args(parts)
        .arg(root)
        .args(entry_args(config, path, cursor))
        .status()
        .with_context(|| format!("launching editor `{editor}`"))?;

    anyhow::ensure!(status.success(), "editor exited with a non-zero status");

    Ok(())
}

/// Launch the configured editor on the given paths.
///
/// The editor is resolved from `config.editor`, falling back to `$EDITOR`.
/// Paths are passed as trailing arguments in order.
pub fn open_paths_in_editor(config: &Config, paths: &[&Path]) -> Result<()> {
    let editor = editor_command(config)?;

    let mut parts = editor.split_whitespace();
    let program = parts.next().context("editor command is empty")?;
    let status = std::process::Command::new(program)
        .args(parts)
        .args(paths)
        .status()
        .with_context(|| format!("launching editor `{editor}`"))?;

    anyhow::ensure!(status.success(), "editor exited with a non-zero status");

    Ok(())
}

/// Resolve the editor command from `config.editor`, falling back to `$EDITOR`.
fn editor_command(config: &Config) -> Result<String> {
    config
        .editor
        .clone()
        .or_else(|| std::env::var("EDITOR").ok())
        .context("no editor configured; set one with `selfnotes config set editor <cmd>` or `$EDITOR`")
}

/// Build the editor argument(s) for the entry, applying the cursor position when present.
///
/// With no cursor, this is just the path. With a cursor, `Config::cursor_format` is expanded (`{path}`, `{line}`,
/// `{column}`) and split on whitespace so multi-argument forms like vim's `+{line} {path}` work while a path with
/// spaces stays in a single argument.
fn entry_args(config: &Config, path: &Path, cursor: Option<&Cursor>) -> Vec<String> {
    let path = path.to_string_lossy();

    match cursor {
        Some(cursor) => {
            let line = cursor.line.to_string();
            let column = cursor.column.to_string();

            config
                .cursor_format()
                .split_whitespace()
                .map(|token| {
                    token
                        .replace("{path}", &path)
                        .replace("{line}", &line)
                        .replace("{column}", &column)
                })
                .collect()
        },
        None => vec![path.into_owned()],
    }
}

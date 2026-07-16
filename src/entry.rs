//! Building paths and creating journal / folder entries.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
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
    // Re-validate at the sink so the invariant holds for every caller. `main` runs these same checks earlier to fail
    // before prompting the user; both are cheap and idempotent.
    validate_entry_name(name)?;

    let dir = folder_dir(config, folder)?;
    let file_name = format!("{}.{}", name, config.folder_format(folder));
    // With `dir` inside the root and `name` a single plain component, this stays under the root.
    let path = dir.join(file_name);

    let ctx = Context::now().with_name(name).with_fields(folder.name.as_str(), fields);
    let template = folder.template_file.as_deref();
    let default = "# {{name}}\n\n_Created {{datetime}}_\n\n";

    write_entry(&path, template, default, &ctx)
}

/// Resolve the directory a folder's entries live in: `<root>/<folder.path-or-name>`.
///
/// Errors if the directory escapes the journal root, so callers can validate before prompting the user for anything.
pub fn folder_dir(config: &Config, folder: &FolderConfig) -> Result<PathBuf> {
    folder_dir_in(&config.resolved_journal_root()?, folder)
}

/// Resolve a folder's directory under an explicit `root`, validating it stays inside.
///
/// Like [`folder_dir`] but with the root supplied directly, so a config layer whose own `journal_root` is unset can
/// still be checked against the effective root during validation.
pub fn folder_dir_in(root: &Path, folder: &FolderConfig) -> Result<PathBuf> {
    let sub = folder.path.as_deref().unwrap_or(&folder.name);
    let dir = root.join(sub);

    ensure_within_root(root, &dir)?;

    Ok(dir)
}

/// Reject entry names that are not a single, plain filename.
///
/// The name is joined onto the journal root to form the entry path, so it must not contain a path separator (`/` or
/// `\`), be `.`/`..`, or be absolute. Without this, `selfnotes new ticket ../../secret` would write outside the journal
/// root.
pub fn validate_entry_name(name: &str) -> Result<()> {
    use std::path::Component;

    // `Path::components` treats `\` as a separator only on Windows, so reject it explicitly for cross-platform safety.
    if name.contains('/') || name.contains('\\') {
        bail!("invalid entry name `{name}`: must not contain a path separator");
    }

    let mut components = Path::new(name).components();

    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => bail!("invalid entry name `{name}`: must not be empty, `.`, `..`, or an absolute path"),
    }
}

/// Ensure `path` stays within `root` after lexically resolving any `.`/`..` components.
///
/// This is lexical (no filesystem access), so it works before the entry file or its directories exist and guards
/// against a `folder.path` config value that walks out of the journal root.
fn ensure_within_root(root: &Path, path: &Path) -> Result<()> {
    let normalized = lexical_normalize(path);

    anyhow::ensure!(
        normalized.starts_with(lexical_normalize(root)),
        "resolved entry path {} escapes the journal root {}",
        normalized.display(),
        root.display(),
    );

    Ok(())
}

/// Lexically resolve `.` and `..` components without touching the filesystem.
fn lexical_normalize(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut out = PathBuf::new();

    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            },
            Component::CurDir => {},
            other => out.push(other.as_os_str()),
        }
    }

    out
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
#[allow(clippy::literal_string_with_formatting_args)] // The `{path}`/`{line}`/`{column}` literals are template placeholders replaced by hand, not `format!` arguments.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_entry_names() {
        for name in ["ticket", "my-note", "2026-07-16", "note.with.dots"] {
            assert!(validate_entry_name(name).is_ok(), "expected `{name}` to be accepted");
        }
    }

    #[test]
    fn rejects_traversal_and_separators() {
        for name in [
            "",
            ".",
            "..",
            "../secret",
            "../../secret",
            "a/b",
            "/etc/passwd",
            "a\\b",
            "..\\secret",
        ] {
            assert!(validate_entry_name(name).is_err(), "expected `{name}` to be rejected");
        }
    }

    #[test]
    fn within_root_accepts_contained_paths() {
        let root = Path::new("/home/u/notes");

        assert!(ensure_within_root(root, Path::new("/home/u/notes/tickets/foo.md")).is_ok());
        // A `..` that stays under the root is fine.
        assert!(ensure_within_root(root, Path::new("/home/u/notes/tickets/../ideas/foo.md")).is_ok());
    }

    #[test]
    fn within_root_rejects_escaping_paths() {
        let root = Path::new("/home/u/notes");

        assert!(ensure_within_root(root, Path::new("/home/u/notes/../../secret.md")).is_err());
        assert!(ensure_within_root(root, Path::new("/etc/passwd")).is_err());
    }

    #[test]
    fn folder_dir_uses_path_when_set() {
        let root = Path::new("/home/u/notes");
        let folder = FolderConfig {
            name: "ticket".into(),
            path: Some("tickets".into()),
            ..FolderConfig::default()
        };

        assert_eq!(folder_dir_in(root, &folder).unwrap(), root.join("tickets"));
    }

    #[test]
    fn folder_dir_falls_back_to_name() {
        let root = Path::new("/home/u/notes");
        let folder = FolderConfig {
            name: "idea".into(),
            path: None,
            ..FolderConfig::default()
        };

        assert_eq!(folder_dir_in(root, &folder).unwrap(), root.join("idea"));
    }

    #[test]
    fn folder_dir_rejects_escaping_path() {
        let root = Path::new("/home/u/notes");
        let folder = FolderConfig {
            name: "ticket".into(),
            path: Some("../../secret".into()),
            ..FolderConfig::default()
        };

        assert!(folder_dir_in(root, &folder).is_err());
    }

    #[test]
    fn entry_args_without_cursor_is_just_the_path() {
        let config = Config::default();
        let path = Path::new("/home/u/notes/2026/07/17.md");

        assert_eq!(
            entry_args(&config, path, None),
            vec![path.to_string_lossy().into_owned()]
        );
    }

    #[test]
    fn entry_args_expands_the_default_cursor_format() {
        // No `cursor_format` set, so the zed / VS Code default `{path}:{line}:{column}` applies.
        let config = Config::default();
        let path = Path::new("/home/u/notes/a.md");
        let cursor = Cursor { line: 12, column: 5 };

        assert_eq!(
            entry_args(&config, path, Some(&cursor)),
            vec!["/home/u/notes/a.md:12:5"]
        );
    }

    #[test]
    fn entry_args_splits_multi_argument_cursor_formats() {
        // vim's `+{line} {path}` becomes two arguments after the whitespace split.
        let config = Config {
            cursor_format: Some("+{line} {path}".into()),
            ..Config::default()
        };
        let path = Path::new("/home/u/notes/a.md");
        let cursor = Cursor { line: 3, column: 1 };

        assert_eq!(
            entry_args(&config, path, Some(&cursor)),
            vec!["+3", "/home/u/notes/a.md"]
        );
    }

    #[test]
    fn entry_args_keeps_a_spaced_path_in_one_argument() {
        // The split is on the *format's* whitespace, not the path's, so a path with spaces stays a single argument.
        let config = Config::default();
        let path = Path::new("/home/u/my notes/a.md");
        let cursor = Cursor { line: 1, column: 1 };

        assert_eq!(
            entry_args(&config, path, Some(&cursor)),
            vec!["/home/u/my notes/a.md:1:1"]
        );
    }
}

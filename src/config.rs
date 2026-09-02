//! Configuration loading and merging.
//!
//! Up to three layers are merged, each overriding the previous: a global config
//! at `~/.config/selfnotes/config.toml`, any path-scoped `[[overrides]]` from the
//! global config whose glob matches the working directory, and a local
//! `.selfnotes.toml` found by walking up from the current directory.
//!
//! `selfnotes config new` writes the local layer: a [`LOCAL_CONFIG_NAME`] file in the current directory, seeded with
//! [`LOCAL_CONFIG_TEMPLATE`].

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use glob::Pattern;
use serde::{Deserialize, Serialize};

/// File name used for local, per-project configuration.
pub const LOCAL_CONFIG_NAME: &str = ".selfnotes.toml";

/// Starting point written by `selfnotes config new` into a new [`LOCAL_CONFIG_NAME`].
pub const LOCAL_CONFIG_TEMPLATE: &str = "\
# Layered on top of the global config for this directory and everything under it,
# since a `selfnotes` run looks for this file by walking up from where it started.
# The nearest one wins outright: a copy in a subdirectory replaces this file
# rather than adding to it.
#
# Any top-level key of the global config belongs in this file, for example:

# journal_root = \"~/work/journal\"
# people_file  = \"~/work/people.toml\"
# default_tags = [\"work\"]
# What spaces in a `selfnotes new` name become in the file name, so \"login bug\"
# is filed as `login-bug.md`. The note still reads `# login bug`.
# space_replacement = \"-\"

# [journal]
# template_file = \"~/work/templates/journal.md\"

# Folders you create entries in with `selfnotes new <name>`. A folder named like
# one in the global config replaces it; any other name is added alongside.

# [[custom_folders]]
# name = \"ticket\"
# # Directory, resolved against this file's own directory; defaults to the folder's
# # own name. Set `journal_root` above to put these folders under it instead.
# path = \"tickets\"
# template_file = \"~/work/templates/ticket.md\"
# # Added to the `default_tags` above, for this folder's entries only.
# default_tags = [\"ticket\"]
# # Ask for extra tags when the entry is created, as one comma-separated line.
# prompt_tags = true
# # Overrides the `space_replacement` above, for this folder only.
# space_replacement = \"_\"

# The ladder `selfnotes status` and `selfnotes next` move an entry along, in order.
# Replaces the top-level `statuses` rather than adding to it.

# statuses = [\"backlog\", \"todo\", \"doing\", \"blocked\", \"staging\", \"prod\"]
# # What a new entry starts in; defaults to the first status above.
# default_status = \"backlog\"
# # Statuses that close an entry, which `selfnotes board` hides unless given --all.
# terminal_statuses = [\"prod\"]

# Values prompted for when the entry is created, and read by its template as
# `{{ticket.priority}}` and `{{ticket.assignee}}`.

# [[custom_folders.fields]]
# name = \"priority\"
# prompt = \"Priority\"
# default = \"medium\"

# [[custom_folders.fields]]
# name = \"assignee\"
# prompt = \"Assigned to\"
";

/// Default file extension used when none is configured.
pub const DEFAULT_FORMAT: &str = "md";

/// Default cursor-position format, matching zed / VS Code (`-g`) syntax.
pub const DEFAULT_CURSOR_FORMAT: &str = "{path}:{line}:{column}";

/// Default minimum length of an all-hexadecimal inline `#token` treated as a git hash rather than a tag.
///
/// Set to 6 so abbreviated commit hashes (git's `--short` is commonly 7, sometimes 6) and full 40-character SHA-1
/// hashes are never mistaken for tags. See [`Config::hash_tag_min_len`].
pub const DEFAULT_HASH_TAG_MIN_LEN: usize = 6;

/// Default section of the previous journal entry read by the `{{last_day.*}}` template placeholders.
///
/// See [`Config::journal_carry_over_section`].
pub const DEFAULT_CARRY_OVER_SECTION: &str = "Today";

/// Top-level configuration, deserialized from TOML.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Config {
    /// Root directory under which all notes are created.
    pub journal_root: Option<String>,
    /// Default file extension (without the dot), e.g. `md`.
    pub format: Option<String>,
    /// Editor command used by `--open` (falls back to `$EDITOR`).
    pub editor: Option<String>,
    /// Editor cursor-position format for a `{{cursor}}` marker. Supports `{path}`, `{line}`, and `{column}`; split on
    /// whitespace into arguments.
    /// Defaults to `{path}:{line}:{column}` (zed / VS Code style).
    pub cursor_format: Option<String>,
    /// String that whitespace in a `selfnotes new` entry name is replaced with to build the file name. Unset leaves
    /// the name as typed; `""` removes the whitespace outright. Only the file name is affected, never the `{{name}}`
    /// the template renders.
    pub space_replacement: Option<String>,
    /// Tags seeded into the frontmatter of every new note, on top of any per-source defaults.
    #[serde(default)]
    pub default_tags: Vec<String>,
    /// Whether `selfnotes new` asks for extra tags on top of the resolved `default_tags`, for every folder that
    /// declares nothing of its own. Unset means no prompt. The journal never prompts.
    pub prompt_tags: Option<bool>,
    /// Minimum length of an all-hexadecimal inline `#token` treated as a git hash rather than a tag. `Some(0)`
    /// disables the heuristic; unset uses [`DEFAULT_HASH_TAG_MIN_LEN`].
    pub hash_tag_min_len: Option<usize>,
    /// Path to the roster of people completed after an `@` by `selfnotes lsp`. A leading `~` is expanded. Unset
    /// resolves to a `people.toml` beside the global config; a path-scoped override can point at a different roster
    /// per project.
    pub people_file: Option<String>,
    /// Statuses a note's frontmatter `status` may take, in workflow order, for every source that declares none of its
    /// own. Empty means statuses are not tracked. See [`crate::status`].
    #[serde(default)]
    pub statuses: Vec<String>,
    /// Status new notes start in. Unset (or not one of `statuses`) means the first declared status.
    pub default_status: Option<String>,
    /// Statuses that close a note, which `selfnotes board` leaves out unless asked for everything.
    #[serde(default)]
    pub terminal_statuses: Vec<String>,
    /// Journal-specific settings.
    pub journal: Option<JournalConfig>,
    /// User-defined folders, selected by name from the command line.
    #[serde(default)]
    pub custom_folders: Vec<FolderConfig>,
    /// Path-scoped config overrides. When the working directory matches an entry's glob, the referenced config is
    /// layered on top of the global config (but below any local `.selfnotes.toml`). Only meaningful in the global
    /// config.
    #[serde(default)]
    pub overrides: Vec<Override>,
}

/// A path-scoped override that points at an extra config file.
///
/// Unknown keys are rejected rather than ignored: an override's vocabulary is closed, so a setting written here is
/// almost always one that belongs at the top level of the config it points at, and silently dropping it would leave
/// the setting looking applied when it never was.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Override {
    /// Glob (or globs) matched against the current working directory. A leading `~` is expanded, and `**` matches
    /// across directory separators. Also spelled `paths`, which reads better for a list.
    #[serde(alias = "paths")]
    pub path: Globs,
    /// Path to the config file layered on top of the global config when the pattern matches. A leading `~` is
    /// expanded.
    pub config: String,
}

/// The glob, or globs, an override matches the working directory against.
///
/// A bare string covers the common case of one tree. An array covers one config file shared by several disjoint
/// trees, which is otherwise only expressible by repeating the whole entry.
///
/// ```toml
/// [[overrides]]
/// path = "~/work/**"
///
/// [[overrides]]
/// paths = ["~/work/**", "~/clients/acme/**"]
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Globs {
    /// A single glob.
    One(String),
    /// Several globs, any one of which selects a directory.
    Many(Vec<String>),
}

impl Default for Globs {
    /// No globs, which selects nothing.
    fn default() -> Self {
        Self::Many(Vec::new())
    }
}

impl From<&str> for Globs {
    fn from(glob: &str) -> Self {
        Self::One(glob.to_owned())
    }
}

impl Globs {
    /// The globs, as a slice.
    pub fn as_slice(&self) -> &[String] {
        match self {
            Self::One(glob) => std::slice::from_ref(glob),
            Self::Many(globs) => globs,
        }
    }
}

impl fmt::Display for Globs {
    /// The globs as a backticked, comma-separated list, ready to drop into a message.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, glob) in self.as_slice().iter().enumerate() {
            if index > 0 {
                write!(formatter, ", ")?;
            }

            write!(formatter, "`{glob}`")?;
        }

        Ok(())
    }
}

/// Settings for the built-in journal.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct JournalConfig {
    /// Path to a template file used when creating a journal entry.
    pub template_file: Option<String>,
    /// Extension override for journal entries.
    pub format: Option<String>,
    /// Tags seeded into the frontmatter of new journal entries, in addition to the global `default_tags`.
    #[serde(default)]
    pub default_tags: Vec<String>,
    /// Section of the previous entry that the `{{last_day.*}}` template placeholders read their checklist from.
    /// Defaults to [`DEFAULT_CARRY_OVER_SECTION`].
    pub carry_over_section: Option<String>,
}

/// The config layer a folder was declared in, so a message can say where a name came from.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FolderSource {
    /// The global config.
    #[default]
    Global,
    /// A config file pulled in by a path-scoped `[[overrides]]` entry.
    Override,
    /// The nearest local `.selfnotes.toml`.
    Local,
}

impl fmt::Display for FolderSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Global => "global",
            Self::Override => "override",
            Self::Local => "local",
        })
    }
}

/// Settings for a user-defined folder such as `ticket`.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct FolderConfig {
    /// Name used to select this folder on the command line.
    pub name: String,
    /// Directory (relative to the journal root) where entries are created.
    /// Defaults to the folder's own name.
    pub path: Option<String>,
    /// Path to a template file used when creating an entry.
    pub template_file: Option<String>,
    /// Extension override for this folder's entries.
    pub format: Option<String>,
    /// Override of the top-level `space_replacement` for this folder's entry names.
    pub space_replacement: Option<String>,
    /// Tags seeded into the frontmatter of new entries in this folder, in addition to the global `default_tags`.
    #[serde(default)]
    pub default_tags: Vec<String>,
    /// Whether creating an entry in this folder asks for extra tags, overriding the top-level `prompt_tags`.
    pub prompt_tags: Option<bool>,
    /// Statuses this folder's entries move through, in workflow order.
    ///
    /// Unlike `default_tags`, this *replaces* the top-level `statuses` rather than adding to it: a workflow is an
    /// ordered whole, and an ideas folder ending at `dropped` has nothing to gain from also inheriting a deployment
    /// ladder's `staging` and `prod`.
    #[serde(default)]
    pub statuses: Vec<String>,
    /// Status new entries in this folder start in, overriding the top-level `default_status`.
    pub default_status: Option<String>,
    /// Statuses that close one of this folder's entries, overriding the top-level `terminal_statuses`.
    #[serde(default)]
    pub terminal_statuses: Vec<String>,
    /// Custom fields prompted for when creating an entry and exposed to the template as `{{<folder-name>.<field>}}`.
    #[serde(default)]
    pub fields: Vec<TemplateField>,
    /// Explicit prompt arrangement by field name. Names listed here are prompted first, in this order; any field not
    /// listed follows in declaration order. Unknown names are ignored.
    #[serde(default)]
    pub field_order: Vec<String>,
    /// Directory that `path` is resolved against, in place of the journal root.
    ///
    /// Not a config key: it is stamped at load time onto the folders declared by a local `.selfnotes.toml` that sets
    /// no `journal_root` of its own, and holds that file's directory. See [`root_local_folders`].
    #[serde(skip)]
    pub base_dir: Option<PathBuf>,
    /// Which config file declared this folder. Not a config key: it is stamped at load time. See [`mark_folders`].
    #[serde(skip)]
    pub source: FolderSource,
}

impl FolderConfig {
    /// Fields in the order they should be prompted: names listed in `field_order` first (in that order), then any
    /// remaining fields in declaration order. Unknown or repeated names in `field_order` are ignored.
    pub fn ordered_fields(&self) -> Vec<&TemplateField> {
        let mut ordered = Vec::with_capacity(self.fields.len());
        let mut used = vec![false; self.fields.len()];

        for name in &self.field_order {
            if let Some(idx) = self.fields.iter().position(|field| &field.name == name)
                && !used[idx]
            {
                used[idx] = true;
                ordered.push(&self.fields[idx]);
            }
        }

        for (idx, field) in self.fields.iter().enumerate() {
            if !used[idx] {
                ordered.push(field);
            }
        }

        ordered
    }
}

/// A custom, per-folder template field.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct TemplateField {
    /// Field key, used in templates as `{{<folder-name>.<name>}}`.
    pub name: String,
    /// Prompt shown when creating an entry; defaults to `name`.
    pub prompt: Option<String>,
    /// Default value pre-filled at the prompt.
    pub default: Option<String>,
}

impl Config {
    /// Overlay `other` (higher priority) onto `self`, mutating in place.
    fn overlay(&mut self, other: Self) {
        if other.journal_root.is_some() {
            self.journal_root = other.journal_root;
        }

        if other.format.is_some() {
            self.format = other.format;
        }

        if other.editor.is_some() {
            self.editor = other.editor;
        }

        if other.cursor_format.is_some() {
            self.cursor_format = other.cursor_format;
        }

        if other.space_replacement.is_some() {
            self.space_replacement = other.space_replacement;
        }

        if !other.default_tags.is_empty() {
            self.default_tags = other.default_tags;
        }

        if other.prompt_tags.is_some() {
            self.prompt_tags = other.prompt_tags;
        }

        if other.hash_tag_min_len.is_some() {
            self.hash_tag_min_len = other.hash_tag_min_len;
        }

        if other.people_file.is_some() {
            self.people_file = other.people_file;
        }

        if !other.statuses.is_empty() {
            self.statuses = other.statuses;
        }

        if other.default_status.is_some() {
            self.default_status = other.default_status;
        }

        if !other.terminal_statuses.is_empty() {
            self.terminal_statuses = other.terminal_statuses;
        }

        if let Some(other_journal) = other.journal {
            let journal = self.journal.get_or_insert_with(JournalConfig::default);

            if other_journal.template_file.is_some() {
                journal.template_file = other_journal.template_file;
            }

            if other_journal.format.is_some() {
                journal.format = other_journal.format;
            }

            if !other_journal.default_tags.is_empty() {
                journal.default_tags = other_journal.default_tags;
            }

            if other_journal.carry_over_section.is_some() {
                journal.carry_over_section = other_journal.carry_over_section;
            }
        }
        // A local folder replaces the global one of the same name;
        // otherwise it is appended.
        for folder in other.custom_folders {
            match self
                .custom_folders
                .iter_mut()
                .find(|existing| existing.name == folder.name)
            {
                Some(existing) => *existing = folder,
                None => self.custom_folders.push(folder),
            }
        }
        // Carry overrides forward so `apply_overrides` can act on the global config's declarations. Overrides are only
        // honored from the global config (applied between the global and local layers), so a later layer replaces
        // rather than merges them.
        if !other.overrides.is_empty() {
            self.overrides = other.overrides;
        }
    }

    /// Find a configured folder by its name.
    pub fn folder(&self, name: &str) -> Option<&FolderConfig> {
        self.custom_folders.iter().find(|folder| folder.name == name)
    }

    /// The configured folder names, in configuration order, as the folder picker lists them.
    pub fn folder_labels(&self) -> Vec<String> {
        self.labelled_folders("")
    }

    /// The configured folder names as a backticked, comma-separated list, ready to drop into a message.
    pub fn folder_names(&self) -> String {
        self.labelled_folders("`").join(", ")
    }

    /// The configured folder names, each wrapped in `quote` and carrying the config it was declared in.
    ///
    /// The label is only added once the folders come from more than one config, which is when a name being missing,
    /// or not being quite the one you meant, is worth tracing to a file. With a single source it would just repeat on
    /// every name, saying nothing.
    ///
    /// Shared by the picker and the messages listing what a folder argument accepts, so the two never describe the
    /// same folders differently.
    fn labelled_folders(&self, quote: &str) -> Vec<String> {
        let mixed = self
            .custom_folders
            .windows(2)
            .any(|pair| pair[0].source != pair[1].source);

        self.custom_folders
            .iter()
            .map(|folder| {
                let name = &folder.name;
                let name = format!("{quote}{name}{quote}");

                if mixed {
                    format!("{name} ({})", folder.source)
                } else {
                    name
                }
            })
            .collect()
    }

    /// Resolve the effective journal root, expanding a leading `~`.
    pub fn resolved_journal_root(&self) -> Result<PathBuf> {
        let root = self
            .journal_root
            .as_deref()
            .context("no `journal_root` configured; set one with `selfnotes config set journal-root <path>`")?;

        Ok(expand_tilde(root))
    }

    /// Effective extension for journal entries.
    pub fn journal_format(&self) -> &str {
        self.journal
            .as_ref()
            .and_then(|journal| journal.format.as_deref())
            .or(self.format.as_deref())
            .unwrap_or(DEFAULT_FORMAT)
    }

    /// Effective section of the previous entry that the `{{last_day.*}}` placeholders read.
    ///
    /// A blank value falls back to the default, since a nameless section matches nothing.
    pub fn journal_carry_over_section(&self) -> &str {
        self.journal
            .as_ref()
            .and_then(|journal| journal.carry_over_section.as_deref())
            .map(str::trim)
            .filter(|section| !section.is_empty())
            .unwrap_or(DEFAULT_CARRY_OVER_SECTION)
    }

    /// Effective extension for the given folder.
    pub fn folder_format<'a>(&'a self, folder: &'a FolderConfig) -> &'a str {
        folder
            .format
            .as_deref()
            .or(self.format.as_deref())
            .unwrap_or(DEFAULT_FORMAT)
    }

    /// Effective editor cursor-position format.
    pub fn cursor_format(&self) -> &str {
        self.cursor_format.as_deref().unwrap_or(DEFAULT_CURSOR_FORMAT)
    }

    /// Effective whitespace replacement for entry names created in `folder`, or `None` when names are kept as typed.
    ///
    /// An empty string is a meaningful value (it joins the words with nothing), so it is returned as `Some("")` rather
    /// than falling back to the top level.
    pub fn folder_space_replacement<'a>(&'a self, folder: &'a FolderConfig) -> Option<&'a str> {
        folder
            .space_replacement
            .as_deref()
            .or(self.space_replacement.as_deref())
    }

    /// Effective minimum length for treating an all-hex inline `#token` as a git hash rather than a tag.
    pub fn hash_tag_min_len(&self) -> usize {
        self.hash_tag_min_len.unwrap_or(DEFAULT_HASH_TAG_MIN_LEN)
    }

    /// Tags seeded into a new journal entry: the global `default_tags` plus the journal's own, de-duplicated.
    pub fn journal_default_tags(&self) -> Vec<String> {
        let mut tags = self.default_tags.clone();

        if let Some(journal) = &self.journal {
            extend_unique(&mut tags, &journal.default_tags);
        }

        tags
    }

    /// Tags seeded into a new entry in `folder`: the global `default_tags` plus the folder's own, de-duplicated.
    pub fn folder_default_tags(&self, folder: &FolderConfig) -> Vec<String> {
        let mut tags = self.default_tags.clone();

        extend_unique(&mut tags, &folder.default_tags);

        tags
    }

    /// Whether creating an entry in `folder` asks for extra tags: the folder's own answer, else the top-level one,
    /// else no prompt.
    pub fn folder_prompt_tags(&self, folder: &FolderConfig) -> bool {
        folder.prompt_tags.or(self.prompt_tags).unwrap_or(false)
    }

    /// Statuses `folder`'s entries move through: its own workflow, or the top-level one when it declares none.
    ///
    /// Each of the three status keys falls back independently, so a folder can rename the ladder without restating
    /// which of its steps is the default. A combination that does not line up (a `default_status` outside the folder's
    /// own `statuses`, say) is reported by `selfnotes config validate`.
    pub fn folder_statuses<'a>(&'a self, folder: &'a FolderConfig) -> &'a [String] {
        if folder.statuses.is_empty() {
            &self.statuses
        } else {
            &folder.statuses
        }
    }

    /// Status new entries in `folder` start in, before it is checked against the effective workflow.
    pub fn folder_default_status<'a>(&'a self, folder: &'a FolderConfig) -> Option<&'a str> {
        folder.default_status.as_deref().or(self.default_status.as_deref())
    }

    /// Statuses that close one of `folder`'s entries.
    pub fn folder_terminal_statuses<'a>(&'a self, folder: &'a FolderConfig) -> &'a [String] {
        if folder.terminal_statuses.is_empty() {
            &self.terminal_statuses
        } else {
            &folder.terminal_statuses
        }
    }
}

/// Append each tag from `extra` that is not already in `base`, preserving order.
fn extend_unique(base: &mut Vec<String>, extra: &[String]) {
    for tag in extra {
        if !base.contains(tag) {
            base.push(tag.clone());
        }
    }
}

/// Load and merge the global and local configuration layers.
pub fn load() -> Result<Config> {
    let mut config = Config::default();

    if let Some(global_path) = global_config_path()
        && let Some(mut global) = read_config_file(&global_path)?
    {
        mark_folders(&mut global, FolderSource::Global);
        config.overlay(global);
    }

    // Path-scoped overrides sit between the global and local layers: they refine the global config for the current
    // directory, but a local `.selfnotes.toml` still wins.
    let cwd = std::env::current_dir()?;
    apply_overrides(&mut config, &cwd)?;

    if let Some(local_path) = find_local_config(&cwd)
        && let Some(mut local) = read_config_file(&local_path)?
    {
        root_local_folders(&mut local, &local_path);
        mark_folders(&mut local, FolderSource::Local);
        config.overlay(local);
    }

    Ok(config)
}

/// Record which config layer every folder in `config` was declared in.
///
/// Overlaying replaces a same-named folder outright, so the stamp that survives the merge is the one on the
/// declaration that actually took effect.
fn mark_folders(config: &mut Config, source: FolderSource) {
    for folder in &mut config.custom_folders {
        folder.source = source;
    }
}

/// Point the folders declared by the local config at `path` at that file's own directory.
///
/// A `.selfnotes.toml` marks the root of a tree, so a folder declared there belongs to that tree: `path = "ideas"` in
/// `~/work/.selfnotes.toml` means `~/work/ideas`, not a directory in whatever journal root the global config happens
/// to name. The journal is left alone, since it is not declared here and stays wherever `journal_root` puts it.
///
/// A local config that sets its own `journal_root` has already said where its notes live, so its folders keep
/// resolving against that and nothing is stamped.
pub fn root_local_folders(config: &mut Config, path: &Path) {
    if config.journal_root.is_some() {
        return;
    }

    let Some(base) = path.parent() else {
        return;
    };

    for folder in &mut config.custom_folders {
        folder.base_dir = Some(base.to_path_buf());
    }
}

/// Overlay every override config whose glob matches `cwd`, in declaration order.
///
/// A missing referenced file is skipped so a stale override never breaks a run.
fn apply_overrides(config: &mut Config, cwd: &Path) -> Result<()> {
    // Take the list out so we can overlay into `config` without aliasing it;
    // overlaying never touches `overrides`, so it stays empty meanwhile.
    let overrides = std::mem::take(&mut config.overrides);

    for path in matching_override_paths(&overrides, cwd)? {
        if let Some(mut scoped) = read_config_file(&path)? {
            mark_folders(&mut scoped, FolderSource::Override);
            config.overlay(scoped);
        }
    }

    config.overrides = overrides;

    Ok(())
}

/// Resolve, in order, the config-file paths of the overrides matching `cwd`.
fn matching_override_paths(overrides: &[Override], cwd: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();

    for entry in overrides {
        if override_matches(entry, cwd)? {
            paths.push(expand_tilde(&entry.config));
        }
    }

    Ok(paths)
}

/// Whether the override glob `glob` selects the directory `dir`. A leading `~` is expanded and `**` spans separators.
///
/// A trailing `/**` also selects the base directory itself. `~/work/**` is how a whole tree is written, and a tree
/// includes its own root, but the `glob` crate's `**` only spans the components *below* its parent: without this,
/// a config created for `~/work` would go unread by a `selfnotes` run from `~/work`.
fn glob_selects_dir(glob: &str, dir: &Path) -> Result<bool> {
    let expanded = expand_tilde(glob);
    let expanded = expanded.to_string_lossy();
    let pattern = compile_glob(&expanded, glob)?;

    if pattern.matches_path(dir) {
        return Ok(true);
    }

    match expanded.strip_suffix("/**") {
        Some(base) => Ok(compile_glob(base, glob)?.matches_path(dir)),
        None => Ok(false),
    }
}

/// Compile `pattern`, reporting the failure against the glob as the user wrote it.
fn compile_glob(pattern: &str, as_written: &str) -> Result<Pattern> {
    Pattern::new(pattern).with_context(|| format!("invalid override glob `{as_written}`"))
}

/// Path to the global config file, if a config directory can be determined.
pub fn global_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|dir| dir.join(".config").join("selfnotes").join("config.toml"))
}

/// Walk up from `start` looking for a local config file.
pub fn find_local_config(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);

    while let Some(dir) = current {
        let candidate = dir.join(LOCAL_CONFIG_NAME);

        if candidate.is_file() {
            return Some(candidate);
        }

        current = dir.parent();
    }

    None
}

/// Whether any of an override's globs matches `dir`. Errors when one of them is invalid.
///
/// See [`glob_selects_dir`] for what a single glob matches.
pub fn override_matches(entry: &Override, dir: &Path) -> Result<bool> {
    let mut matched = false;

    // Every glob is compiled even once one has matched, so `config validate` reports a typo in a later glob rather
    // than letting an earlier match hide it.
    for glob in entry.path.as_slice() {
        matched |= glob_selects_dir(glob, dir)?;
    }

    Ok(matched)
}

/// Read and parse a config file, returning `None` if it does not exist.
pub fn read_config_file(path: &Path) -> Result<Option<Config>> {
    if !path.exists() {
        return Ok(None);
    }

    let contents = std::fs::read_to_string(path).with_context(|| format!("reading config file {}", path.display()))?;
    let config = toml::from_str(&contents).with_context(|| format!("parsing config file {}", path.display()))?;

    Ok(Some(config))
}

/// Read only the global config, returning defaults when absent.
pub fn load_global() -> Result<Config> {
    match global_config_path() {
        Some(path) => Ok(read_config_file(&path)?.unwrap_or_default()),
        None => Ok(Config::default()),
    }
}

/// Persist a config to the global config path, creating parent directories.
pub fn save_global(config: &Config) -> Result<PathBuf> {
    let path = global_config_path().context("could not determine a config directory")?;

    write_config(&path, config)?;

    Ok(path)
}

/// Serialize `config` to TOML and write it to `path`, creating parent directories.
fn write_config(path: &Path, config: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating config directory {}", parent.display()))?;
    }

    let contents = toml::to_string_pretty(config).context("serializing config")?;

    std::fs::write(path, contents).with_context(|| format!("writing config {}", path.display()))
}

/// Expand a leading `~` in a path to the user's home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    } else if path == "~"
        && let Some(home) = dirs::home_dir()
    {
        return home;
    }

    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A local config declaring one folder, as read from `dir/.selfnotes.toml`.
    fn local_with_folder(journal_root: Option<&str>) -> Config {
        Config {
            journal_root: journal_root.map(str::to_owned),
            custom_folders: vec![FolderConfig {
                name: "idea".into(),
                path: Some("ideas".into()),
                ..FolderConfig::default()
            }],
            ..Config::default()
        }
    }

    #[test]
    fn prompt_tags_falls_back_from_the_folder_to_the_top_level() {
        let plain = FolderConfig {
            name: "ticket".into(),
            ..FolderConfig::default()
        };
        let opted_out = FolderConfig {
            prompt_tags: Some(false),
            ..plain.clone()
        };
        let opted_in = FolderConfig {
            prompt_tags: Some(true),
            ..plain.clone()
        };

        let silent = Config::default();
        let asking = Config {
            prompt_tags: Some(true),
            ..Config::default()
        };

        assert!(!silent.folder_prompt_tags(&plain));
        assert!(silent.folder_prompt_tags(&opted_in));
        assert!(asking.folder_prompt_tags(&plain));
        // A folder saying `false` outranks a top level saying `true`, so one noisy folder can be quietened.
        assert!(!asking.folder_prompt_tags(&opted_out));
    }

    /// A config with a top-level workflow and one folder, so the fallbacks can be exercised.
    fn config_with_statuses(folder: FolderConfig) -> Config {
        Config {
            statuses: vec!["backlog".into(), "todo".into(), "done".into()],
            default_status: Some("todo".into()),
            terminal_statuses: vec!["done".into()],
            custom_folders: vec![folder],
            ..Config::default()
        }
    }

    #[test]
    fn a_folder_without_statuses_runs_on_the_top_level_workflow() {
        let config = config_with_statuses(FolderConfig {
            name: "idea".into(),
            ..FolderConfig::default()
        });
        let folder = config.folder("idea").unwrap();

        assert_eq!(config.folder_statuses(folder), ["backlog", "todo", "done"]);
        assert_eq!(config.folder_default_status(folder), Some("todo"));
        assert_eq!(config.folder_terminal_statuses(folder), ["done"]);
    }

    #[test]
    fn a_folder_replaces_the_workflow_rather_than_extending_it() {
        let config = config_with_statuses(FolderConfig {
            name: "idea".into(),
            statuses: vec!["open".into(), "shipped".into()],
            ..FolderConfig::default()
        });
        let folder = config.folder("idea").unwrap();

        assert_eq!(config.folder_statuses(folder), ["open", "shipped"]);
        // The other two keys fall back on their own, so a renamed ladder need not restate them.
        assert_eq!(config.folder_default_status(folder), Some("todo"));
        assert_eq!(config.folder_terminal_statuses(folder), ["done"]);
    }

    #[test]
    fn a_folder_overrides_the_default_and_terminal_statuses_on_their_own() {
        let config = config_with_statuses(FolderConfig {
            name: "idea".into(),
            default_status: Some("backlog".into()),
            terminal_statuses: vec!["dropped".into()],
            ..FolderConfig::default()
        });
        let folder = config.folder("idea").unwrap();

        assert_eq!(config.folder_default_status(folder), Some("backlog"));
        assert_eq!(config.folder_terminal_statuses(folder), ["dropped"]);
    }

    #[test]
    fn statuses_overlay_wholesale_rather_than_merging() {
        let mut config = Config {
            statuses: vec!["backlog".into(), "todo".into()],
            terminal_statuses: vec!["todo".into()],
            ..Config::default()
        };

        config.overlay(Config {
            statuses: vec!["open".into(), "closed".into()],
            ..Config::default()
        });

        assert_eq!(config.statuses, ["open", "closed"]);
        // An empty list in the higher layer says nothing, so the lower one stands.
        assert_eq!(config.terminal_statuses, ["todo"]);
    }

    /// A config whose folders are named by `names`, each stamped with the layer it came from.
    fn config_with_folders(names: &[(&str, FolderSource)]) -> Config {
        Config {
            custom_folders: names
                .iter()
                .map(|(name, source)| FolderConfig {
                    name: (*name).to_owned(),
                    source: *source,
                    ..FolderConfig::default()
                })
                .collect(),
            ..Config::default()
        }
    }

    #[test]
    fn folder_names_are_bare_when_one_config_declares_them_all() {
        let config = config_with_folders(&[
            ("a", FolderSource::Global),
            ("b", FolderSource::Global),
            ("c", FolderSource::Global),
        ]);

        assert_eq!(config.folder_names(), "`a`, `b`, `c`");
    }

    #[test]
    fn folder_names_name_their_config_once_several_declare_them() {
        let config = config_with_folders(&[
            ("a", FolderSource::Global),
            ("b", FolderSource::Local),
            ("c", FolderSource::Override),
        ]);

        assert_eq!(config.folder_names(), "`a` (global), `b` (local), `c` (override)");
    }

    #[test]
    fn folder_names_is_empty_without_folders() {
        assert!(Config::default().folder_names().is_empty());
    }

    #[test]
    fn folder_labels_drop_the_backticks_the_picker_would_show_literally() {
        let config = config_with_folders(&[("a", FolderSource::Global), ("b", FolderSource::Global)]);

        assert_eq!(config.folder_labels(), ["a", "b"]);
    }

    #[test]
    fn folder_labels_name_their_config_once_several_declare_them() {
        let config = config_with_folders(&[("a", FolderSource::Global), ("b", FolderSource::Local)]);

        assert_eq!(config.folder_labels(), ["a (global)", "b (local)"]);
    }

    #[test]
    fn folder_labels_stay_in_configuration_order() {
        // The picker indexes its choice straight back into `custom_folders`, so the two must not drift apart.
        let config = config_with_folders(&[
            ("zeta", FolderSource::Local),
            ("alpha", FolderSource::Global),
            ("mid", FolderSource::Override),
        ]);

        assert_eq!(
            config.folder_labels(),
            ["zeta (local)", "alpha (global)", "mid (override)"]
        );
    }

    #[test]
    fn local_folders_are_rooted_beside_their_config() {
        let mut config = local_with_folder(None);

        root_local_folders(&mut config, Path::new("/home/u/work/.selfnotes.toml"));

        assert_eq!(
            config.custom_folders[0].base_dir.as_deref(),
            Some(Path::new("/home/u/work"))
        );
    }

    #[test]
    fn a_local_journal_root_keeps_folders_relative_to_it() {
        let mut config = local_with_folder(Some("/home/u/work/journal"));

        root_local_folders(&mut config, Path::new("/home/u/work/.selfnotes.toml"));

        assert_eq!(config.custom_folders[0].base_dir, None);
    }

    #[test]
    fn overlay_prefers_local_scalars() {
        let mut global = Config {
            journal_root: Some("/global".into()),
            format: Some("md".into()),
            ..Config::default()
        };

        let local = Config {
            journal_root: Some("/local".into()),
            ..Config::default()
        };
        global.overlay(local);

        assert_eq!(global.journal_root.as_deref(), Some("/local"));
        // Untouched scalar is preserved from the global layer.
        assert_eq!(global.format.as_deref(), Some("md"));
    }

    #[test]
    fn overlay_keeps_the_source_of_the_declaration_that_won() {
        let mut global = config_with_folders(&[("ticket", FolderSource::Global), ("idea", FolderSource::Global)]);

        global.overlay(config_with_folders(&[("ticket", FolderSource::Local)]));

        // `ticket` is replaced by the local declaration and reports it; `idea`, untouched, still reports the global.
        assert_eq!(global.folder_names(), "`ticket` (local), `idea` (global)");
    }

    #[test]
    fn overlay_merges_folders_by_name() {
        let mut global = Config {
            custom_folders: vec![FolderConfig {
                name: "ticket".into(),
                path: Some("tickets".into()),
                ..FolderConfig::default()
            }],
            ..Config::default()
        };

        let local = Config {
            custom_folders: vec![
                // Same name: replaces the global entry.
                FolderConfig {
                    name: "ticket".into(),
                    path: Some("local-tickets".into()),
                    ..FolderConfig::default()
                },
                // New name: appended.
                FolderConfig {
                    name: "idea".into(),
                    ..FolderConfig::default()
                },
            ],
            ..Config::default()
        };

        global.overlay(local);

        assert_eq!(global.custom_folders.len(), 2);
        assert_eq!(
            global.folder("ticket").and_then(|f| f.path.as_deref()),
            Some("local-tickets")
        );
        assert!(global.folder("idea").is_some());
    }

    #[test]
    fn overrides_match_by_glob_in_order() {
        let overrides = vec![
            Override {
                path: "/Affluences/**".into(),
                config: "/Affluences/afl-notes/selfnotes.config".into(),
            },
            Override {
                path: "/Other/**".into(),
                config: "/other.toml".into(),
            },
        ];

        let matched = matching_override_paths(&overrides, Path::new("/Affluences/afl-notes")).unwrap();
        assert_eq!(matched, vec![PathBuf::from("/Affluences/afl-notes/selfnotes.config")]);

        let none = matching_override_paths(&overrides, Path::new("/elsewhere")).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn overlay_carries_global_overrides_forward() {
        // Regression: `overlay` must propagate `overrides` so `apply_overrides` can act on the global declarations.
        let mut config = Config::default();
        let global = Config {
            overrides: vec![Override {
                path: "/Affluences/**".into(),
                config: "/tmp/afl.toml".into(),
            }],
            ..Config::default()
        };

        config.overlay(global);

        assert_eq!(config.overrides.len(), 1);
        assert_eq!(config.overrides[0].path.as_slice(), ["/Affluences/**"]);
    }

    #[test]
    fn an_override_takes_one_glob_or_several() {
        let config: Config = toml::from_str(
            r#"
            [[overrides]]
            path = "~/work/**"
            config = "~/work/selfnotes.config"

            [[overrides]]
            paths = ["~/work/**", "~/clients/acme/**"]
            config = "~/work/selfnotes.config"
            "#,
        )
        .unwrap();

        assert_eq!(config.overrides[0].path.as_slice(), ["~/work/**"]);
        // `paths` is the same key, spelled for a list.
        assert_eq!(config.overrides[1].path.as_slice(), ["~/work/**", "~/clients/acme/**"]);
    }

    #[test]
    fn any_of_an_overrides_globs_selects_the_directory() {
        // The point of a list: one config file covering trees that share no common root.
        let entry = Override {
            path: Globs::Many(vec!["/work/**".into(), "/clients/acme/**".into()]),
            config: "/work/selfnotes.config".into(),
        };

        assert!(override_matches(&entry, Path::new("/work/notes")).unwrap());
        assert!(override_matches(&entry, Path::new("/clients/acme")).unwrap());
        assert!(!override_matches(&entry, Path::new("/clients/other")).unwrap());
    }

    #[test]
    fn an_invalid_glob_is_reported_even_after_one_has_matched() {
        // Regression: short-circuiting on the first match would let `config validate` pass over a typo behind it.
        let entry = Override {
            path: Globs::Many(vec!["/work/**".into(), "/a/b**".into()]),
            config: "/work/selfnotes.config".into(),
        };

        let error = override_matches(&entry, Path::new("/work/notes"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("/a/b**"), "{error}");
    }

    #[test]
    fn an_override_without_a_glob_selects_nothing() {
        let entry = Override {
            path: Globs::Many(Vec::new()),
            config: "/work/selfnotes.config".into(),
        };

        assert!(!override_matches(&entry, Path::new("/work")).unwrap());
    }

    #[test]
    fn a_globs_list_survives_a_round_trip_through_toml() {
        // `config set` rewrites the whole global config, so a list must not come back as something else.
        let before = Config {
            overrides: vec![
                Override {
                    path: "~/work/**".into(),
                    config: "~/work/selfnotes.config".into(),
                },
                Override {
                    path: Globs::Many(vec!["~/work/**".into(), "~/clients/acme/**".into()]),
                    config: "~/work/selfnotes.config".into(),
                },
            ],
            ..Config::default()
        };

        let after: Config = toml::from_str(&toml::to_string_pretty(&before).unwrap()).unwrap();

        assert_eq!(after.overrides[0].path.as_slice(), ["~/work/**"]);
        assert!(matches!(after.overrides[0].path, Globs::One(_)));
        assert_eq!(after.overrides[1].path.as_slice(), ["~/work/**", "~/clients/acme/**"]);
        assert!(matches!(after.overrides[1].path, Globs::Many(_)));
    }

    #[test]
    fn globs_render_as_a_backticked_list() {
        assert_eq!(Globs::from("~/work/**").to_string(), "`~/work/**`");
        assert_eq!(
            Globs::Many(vec!["~/work/**".into(), "~/clients/acme/**".into()]).to_string(),
            "`~/work/**`, `~/clients/acme/**`"
        );
    }

    #[test]
    fn a_setting_misplaced_inside_an_override_is_an_error() {
        // Regression: `people_file` belongs at the top level. Accepting and ignoring it here made a roster look
        // configured while the default one was still being read.
        let error = toml::from_str::<Config>(
            r#"
            [[overrides]]
            path = "~/work/**"
            config = "~/work/selfnotes.config"
            people_file = "~/work/people.toml"
            "#,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("people_file"), "{error}");
    }

    /// The template's example lines with their outer `#` removed, as uncommenting them by hand would leave the file.
    ///
    /// Prose lines are dropped: only a line that opens a table or assigns a key is part of an example.
    fn uncommented_examples(template: &str) -> String {
        template
            .lines()
            .filter_map(|line| line.strip_prefix("# "))
            .filter(|line| line.starts_with('[') || line.contains(" = "))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_local_config_template_is_all_commentary() {
        // The file `config new` writes must load as an empty config, so it changes nothing until it is edited.
        let config: Config = toml::from_str(LOCAL_CONFIG_TEMPLATE).unwrap();

        assert!(config.journal_root.is_none());
        assert!(config.journal.is_none());
        assert!(config.custom_folders.is_empty());
    }

    #[test]
    fn the_local_config_template_examples_match_the_schema() {
        // Guards the template against drifting from the config it documents: uncommenting the examples has to
        // produce exactly what they claim, keys and nesting included.
        let config: Config = toml::from_str(&uncommented_examples(LOCAL_CONFIG_TEMPLATE)).unwrap();

        assert_eq!(config.journal_root.as_deref(), Some("~/work/journal"));
        assert_eq!(config.people_file.as_deref(), Some("~/work/people.toml"));
        assert_eq!(config.default_tags, ["work"]);
        assert!(
            config
                .journal
                .as_ref()
                .and_then(|journal| journal.template_file.as_deref())
                .is_some()
        );

        let folder = config.folder("ticket").expect("the example folder");
        assert_eq!(folder.path.as_deref(), Some("tickets"));
        assert_eq!(folder.default_tags, ["ticket"]);

        let fields: Vec<&str> = folder.fields.iter().map(|field| field.name.as_str()).collect();
        assert_eq!(fields, ["priority", "assignee"]);
        assert_eq!(folder.fields[0].default.as_deref(), Some("medium"));
        assert_eq!(folder.fields[1].prompt.as_deref(), Some("Assigned to"));
    }

    #[test]
    fn a_trailing_double_star_also_selects_the_base_directory() {
        // Regression: `config new` writes `<dir>/**`, and the `glob` crate's `**` spans only the components below its
        // parent. Without the base-directory case, a config created for `~/work` went unread by a run from `~/work`.
        let entry = Override {
            path: "/Affluences/**".into(),
            config: "/tmp/afl.toml".into(),
        };

        assert!(override_matches(&entry, Path::new("/Affluences")).unwrap());
        assert!(override_matches(&entry, Path::new("/Affluences/afl-notes")).unwrap());
        assert!(override_matches(&entry, Path::new("/Affluences/afl-notes/deep")).unwrap());
        // A sibling sharing the prefix is still not under the tree.
        assert!(!override_matches(&entry, Path::new("/Affluences-old")).unwrap());
    }

    #[test]
    fn override_matches_respects_the_glob() {
        let entry = Override {
            path: "/Affluences/**".into(),
            config: "/tmp/afl.toml".into(),
        };

        assert!(override_matches(&entry, Path::new("/Affluences/afl-notes")).unwrap());
        // A different tree does not match: `/Users/.../Affluences` is not under `/Affluences`.
        assert!(!override_matches(&entry, Path::new("/Users/tom/Affluences")).unwrap());
    }

    #[test]
    fn format_falls_back_to_default() {
        let config = Config::default();

        assert_eq!(config.journal_format(), DEFAULT_FORMAT);
    }

    #[test]
    fn expand_tilde_resolves_home_prefixed_paths() {
        let Some(home) = dirs::home_dir() else {
            // No home directory to expand against; nothing to assert.
            return;
        };

        assert_eq!(expand_tilde("~"), home);
        assert_eq!(expand_tilde("~/notes/journal"), home.join("notes/journal"));
    }

    #[test]
    fn expand_tilde_leaves_other_paths_untouched() {
        // Absolute and relative paths pass through verbatim.
        assert_eq!(expand_tilde("/etc/passwd"), PathBuf::from("/etc/passwd"));
        assert_eq!(expand_tilde("notes/journal"), PathBuf::from("notes/journal"));
        // A bare `~` is only expanded when it is the whole path or a `~/` prefix, not `~user`.
        assert_eq!(expand_tilde("~user/notes"), PathBuf::from("~user/notes"));
    }

    #[test]
    fn folder_fields_keep_declaration_order() {
        // Fields are prompted in the order they appear, so parsing must
        // preserve their declaration order.
        let toml = "\
[[custom_folders]]
name = \"ticket\"

[[custom_folders.fields]]
name = \"priority\"

[[custom_folders.fields]]
name = \"assignee\"

[[custom_folders.fields]]
name = \"due\"
";

        let config: Config = toml::from_str(toml).unwrap();
        let names: Vec<&str> = config
            .folder("ticket")
            .unwrap()
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect();

        assert_eq!(names, ["priority", "assignee", "due"]);
    }

    #[test]
    fn field_order_arranges_then_appends_the_rest() {
        let folder = FolderConfig {
            name: "ticket".into(),
            fields: vec![
                TemplateField {
                    name: "priority".into(),
                    ..TemplateField::default()
                },
                TemplateField {
                    name: "assignee".into(),
                    ..TemplateField::default()
                },
                TemplateField {
                    name: "due".into(),
                    ..TemplateField::default()
                },
            ],
            // Arrange two explicitly; an unknown name is ignored, and the
            // unlisted `due` falls through in declaration order.
            field_order: vec!["assignee".into(), "nope".into(), "priority".into()],
            ..FolderConfig::default()
        };

        let names: Vec<&str> = folder
            .ordered_fields()
            .iter()
            .map(|field| field.name.as_str())
            .collect();

        assert_eq!(names, ["assignee", "priority", "due"]);
    }
}

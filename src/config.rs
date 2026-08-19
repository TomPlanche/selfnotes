//! Configuration loading and merging.
//!
//! Up to three layers are merged, each overriding the previous: a global config
//! at `~/.config/selfnotes/config.toml`, any path-scoped `[[overrides]]` from the
//! global config whose glob matches the working directory, and a local
//! `.selfnotes.toml` found by walking up from the current directory.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use glob::Pattern;
use serde::{Deserialize, Serialize};

/// File name used for local, per-project configuration.
pub const LOCAL_CONFIG_NAME: &str = ".selfnotes.toml";

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
    /// Tags seeded into the frontmatter of every new note, on top of any per-source defaults.
    #[serde(default)]
    pub default_tags: Vec<String>,
    /// Minimum length of an all-hexadecimal inline `#token` treated as a git hash rather than a tag. `Some(0)`
    /// disables the heuristic; unset uses [`DEFAULT_HASH_TAG_MIN_LEN`].
    pub hash_tag_min_len: Option<usize>,
    /// Path to the roster of people completed after an `@` by `selfnotes lsp`. A leading `~` is expanded. Unset
    /// resolves to a `people.toml` beside the global config; a path-scoped override can point at a different roster
    /// per project.
    pub people_file: Option<String>,
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
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Override {
    /// Glob pattern matched against the current working directory. A leading `~` is expanded, and `**` matches across
    /// directory separators.
    pub path: String,
    /// Path to the config file layered on top of the global config when the pattern matches. A leading `~` is
    /// expanded.
    pub config: String,
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
    /// Tags seeded into the frontmatter of new entries in this folder, in addition to the global `default_tags`.
    #[serde(default)]
    pub default_tags: Vec<String>,
    /// Custom fields prompted for when creating an entry and exposed to the template as `{{<folder-name>.<field>}}`.
    #[serde(default)]
    pub fields: Vec<TemplateField>,
    /// Explicit prompt arrangement by field name. Names listed here are prompted first, in this order; any field not
    /// listed follows in declaration order. Unknown names are ignored.
    #[serde(default)]
    pub field_order: Vec<String>,
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

        if !other.default_tags.is_empty() {
            self.default_tags = other.default_tags;
        }

        if other.hash_tag_min_len.is_some() {
            self.hash_tag_min_len = other.hash_tag_min_len;
        }

        if other.people_file.is_some() {
            self.people_file = other.people_file;
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
        && let Some(global) = read_config_file(&global_path)?
    {
        config.overlay(global);
    }

    // Path-scoped overrides sit between the global and local layers: they refine the global config for the current
    // directory, but a local `.selfnotes.toml` still wins.
    let cwd = std::env::current_dir()?;
    apply_overrides(&mut config, &cwd)?;

    if let Some(local_path) = find_local_config(&cwd)
        && let Some(local) = read_config_file(&local_path)?
    {
        config.overlay(local);
    }

    Ok(config)
}

/// Overlay every override config whose glob matches `cwd`, in declaration order.
///
/// A missing referenced file is skipped so a stale override never breaks a run.
fn apply_overrides(config: &mut Config, cwd: &Path) -> Result<()> {
    // Take the list out so we can overlay into `config` without aliasing it;
    // overlaying never touches `overrides`, so it stays empty meanwhile.
    let overrides = std::mem::take(&mut config.overrides);

    for path in matching_override_paths(&overrides, cwd)? {
        if let Some(scoped) = read_config_file(&path)? {
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
        let pattern = expand_tilde(&entry.path);
        let pattern = Pattern::new(&pattern.to_string_lossy())
            .with_context(|| format!("invalid override glob `{}`", entry.path))?;

        if pattern.matches_path(cwd) {
            paths.push(expand_tilde(&entry.config));
        }
    }

    Ok(paths)
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

/// Whether an override's glob matches `dir`. A leading `~` is expanded and `**` spans separators. Errors when the
/// glob is invalid.
pub fn override_matches(entry: &Override, dir: &Path) -> Result<bool> {
    let pattern = expand_tilde(&entry.path);
    let pattern =
        Pattern::new(&pattern.to_string_lossy()).with_context(|| format!("invalid override glob `{}`", entry.path))?;

    Ok(pattern.matches_path(dir))
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

/// Persist a config as a local `.selfnotes.toml` in the current directory.
pub fn save_local(config: &Config) -> Result<PathBuf> {
    let path = std::env::current_dir()?.join(LOCAL_CONFIG_NAME);

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
        assert_eq!(config.overrides[0].path, "/Affluences/**");
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

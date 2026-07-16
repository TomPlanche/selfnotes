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

/// Top-level configuration, deserialized from TOML.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Config {
    /// Root directory under which all notes are created.
    pub journal_root: Option<String>,
    /// Default file extension (without the dot), e.g. `md`.
    pub format: Option<String>,
    /// Editor command used by `--open` (falls back to `$EDITOR`).
    pub editor: Option<String>,
    /// Journal-specific settings.
    pub journal: Option<JournalConfig>,
    /// User-defined folders, selected by name from the command line.
    #[serde(default)]
    pub custom_folders: Vec<FolderConfig>,
    /// Path-scoped config overrides. When the working directory matches an
    /// entry's glob, the referenced config is layered on top of the global
    /// config (but below any local `.selfnotes.toml`). Only meaningful in the
    /// global config.
    #[serde(default)]
    pub overrides: Vec<Override>,
}

/// A path-scoped override that points at an extra config file.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Override {
    /// Glob pattern matched against the current working directory. A leading
    /// `~` is expanded, and `**` matches across directory separators.
    pub path: String,
    /// Path to the config file layered on top of the global config when the
    /// pattern matches. A leading `~` is expanded.
    pub config: String,
}

/// Settings for the built-in journal.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct JournalConfig {
    /// Path to a template file used when creating a journal entry.
    pub template_file: Option<String>,
    /// Extension override for journal entries.
    pub format: Option<String>,
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
}

impl Config {
    /// Overlay `other` (higher priority) onto `self`, mutating in place.
    fn overlay(&mut self, other: Config) {
        if other.journal_root.is_some() {
            self.journal_root = other.journal_root;
        }

        if other.format.is_some() {
            self.format = other.format;
        }

        if other.editor.is_some() {
            self.editor = other.editor;
        }

        if let Some(other_journal) = other.journal {
            let journal = self.journal.get_or_insert_with(JournalConfig::default);

            if other_journal.template_file.is_some() {
                journal.template_file = other_journal.template_file;
            }

            if other_journal.format.is_some() {
                journal.format = other_journal.format;
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

    /// Effective extension for the given folder.
    pub fn folder_format<'a>(&'a self, folder: &'a FolderConfig) -> &'a str {
        folder
            .format
            .as_deref()
            .or(self.format.as_deref())
            .unwrap_or(DEFAULT_FORMAT)
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

    // Path-scoped overrides sit between the global and local layers: they
    // refine the global config for the current directory, but a local
    // `.selfnotes.toml` still wins.
    let cwd = std::env::current_dir()?;
    apply_overrides(&mut config, &cwd)?;

    if let Some(local_path) = find_local_config(cwd)?
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
pub fn find_local_config(start: PathBuf) -> Result<Option<PathBuf>> {
    let mut current: Option<&Path> = Some(start.as_path());

    while let Some(dir) = current {
        let candidate = dir.join(LOCAL_CONFIG_NAME);

        if candidate.is_file() {
            return Ok(Some(candidate));
        }

        current = dir.parent();
    }

    Ok(None)
}

/// Read and parse a config file, returning `None` if it does not exist.
fn read_config_file(path: &Path) -> Result<Option<Config>> {
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
    fn format_falls_back_to_default() {
        let config = Config::default();

        assert_eq!(config.journal_format(), DEFAULT_FORMAT);
    }
}

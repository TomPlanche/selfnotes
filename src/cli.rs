//! Command-line interface for `selfnotes`.
//! `selfnotes -h` for full usage information.

use clap::{Parser, Subcommand};

/// The main CLI for `selfnotes`.
#[derive(Debug, Parser)]
#[command(
    name = "selfnotes",
    version,
    about,
    author,
    help_template = "{name} {version}\n{author}\n{about}\n\n{usage-heading} {usage}\n\n{all-args}"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create or open today's journal entry (default action).
    Journal {
        /// Skip opening the entry in your editor after creating it.
        #[arg(long)]
        no_open: bool,
    },
    /// Create or open an entry in a custom folder.
    New {
        /// Folder name as defined in the configuration; prompted for if omitted.
        folder: Option<String>,
        /// Entry name; prompted for if omitted.
        name: Option<String>,
        /// Skip opening the entry in your editor after creating it.
        #[arg(long)]
        no_open: bool,
    },
    /// List recent entries across the journal and custom folders (newest first).
    #[command(visible_alias = "recent")]
    List {
        /// Maximum number of entries to show.
        #[arg(short = 'n', long, default_value_t = 10)]
        limit: usize,
        /// Restrict to a single source: a custom folder's name, or `journal` for the built-in journal.
        #[arg(long)]
        folder: Option<String>,
    },
    /// Inspect or change configuration.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Show resolved config file locations and effective values.
    Path,
    /// Check the effective configuration for problems (bad paths, missing templates, ...).
    Validate,
    /// Open a config file in your editor, creating it if needed.
    Open,
    /// Print a single configuration value.
    Get {
        /// One of: journal-root, format, editor, cursor-format.
        key: String,
    },
    /// Set a value in the global configuration.
    Set {
        /// One of: journal-root, format, editor, cursor-format.
        key: String,
        /// The value to store.
        value: String,
    },
}

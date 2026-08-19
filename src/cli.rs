//! Command-line interface for `selfnotes`.
//! `selfnotes -h` for full usage information.

use clap::{Parser, Subcommand, ValueEnum};

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
    /// Create or open a journal entry, today's by default.
    Journal {
        /// Which day to create or open: `YYYY-MM-DD`, `today`/`yesterday`/`tomorrow`, or a signed day offset such as
        /// `-1` or `+3`.
        #[arg(short, long, value_name = "DATE", allow_hyphen_values = true)]
        date: Option<String>,
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
        /// Only show entries carrying this tag (repeatable; every listed tag must match). Matches nested tags too, so
        /// `--tag work` also matches `work/project`.
        #[arg(long = "tag")]
        tags: Vec<String>,
    },
    /// Search note bodies for text (newest first).
    Search {
        /// Text to look for. Matched literally, not as a pattern.
        query: String,
        /// Maximum number of notes to show.
        #[arg(short = 'n', long, default_value_t = 10)]
        limit: usize,
        /// Restrict to a single source: a custom folder's name, or `journal` for the built-in journal.
        #[arg(long)]
        folder: Option<String>,
        /// Only search notes carrying this tag (repeatable; every listed tag must match). Matches nested tags too, so
        /// `--tag work` also matches `work/project`.
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Lines of context to show either side of each match.
        #[arg(short = 'C', long, default_value_t = 0)]
        context: usize,
        /// Match the query's case exactly (matching is case-insensitive by default).
        #[arg(short = 's', long)]
        case_sensitive: bool,
        /// Print only the paths of matching notes, one per line.
        #[arg(short = 'l', long)]
        files: bool,
    },
    /// List every tag and how many notes use it.
    Tags {
        /// Restrict to a single source: a custom folder's name, or `journal` for the built-in journal.
        #[arg(long)]
        folder: Option<String>,
        /// Sort order for the listing.
        #[arg(long, value_enum, default_value_t = TagSort::Count)]
        sort: TagSort,
    },
    /// Show a note's outbound `[[links]]` and the notes that link back to it.
    Links {
        /// Note to inspect, by name (optionally `folder/name`).
        name: String,
    },
    /// Resolve a `[[note-name]]` target and open it in your editor.
    Open {
        /// Note to open, by name (optionally `folder/name`).
        name: String,
    },
    /// List the people that `@mentions` complete to.
    People {
        #[command(subcommand)]
        action: Option<PeopleAction>,
    },
    /// Serve `@mention` completion over the Language Server Protocol, on stdin and stdout.
    ///
    /// Started by an editor extension rather than run by hand.
    Lsp,
    /// Inspect or change configuration.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Debug, Clone, Copy, Subcommand)]
pub enum PeopleAction {
    /// Show where the roster is read from.
    Path,
    /// Open the roster in your editor, creating it from a template if needed.
    Open,
}

/// Sort order for the `tags` listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TagSort {
    /// Most-used tags first.
    Count,
    /// Alphabetical.
    Name,
}

/// Which configuration file a `config` action targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ConfigScope {
    /// The global config at `~/.config/selfnotes/config.toml`.
    Global,
    /// The nearest local `.selfnotes.toml`.
    Local,
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Show resolved config file locations and effective values.
    Path,
    /// Check the effective configuration for problems (bad paths, missing templates, ...).
    Validate,
    /// Open a config file in your editor, creating it if needed.
    Open {
        /// Which config to open: `global` or `local`. Prompted for if omitted.
        scope: Option<ConfigScope>,
    },
    /// Print a single configuration value.
    Get {
        /// One of: journal-root, format, editor, cursor-format, hash-tag-min-len, people-file.
        key: String,
    },
    /// Set a value in the global configuration.
    Set {
        /// One of: journal-root, format, editor, cursor-format, hash-tag-min-len, people-file.
        key: String,
        /// The value to store.
        value: String,
    },
}

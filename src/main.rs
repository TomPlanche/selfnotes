//! `selfnotes`: a CLI that manages a journal-style notes filesystem.
//! `selfnotes -h` for full usage information.

mod cli;
mod config;
mod entry;
mod template;

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Input, Select};

use cli::{Cli, Command, ConfigAction};
use config::{Config, FolderConfig};
use entry::Entry;

fn main() -> Result<()> {
    let args = Cli::parse();
    // A bare invocation creates today's journal entry.
    let command = args.command.unwrap_or(Command::Journal { no_open: false });

    match command {
        Command::Journal { no_open } => {
            let config = config::load()?;
            let entry = entry::create_journal(&config)?;

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

            let name = match name {
                Some(name) => name,
                None => prompt_name()?,
            };
            let name = name.trim();

            if name.is_empty() {
                bail!("entry name cannot be empty");
            }

            let fields = prompt_fields(&folder_config)?;

            let entry = entry::create_folder_entry(&config, &folder_config, name, fields)?;
            report(&entry);
            maybe_open(&config, &entry, no_open);
        },
        Command::Config { action } => run_config(action)?,
    }

    Ok(())
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

    if let Err(err) = entry::open_in_editor(config, &root, &entry.path) {
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

    for field in &folder.fields {
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

/// Handle the `config` subcommand.
fn run_config(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Open => return open_config(),
        ConfigAction::Path => {
            match config::global_config_path() {
                Some(path) => println!("global: {}", path.display()),
                None => println!("global: <unavailable>"),
            }
            match config::find_local_config(std::env::current_dir()?)? {
                Some(path) => println!("local:  {}", path.display()),
                None => println!("local:  <none>"),
            }

            let merged = config::load()?;

            println!();
            println!("effective values:");
            println!(
                "  journal-root = {}",
                merged.journal_root.as_deref().unwrap_or("<unset>")
            );
            println!("  format       = {}", merged.journal_format());
            println!("  editor       = {}", merged.editor.as_deref().unwrap_or("<unset>"));
        },
        ConfigAction::Get { key } => {
            let config = config::load()?;

            let value = match key.as_str() {
                "journal-root" => config.journal_root.clone(),
                "format" => Some(config.journal_format().to_string()),
                "editor" => config.editor.clone(),
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
                other => bail!("unknown config key `{other}`"),
            }

            let path = config::save_global(&config)?;
            println!("Updated {}", path.display());
        },
    }
    Ok(())
}

/// Prompt for the global or local config file, then open it in the editor,
/// creating it if it does not exist.
fn open_config() -> Result<()> {
    let choice = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Which config?")
        .items(["global", "local"])
        .default(0)
        .interact()
        .context("selecting a config scope")?;

    let path = match choice {
        // Global: fixed path; create with current global values if missing.
        0 => {
            let path = config::global_config_path().context("could not determine a config directory")?;
            if !path.exists() {
                config::save_global(&config::load_global()?)?;
                println!("Created {}", path.display());
            }
            path
        },
        // Local: reuse an existing `.selfnotes.toml` up the tree, else create
        // one in the current directory.
        _ => match config::find_local_config(std::env::current_dir()?)? {
            Some(path) => path,
            None => {
                let path = config::save_local(&Config::default())?;
                println!("Created {}", path.display());
                path
            },
        },
    };

    let config = config::load()?;
    entry::open_paths_in_editor(&config, &[&path])
}

//! Zed extension that starts `selfnotes lsp` for Markdown buffers, so typing `@` completes the people in your roster.
//!
//! The extension ships no binary of its own: it finds the `selfnotes` already installed on the machine and runs its
//! `lsp` subcommand. When Zed cannot see it (a GUI launch often inherits a much shorter `$PATH` than a terminal one),
//! point the editor at a specific build through the usual LSP binary settings:
//!
//! ```json
//! { "lsp": { "selfnotes": { "binary": { "path": "/Users/you/.cargo/bin/selfnotes" } } } }
//! ```

use zed_extension_api::settings::LspSettings;
use zed_extension_api::{self as zed, Command, LanguageServerId, Result, Worktree};

/// Name of the binary the language server lives in.
const BINARY: &str = "selfnotes";

/// Subcommand that turns the CLI into a language server on stdin and stdout.
const SUBCOMMAND: &str = "lsp";

struct SelfnotesExtension;

impl zed::Extension for SelfnotesExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(&mut self, id: &LanguageServerId, worktree: &Worktree) -> Result<Command> {
        let binary = LspSettings::for_worktree(id.as_ref(), worktree)
            .ok()
            .and_then(|settings| settings.binary);

        let (path, arguments) = match binary {
            Some(binary) => (binary.path, binary.arguments),
            None => (None, None),
        };

        Ok(Command {
            command: match path {
                Some(path) => path,
                None => find_binary(worktree)?,
            },
            args: arguments.unwrap_or_else(|| vec![SUBCOMMAND.to_owned()]),
            // The server reads the same config the CLI does, which means it needs the same environment.
            env: worktree.shell_env(),
        })
    }
}

/// Locate `selfnotes`: the editor's `$PATH` first, then where cargo and Homebrew install into.
///
/// Probing those fallbacks can be denied by the extension sandbox, in which case only the `$PATH` lookup answers and
/// the error below points at the settings escape hatch.
fn find_binary(worktree: &Worktree) -> Result<String> {
    if let Some(path) = worktree.which(BINARY) {
        return Ok(path);
    }

    let home = worktree
        .shell_env()
        .into_iter()
        .find(|(name, _)| name == "HOME")
        .map(|(_, value)| value);

    let mut candidates = vec![
        format!("/opt/homebrew/bin/{BINARY}"),
        format!("/usr/local/bin/{BINARY}"),
    ];

    if let Some(home) = home {
        candidates.push(format!("{home}/.cargo/bin/{BINARY}"));
    }

    candidates
        .into_iter()
        .find(|candidate| std::fs::metadata(candidate).is_ok())
        .ok_or_else(|| {
            format!(
                "could not find `{BINARY}`. Install it with `cargo install {BINARY}`, or set \
                 `lsp.{BINARY}.binary.path` in your Zed settings."
            )
        })
}

zed::register_extension!(SelfnotesExtension);

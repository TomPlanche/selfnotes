# selfnotes for Zed

Completes and describes `@mentions` of the people in your [selfnotes](../../README.md) roster, in Markdown files.

Type `@` in a Markdown buffer and Zed offers everyone in `people.toml`, with their name, role and team. Hover a written `@handle` to see who it is, and to reach the links you attached to them. Cmd-click a mention to open the first of those links.

Completion and hover work in any Zed. Cmd-clicking a mention needs a build from 2026-05-26 or later, when Zed gained `textDocument/documentLink` support.

## Install

The extension carries no binary of its own: it runs the `selfnotes` already on your machine, as `selfnotes lsp`.

1. Install `selfnotes` so it is on your `$PATH`: `cargo install --path ../..` from here, or `cargo install selfnotes`.
2. Add the `wasm32-wasip2` target Zed builds the extension with: `rustup target add wasm32-wasip2`.
3. In Zed, run `zed: extensions` from the command palette, click `Install Dev Extension`, and pick this directory.

## When Zed cannot find the binary

A GUI launch inherits a shorter `$PATH` than your terminal does, so `selfnotes` may be invisible to Zed even though it works in a shell. Point the editor at it directly:

```json
{
  "lsp": {
    "selfnotes": {
      "binary": { "path": "/Users/you/.cargo/bin/selfnotes" }
    }
  }
}
```

The server logs how many people it loaded, and from which file, as it starts. `debug: open language server logs` shows it.

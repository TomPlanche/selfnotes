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

## Statuses from the editor

[`tasks.json`](./tasks.json) here is a starting point for driving [note statuses](../../README.md#statuses) without leaving Zed. Copy it into your notes repository as `.zed/tasks.json` (or merge it into `~/.config/zed/tasks.json` to have the tasks everywhere), then run `task: spawn` from the command palette.

It defines five tasks:

| Task | What it runs | What it does |
| --- | --- | --- |
| `selfnotes: status` | `selfnotes status $ZED_FILE` | Shows where the open note sits in its workflow, and what comes next. |
| `selfnotes: set status` | `selfnotes status $ZED_FILE --pick` | Picks the new status from a list, right in the task terminal. |
| `selfnotes: next status` | `selfnotes next $ZED_FILE` | Moves the open note one step along its workflow. |
| `selfnotes: board` | `selfnotes board` | Shows every tracked note grouped by status. |
| `selfnotes: status -> doing` | `selfnotes status $ZED_FILE doing` | One status, one keystroke. Copy the entry per status you want bound. |

The commands take the note's path, so they act on whatever buffer is open, whatever it is called. Each task sets `"save": "current"`, so Zed writes the buffer before `selfnotes` reads it, and `"cwd": "$ZED_DIRNAME"`, so a `.selfnotes.toml` next to your notes is found the same way it is from a shell.

Bind the ones you use to a key in `keymap.json`:

```json
{
  "context": "Editor && extension==md",
  "bindings": {
    "cmd-alt-n": ["task::Spawn", { "task_name": "selfnotes: next status" }],
    "cmd-alt-s": ["task::Spawn", { "task_name": "selfnotes: set status" }]
  }
}
```

A status change rewrites the frontmatter on disk. Zed reloads a buffer with no unsaved changes, which `"save": "current"` guarantees, so the new `status` line appears in the editor on its own.

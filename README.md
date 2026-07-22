# selfnotes

<pre>
                     ▄▄▄▄         ▄▄▄▄                                                    
                     ▀▀██        ██▀▀▀                         ██                         
 ▄▄█████▄   ▄████▄     ██      ███████   ██▄████▄   ▄████▄   ███████    ▄████▄   ▄▄█████▄ 
 ██▄▄▄▄ ▀  ██▄▄▄▄██    ██        ██      ██▀   ██  ██▀  ▀██    ██      ██▄▄▄▄██  ██▄▄▄▄ ▀ 
  ▀▀▀▀██▄  ██▀▀▀▀▀▀    ██        ██      ██    ██  ██    ██    ██      ██▀▀▀▀▀▀   ▀▀▀▀██▄ 
 █▄▄▄▄▄██  ▀██▄▄▄▄█    ██▄▄▄     ██      ██    ██  ▀██▄▄██▀    ██▄▄▄   ▀██▄▄▄▄█  █▄▄▄▄▄██ 
  ▀▀▀▀▀▀     ▀▀▀▀▀      ▀▀▀▀     ▀▀      ▀▀    ▀▀    ▀▀▀▀       ▀▀▀▀     ▀▀▀▀▀    ▀▀▀▀▀▀  
</pre>


A small CLI that manages a journal-style notes filesystem. It creates dated journal entries and named entries in custom folders (like `tickets`), each from a configurable template.

## Layout

Journal entries follow a year/month/day structure under the configured root. On the 13th of July 2026, running `selfnotes` creates:

```
<journal-root>/2026/07/13.md
```

Custom folders create named entries directly under the folder:

```
<journal-root>/tickets/<name>.md
```

The file extension (`md` by default) is configurable globally, per journal, and per folder.

## Usage

```
selfnotes                     # create today's journal entry and open it
selfnotes journal             # same as above
selfnotes journal --no-open   # create it without launching the editor
selfnotes new                 # pick a folder, then enter a name (interactive)
selfnotes new ticket          # skip the folder picker, prompt for a name
selfnotes new ticket login-bug   # pass both directly
selfnotes new ticket login-bug --no-open

selfnotes list                # list recent entries, newest first
selfnotes recent              # alias for `list`
selfnotes list -n 20          # show up to 20 entries
selfnotes list --folder journal   # only the built-in journal
selfnotes list --folder ticket    # only the `ticket` folder
selfnotes list --tag work         # only entries tagged #work (repeatable; all must match)

selfnotes tags                # list every tag with a note count, most-used first
selfnotes tags --sort name    # alphabetical instead
selfnotes tags --folder ticket    # only tags in the `ticket` folder

selfnotes links login-bug     # show a note's [[links]] and its backlinks
selfnotes open login-bug      # resolve a [[wikilink]] target and open it

selfnotes config path         # show config locations and effective values
selfnotes config validate     # check the effective config for problems
selfnotes config get journal-root
selfnotes config set journal-root ~/notes
selfnotes config set format md
selfnotes config set editor "zed"
selfnotes config set cursor-format "{path}:{line}:{column}"
selfnotes config set hash-tag-min-len 7   # tune the git-hash-vs-tag threshold
```

By default, creating an entry opens it in your editor. The editor is invoked with two arguments, the journal root and the entry file, so an editor like `zed` opens the whole notes workspace and focuses the file:

```
zed <journal-root> <journal-root>/2026/07/13.md
```

The editor comes from the `editor` config key, falling back to `$EDITOR`. Pass `--no-open` to skip this. Since the entry is written before the editor is launched, a missing or failing editor is reported as a warning and does not fail the command.

Creating an entry never overwrites an existing file: if the target already exists, it is left untouched (and reopened).

## Listing entries

`selfnotes list` (aliased as `selfnotes recent`) scans the journal and every custom folder and prints the entries most recently modified, newest first, so you can find a note without opening the editor. Each line is the modification time, the source (`journal` or a folder name), and the entry path relative to the journal root:

```
2026-07-17 09:00  journal  2026/07/17.md
2026-07-10 09:00  ticket   tickets/login-bug.md
2026-07-01 09:00  idea     idea/dark-mode.md
```

Pass `-n`/`--limit` to change how many entries are shown (default 10). Pass `--folder <name>` to restrict the listing to a single source: a custom folder's name, or the reserved value `journal` for the built-in journal. Dotfiles are skipped, and a missing folder directory simply contributes nothing.

## Tags and links

Notes stay plain text, so tags and links are conventions written inside a note rather than a separate database. `selfnotes` reads them back out on demand, which means your notes remain portable and every change is a plain diff in `git`.

### Tags

A note can be tagged two ways, and both are merged:

- Inline `#tag` anywhere in the body. The `#` must sit at the start of a line or after whitespace, so a markdown heading (`# Title`) or a URL fragment (`page#anchor`) is never mistaken for a tag. Tags may nest with `/`, e.g. `#work/project`.
- A `+++`-delimited TOML frontmatter block at the very top of the file with a `tags` array. Any other frontmatter keys are ignored.

```markdown
+++
tags = ["work", "bug/auth"]
+++

# 2026-07-22

Fixed the login bug today. #work #sprint-42
```

Tags inside fenced code blocks (```` ``` ````) or inline code spans (`` `#notatag` ``) are ignored, so code samples do not pollute your tags.

`selfnotes tags` lists every tag with the number of notes using it, most-used first (`--sort name` for alphabetical). `selfnotes list --tag <tag>` filters the recent listing to notes carrying that tag; pass `--tag` more than once to require all of them. Matching is case-insensitive, and a tag also matches its nested children, so `--tag work` matches both `#work` and `#work/project`. Both commands accept `--folder` to scope to a single source.

```
$ selfnotes tags
2  #work
1  #bug/auth
1  #sprint-42
```

#### Default tags

A config layer can seed tags onto every new note it creates. Set `default_tags` at the top level to tag everything, `[journal]`'s `default_tags` for journal entries, and a folder's `default_tags` for that folder; the effective set for a note is the global list plus the relevant per-source list. The tags are written into the note's `+++` frontmatter at creation (merged into any frontmatter a template already provides), so they are real, portable tags, not an invisible overlay, and only new notes are affected.

```toml
default_tags = ["me"]          # on every new note

[journal]
default_tags = ["daily"]       # journal entries also get #daily (e.g. a daily scrum)

[[custom_folders]]
name = "ticket"
default_tags = ["work"]        # tickets also get #work
```

With the above, `selfnotes` (a journal entry) starts with `tags = ["me", "daily"]` in its frontmatter.

#### Git hashes are not tags

A purely numeric `#123` is never a tag (a tag must start with a letter or underscore), so issue references are already safe. To keep commit hashes such as `#deadbeef` or `#a1b2c3d` from being read as tags too, an inline `#token` that is entirely hexadecimal and at least `hash_tag_min_len` characters long is treated as a git hash and skipped. The default is 6, covering abbreviated (6/7/8-character) and full 40-character hashes. This only applies to inline `#` tags, never to frontmatter tags, which are always explicit.

```toml
# Raise it if you keep short all-hex tags, or set 0 to turn the heuristic off entirely.
hash_tag_min_len = 7
```

The trade-off of a low threshold is that an all-hex word used as a tag (e.g. `#facade`, `#decade`) is also skipped; raise the value or disable it with `0` if that bites. It can also be read or set from the CLI: `selfnotes config get hash-tag-min-len` / `selfnotes config set hash-tag-min-len 7`.

### Links

Link one note to another with a `[[wikilink]]`, optionally with display text as `[[target|shown text]]`. The target is a note's name (its filename without the extension); prefix it with a folder to disambiguate when the same name exists in more than one place, e.g. `[[tickets/login-bug]]`. Matching is case-insensitive, and links inside code are ignored just like tags.

```markdown
Related: [[login-bug]] and [[tickets/PROJ-1|the ticket]].
```

`selfnotes links <name>` resolves a note by name and shows its outbound links (with where each one resolves, or `unresolved` / `ambiguous`) together with its backlinks, the notes that link to it. `selfnotes open <name>` resolves the same way and opens the note in your editor. When a bare name is ambiguous across folders, both commands list the candidates so you can qualify it with a `folder/name`.

```
$ selfnotes links roadmap
ideas/roadmap.md

Outbound links:
  [[login-bug]] -> ideas/login-bug.md

Backlinks:
  2026/07/22.md
```

#### Titles and aliases

A note's filename is not always what you want to type. Give a note a human-readable `title` and any number of `aliases` in its frontmatter, and a `[[wikilink]]`, `selfnotes links`, or `selfnotes open` will resolve to it by any of those, not just the filename. Matching stays case-insensitive, and a `folder/name` qualifier still applies.

```markdown
+++
title = "Login bug investigation"
aliases = ["login-bug", "PROJ-1"]
+++
```

With the above (in a file that might be named `note-42.md`), every one of these finds the note:

```
selfnotes open note-42                    # by filename
selfnotes open PROJ-1                      # by alias
selfnotes open "Login bug investigation"   # by title
```

And in prose, `[[login-bug]]`, `[[PROJ-1]]`, and `[[Login bug investigation]]` all link to it. When a name (filename, title, or alias) is shared by more than one note it is reported as ambiguous, and `links` / `open` list the candidates, showing each note's title where it has one, so you can qualify with `folder/name`.

## Configuration

Configuration is merged from up to three layers, each overriding the previous:

- Global: `~/.config/selfnotes/config.toml` (platform config directory).
- Overrides: path-scoped config files declared in the global config (see below).
- Local: the nearest `.selfnotes.toml` found by walking up from the current directory.

Scalar keys (`journal_root`, `format`, `editor`, `cursor_format`, `hash_tag_min_len`) and the `[journal]` section are merged field by field. `default_tags` (top level and per source) is replaced by a later layer that sets a non-empty list. Each `[[custom_folders]]` entry is matched by `name`: a later entry with the same name replaces the earlier one, and any new names are appended.

### Path-scoped overrides

The global config can declare `[[overrides]]` entries, each pairing a glob `path` with a `config` file. When the current working directory matches the glob, that config is layered on top of the global config (but below any local `.selfnotes.toml`). This lets a whole directory tree pick up its own defaults without a `.selfnotes.toml` in each project.

```toml
[[overrides]]
# When run anywhere under /Affluences, layer in the referenced config.
path = "/Affluences/**"
config = "/Affluences/afl-notes/selfnotes.config"
```

A leading `~` is expanded in both `path` and `config`, and `**` matches across directory separators. Overrides are applied in declaration order, and a referenced file that does not exist is skipped.

### Example

```toml
# Root under which everything is created (a leading ~ is expanded).
journal_root = "~/notes"
# Default file extension for entries.
format = "md"
# Optional editor for `--open` (falls back to $EDITOR).
editor = "nvim"
# Tags seeded into every new note's frontmatter.
default_tags = ["me"]
# All-hex inline #tokens this long or longer are treated as git hashes, not tags (default 6; 0 disables).
hash_tag_min_len = 6

[journal]
# Optional template rendered into new journal entries.
template_file = "~/.config/selfnotes/templates/journal.md"
# Journal entries are also tagged #daily.
default_tags = ["daily"]

[[custom_folders]]
name = "ticket"
# Directory under the root; defaults to the folder name if omitted.
path = "tickets"
template_file = "~/.config/selfnotes/templates/ticket.md"
# Optional per-folder extension override.
format = "md"
# Tickets are also tagged #work.
default_tags = ["work"]

[[custom_folders]]
name = "idea"
# path omitted -> entries land in <journal-root>/idea/
```

Running `selfnotes new` with no folder shows a picker of the configured folder names (via `dialoguer`), then prompts for the entry name.

### Validating the configuration

`selfnotes config validate` checks every config file that contributes to the effective configuration and prints a verdict (`valid` / `INVALID`) per file, with each problem attributed to the file it came from. It exits non-zero when any file is invalid, so it fits in scripts and pre-flight checks. The blocking problems are: an unset `journal_root`, a custom folder whose directory would escape the journal root (via its `path` or a name containing `..`), a folder `name` containing a path separator (`/` or `\`), and a referenced `template_file` (journal or folder) that does not exist. A `field_order` entry naming a field that no field declares is reported as a warning rather than an error.

Each file is judged on what actually takes effect: a folder or journal template shadowed by a higher-priority layer is not re-checked, and folder directories are resolved against the effective journal root even when a given file does not set its own root. The set of files checked mirrors what loading merges: the global config, each matching override's referenced config, and the nearest local `.selfnotes.toml`.

The command also inspects the `[[overrides]]` declared in each config file. An invalid glob is an error. For an override in the global config, it reports whether the glob matches the current directory and checks the referenced config file exists (an error when the override matches here, a warning otherwise); the referenced config's own folders and templates are validated as one of the checked files. An override declared in a local config is reported as ignored, since only the global config's overrides are applied, along with whether its glob would even have matched, which is the usual reason such an override looks like it does nothing.

Entry names are held to the same rule at creation time: a `name` containing a path separator, `..`, or an absolute path is rejected before anything is written, so an entry can never be created outside the journal root.

### Custom folder fields

A folder can declare `[[custom_folders.fields]]` entries. When you create an entry in that folder, you are prompted for each field, and the values are exposed to the template as `{{<folder-name>.<field>}}` (e.g. a `ticket` folder's `priority` field is `{{ticket.priority}}`).

```toml
[[custom_folders]]
name = "ticket"
path = "tickets"
template_file = "~/.config/selfnotes/templates/ticket.md"

[[custom_folders.fields]]
name = "priority"
# Optional prompt label (defaults to the field name).
prompt = "Priority"
# Optional value pre-filled at the prompt.
default = "medium"

[[custom_folders.fields]]
name = "assignee"
```

With the template `ticket.md`:

```markdown
# {{name}}

Priority: {{ticket.priority}}
Assignee: {{ticket.assignee}}
```

Fields are prompted in declaration order by default. An empty answer is allowed, and an unresolved `{{<folder-name>.<field>}}` is left untouched like any other unknown placeholder.

To arrange the prompts independently of how the blocks are written, add a `field_order` list on the folder. Names listed there are prompted first, in that order; any field not listed follows in declaration order, and unknown names are ignored:

```toml
[[custom_folders]]
name = "ticket"
field_order = ["assignee", "priority"]   # `due` (unlisted) is prompted last

[[custom_folders.fields]]
name = "priority"
[[custom_folders.fields]]
name = "assignee"
[[custom_folders.fields]]
name = "due"
```

## Templates

Templates are plain files referenced by `template_file`. When no template is configured, a minimal built-in default is used. Both go through `{{placeholder}}` substitution:

| Placeholder    | Example            | Notes                      |
| -------------- | ------------------ | -------------------------- |
| `{{date}}`     | `2026-07-13`       |                            |
| `{{datetime}}` | `2026-07-13 09:05` |                            |
| `{{time}}`     | `09:05`            |                            |
| `{{year}}`     | `2026`             |                            |
| `{{month}}`    | `07`               | zero-padded                |
| `{{day}}`      | `13`               | zero-padded                |
| `{{weekday}}`  | `Monday`           |                            |
| `{{name}}`     | `login-bug`        | custom-folder entries only |
| `{{<folder-name>.<field>}}` | `high` | custom folder fields, e.g. `{{ticket.priority}}` |

Unknown placeholders are left untouched so typos stay visible.

### Conditional blocks

Wrap part of a template in `{{?key}}...{{/key}}` to render it only when `key` is set to a non-empty value. This is handy for optional custom fields: skip a whole line when the field is left blank.

```markdown
# {{name}}

{{?ticket.priority}}Priority: {{ticket.priority}}
{{/ticket.priority}}{{?ticket.assignee}}Assignee: {{ticket.assignee}}
{{/ticket.assignee}}
```

Any placeholder key works as the condition (custom fields, `{{name}}`, date parts). Blocks may nest, and an unbalanced `{{?key}}` with no matching `{{/key}}` is left untouched like any other unresolved placeholder.

### Cursor position

Place a `{{cursor}}` marker in a template to say where the editor's cursor should land. The marker is removed from the written file, and when the entry is opened, its line and column are handed to the editor.

```markdown
# {{name}}

{{cursor}}
```

The editor argument is built from the `cursor_format` config key, which defaults to `{path}:{line}:{column}` (zed and VS Code with `-g`). It supports the `{path}`, `{line}`, and `{column}` placeholders and is split on whitespace into arguments, so multi-argument editors work too:

```toml
editor = "zed"
# default:
cursor_format = "{path}:{line}:{column}"

# vim / nvim:
# editor = "nvim"
# cursor_format = "+{line} {path}"

# VS Code:
# editor = "code -g"
# cursor_format = "{path}:{line}:{column}"
```

If a template has no `{{cursor}}` marker, the entry opens normally with no position argument. Only the first marker is used, and a marker inside a conditional block that is skipped has no effect.
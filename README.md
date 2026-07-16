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

selfnotes config path         # show config locations and effective values
selfnotes config validate     # check the effective config for problems
selfnotes config get journal-root
selfnotes config set journal-root ~/notes
selfnotes config set format md
selfnotes config set editor "zed"
selfnotes config set cursor-format "{path}:{line}:{column}"
```

By default, creating an entry opens it in your editor. The editor is invoked with two arguments, the journal root and the entry file, so an editor like `zed` opens the whole notes workspace and focuses the file:

```
zed <journal-root> <journal-root>/2026/07/13.md
```

The editor comes from the `editor` config key, falling back to `$EDITOR`. Pass `--no-open` to skip this. Since the entry is written before the editor is launched, a missing or failing editor is reported as a warning and does not fail the command.

Creating an entry never overwrites an existing file: if the target already exists, it is left untouched (and reopened).

## Configuration

Configuration is merged from up to three layers, each overriding the previous:

- Global: `~/.config/selfnotes/config.toml` (platform config directory).
- Overrides: path-scoped config files declared in the global config (see below).
- Local: the nearest `.selfnotes.toml` found by walking up from the current directory.

Scalar keys (`journal_root`, `format`, `editor`) and the `[journal]` section are merged field by field. Each `[[custom_folders]]` entry is matched by `name`: a later entry with the same name replaces the earlier one, and any new names are appended.

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

[journal]
# Optional template rendered into new journal entries.
template_file = "~/.config/selfnotes/templates/journal.md"

[[custom_folders]]
name = "ticket"
# Directory under the root; defaults to the folder name if omitted.
path = "tickets"
template_file = "~/.config/selfnotes/templates/ticket.md"
# Optional per-folder extension override.
format = "md"

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
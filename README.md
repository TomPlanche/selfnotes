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
selfnotes                        # create today's journal entry and open it
selfnotes journal                # same as above
selfnotes journal --no-open      # create it without launching the editor
selfnotes journal --date yesterday    # open (or backfill) yesterday's entry
selfnotes journal -d -1               # same thing, as a day offset
selfnotes journal -d 2026-07-13       # an absolute date
selfnotes journal -d tomorrow         # tomorrow's entry, ready for notes
selfnotes new                    # pick a folder, then enter a name (interactive)
selfnotes new ticket             # skip the folder picker, prompt for a name
selfnotes new ticket login-bug   # pass both directly
selfnotes new ticket "login bug" # a spaced name (see `space_replacement`)
selfnotes new ticket login-bug --no-open

selfnotes list                  # list recent entries, newest first
selfnotes recent                # alias for `list`
selfnotes list -n 20            # show up to 20 entries
selfnotes list --folder journal # only the built-in journal
selfnotes list --folder ticket  # only the `ticket` folder
selfnotes list --tag work       # only entries tagged #work (repeatable; all must match)
selfnotes list --status doing   # only entries whose status is `doing` (repeatable; any may match)

selfnotes search "login bug"            # find notes whose text contains it, newest first
selfnotes search login -C 2             # show 2 lines of context around each match
selfnotes search login -n 20            # show up to 20 notes
selfnotes search login --folder ticket  # only the `ticket` folder
selfnotes search login --tag work       # only notes tagged #work (repeatable; all must match)
selfnotes search login --status doing   # only notes whose status is `doing` (repeatable; any may match)
selfnotes search LOGIN --case-sensitive # exact case (matching is case-insensitive by default)
selfnotes search login --files          # print only the matching notes' paths

selfnotes tags                 # list every tag with a note count, most-used first
selfnotes tags --sort name     # alphabetical instead
selfnotes tags --folder ticket # only tags in the `ticket` folder

selfnotes status login-bug         # show where a note sits in its workflow
selfnotes status login-bug doing   # move it to `doing`
selfnotes status login-bug --pick  # choose the new status from a list
selfnotes status ideas/login-bug.md doing  # a path works too, for editor integrations
selfnotes next login-bug           # move it one step along the workflow
selfnotes advance login-bug        # alias for `next`

selfnotes board                 # every tracked note, grouped by status
selfnotes board --folder ticket # only the `ticket` folder, in its own workflow order
selfnotes board --tag work      # only notes tagged #work
selfnotes board --all           # also show the closed statuses, hidden by default
selfnotes board -n 5            # at most 5 notes per column

selfnotes links login-bug # show a note's [[links]] and its backlinks
selfnotes open login-bug  # resolve a [[wikilink]] target and open it

selfnotes people               # list the people that `@mentions` complete to
selfnotes people path          # show where the roster is read from
selfnotes people open          # open the roster in your editor, creating it if needed
selfnotes people import        # add people from a directory export on stdin (JSON by default)
selfnotes people import --format tsv    # tab-separated rows instead
selfnotes people import --dry-run       # report what would change, write nothing
selfnotes people import --prune         # also drop people the source no longer lists
selfnotes lsp                  # serve `@mention` completion over LSP (started by an editor)

selfnotes config path                # show config locations and effective values
selfnotes config validate            # check the effective config for problems
selfnotes config new                 # create a .selfnotes.toml here, covering this directory and below
selfnotes config open                # pick global or local interactively, then open it
selfnotes config open <local|global> # open the local config directly (or `global`)
selfnotes config get journal-root
selfnotes config set journal-root ~/notes
selfnotes config set format md
selfnotes config set editor "zed"
selfnotes config set cursor-format "{path}:{line}:{column}"
selfnotes config set space-replacement "-"   # file "login bug" as login-bug.md
selfnotes config set people-file ~/work/people.toml
selfnotes config set hash-tag-min-len 7   # tune the git-hash-vs-tag threshold
selfnotes config get statuses             # the workflow, as a comma-separated list
selfnotes config get default-status
selfnotes config get terminal-statuses
```

Lists such as `statuses` are read by `config get` but not written by `config set`, which only handles single values. Edit them with `selfnotes config open`.

By default, creating an entry opens it in your editor. The editor is invoked with two arguments, the journal root and the entry file, so an editor like `zed` opens the whole notes workspace and focuses the file:

```
zed <journal-root> <journal-root>/2026/07/13.md
```

The editor comes from the `editor` config key, falling back to `$EDITOR`. Pass `--no-open` to skip this. Since the entry is written before the editor is launched, a missing or failing editor is reported as a warning and does not fail the command.

Creating an entry never overwrites an existing file: if the target already exists, it is left untouched (and reopened).

## Journal dates

A journal entry defaults to today, but `-d`/`--date` picks another day, so you can reopen an earlier entry or backfill one you missed. Three forms are accepted:

| Form | Example | Meaning |
| ---- | ------- | ------- |
| `YYYY-MM-DD` | `-d 2026-07-13` | that exact day |
| a name | `-d yesterday` | `today`, `yesterday`, or `tomorrow` |
| a signed day offset | `-d -1`, `-d +3` | that many days from today |

The sign on an offset is required, so a bare `-d 3` is an error rather than a guess. Names are case-insensitive. Future dates are allowed: `-d tomorrow` prepares tomorrow's entry.

Nothing about a past or future date is special-cased. The entry lands at its usual `<journal-root>/YYYY/MM/DD.<format>` path, gets the same template and `default_tags`, and is opened in your editor exactly as today's entry would be. If the file already exists it is opened untouched, which is what makes `selfnotes -d yesterday` the way to reread yesterday.

In the rendered template, the date placeholders describe the day the entry is *for*, while `{{time}}` stays the current clock time, since that is when you are actually writing. Backfilling the 13th on the 27th at 09:05 renders `{{date}}` as `2026-07-13` and `{{time}}` as `09:05`.

## Listing entries

`selfnotes list` (aliased as `selfnotes recent`) scans the journal and every custom folder and prints the entries most recently modified, newest first, so you can find a note without opening the editor. Each line is the modification time, the source (`journal` or a folder name), and the entry path relative to the journal root (absolute when the entry sits outside it):

```
2026-07-17 09:00  journal  2026/07/17.md
2026-07-10 09:00  ticket   tickets/login-bug.md
2026-07-01 09:00  idea     idea/dark-mode.md
```

Pass `-n`/`--limit` to change how many entries are shown (default 10). Pass `--folder <name>` to restrict the listing to a single source: a custom folder's name, or the reserved value `journal` for the built-in journal. Dotfiles are skipped, and a missing folder directory simply contributes nothing. `--tag` and `--status` filter the listing further, see [Tags](#tags) and [Statuses](#statuses).

## Searching

`selfnotes search <query>` scans the text of every note and prints the ones that contain the query, newest first. The query is matched literally, not as a regular expression, and matching is case-insensitive unless you pass `-s`/`--case-sensitive`.

Each note gets a header (its source, path relative to the journal root or absolute when it sits outside, frontmatter title if it has one, and how many of its lines matched) followed by the matching lines, each prefixed with its line number in the file:

```
$ selfnotes search "login bug"
ideas  ideas/caps.md  [1 line]
      5: LOGIN BUG in caps.

journal  2026/07/27.md  (Sprint planning)  [2 lines]
      8: Discussed the login bug at length.
  ...
     17: The login bug again, near the end.
```

Line numbers count from the top of the file, frontmatter included, so they line up with what your editor shows. The `...` marks lines skipped between two runs of matches in the same note.

Pass `-C`/`--context <n>` to show `n` lines either side of each match. Context lines are marked with `-` instead of `:`, and windows that overlap are merged into a single run rather than repeating lines:

```
$ selfnotes search "login bug" -C 1
journal  2026/07/27.md  (Sprint planning)  [2 lines]
      7-
      8: Discussed the login bug at length.
      9- Also: deploy pipeline is flaky.
  ...
     16-
     17: The login bug again, near the end.
```

Search accepts the same filters as listing: `-n`/`--limit` (default 10) caps how many notes are shown, `--folder <name>` restricts to one source, `--tag <tag>` (repeatable) keeps only notes carrying every listed tag, with the same case-insensitive, nested-tag matching `list --tag` uses, and `--status <state>` (repeatable) keeps only notes in one of those [statuses](#statuses). Pass `-l`/`--files` to print just the paths of the matching notes, one per line, which is convenient for piping into another tool.

Only a note's body is searched: a `+++` frontmatter block at the top is skipped, so a search for `tags` does not match every tagged note's metadata. Unlike tag and link parsing, fenced code blocks and inline code spans *are* searched, since finding a command or a snippet you wrote down is usually the point.

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

## Statuses

A note can also carry a `status`: where it sits in a workflow you declare, such as `backlog`, `todo`, `doing`, `staging`, `prod`. It is the piece that turns a folder of notes into a board of tickets.

A status is not a tag, and it is not stored as one. A tag is an open set and a note can carry any number of them; a status is single-valued, exclusive and mutable, so it lives in its own key of the `+++` frontmatter and moving a note means replacing that value rather than adding another:

```markdown
+++
tags = ["idea"]
status = "doing"
+++

# MR/PR support

Add the possibility to create MR/PRs from the command line.
```

### Declaring a workflow

Nothing is tracked until a folder says which statuses its entries may take. The ladder belongs in the config rather than in the binary: a ticket folder ends at `prod`, an ideas folder ends at `dropped`, and neither is `selfnotes`' business to decide.

```toml
# The default ladder for folders that declare none of their own.
statuses = ["backlog", "todo", "doing", "blocked", "done", "dropped"]
default_status = "backlog"     # what a new entry starts in; defaults to the first status
terminal_statuses = ["done", "dropped"]   # what closes an entry

[[custom_folders]]
name = "ticket"
path = "tickets"
# A folder replaces the ladder rather than extending it: a workflow is an ordered whole.
statuses = ["backlog", "todo", "doing", "staging", "prod"]
terminal_statuses = ["prod"]
```

The three keys fall back independently, so a folder can rename the ladder without restating which step is the default. Declaration order is the workflow order: it is what `selfnotes next` walks and the order a board shows its columns in. `selfnotes config validate` reports a blank or repeated status, and a `default_status` or `terminal_statuses` naming a step the workflow does not have.

Statuses apply to custom folders only. A journal entry is a dated log rather than a ticket, so it never carries one, and a top-level `statuses` is a default *for folders* rather than a workflow every note is dragged into.

New entries are created with `default_status` already in their frontmatter, alongside any `default_tags`. A template that sets its own `status` (from a [custom field](#custom-folder-fields), say) keeps it.

### Moving a note

```
$ selfnotes status mr-pr-support
ideas/mr-pr-support.md  (MR/PR support)

status:   todo
workflow: backlog -> [todo] -> doing -> blocked -> done -> dropped
next:     doing  (`selfnotes next`)

$ selfnotes next mr-pr-support
ideas/mr-pr-support.md: todo -> doing

$ selfnotes status mr-pr-support blocked
ideas/mr-pr-support.md: doing -> blocked
```

`selfnotes status <note>` on its own writes nothing: it reports where the note is, the workflow it belongs to, and what comes next. Pass a state to move it there, or `--pick` to choose from a list. A state is matched case-insensitively and stored the way the config spells it, so `DOING` is filed as `doing`; a state the workflow does not have is refused, with the ones it does have.

`selfnotes next` (aliased `advance`) moves a note one step along. A note with no status yet joins at the folder's `default_status`, and one already in the last status says so rather than failing.

Both accept a note by name, exactly as `links` and `open` do, or by path. The path form is what editor integrations use, since it needs no guessing about how a note is named and is never ambiguous between two folders holding the same name.

Writing a status edits a file you wrote, so only one line changes. An existing `status` line is replaced where it stands, keeping its indentation and its place among the other keys; otherwise the assignment is inserted after the last top-level key, before any `[table]` and the comments introducing it. Comments, key order and formatting are left byte for byte as they were, and a frontmatter that is not valid TOML (or a `+++` fence that is never closed) is refused rather than guessed at.

### The board

`selfnotes board` groups every tracked note by status, in workflow order:

```
$ selfnotes board
backlog (0)
todo (0)

doing (1)
  idea  ideas/mr-pr-support.md  (MR/PR support)

blocked (0)

staging (1)
  ticket  tickets/deploy-pipeline.md

(no status) (1)
  idea  ideas/push-branch-after-creation.md

1 closed entry hidden (--all shows them).
```

A declared status keeps its column even when nothing is in it, since an empty stage is still part of the workflow. Closed statuses (a folder's `terminal_statuses`) are left out, with a count at the bottom; `--all` brings them back. Notes carrying no status yet gather under `(no status)`, which is your triage list, and a status no workflow declares gets its own column marked `[not in the workflow]`, which is almost always a typo worth seeing.

Without `--folder`, the columns are every folder's workflow in turn, de-duplicated in configuration order, so folders sharing a ladder share its columns. With `--folder <name>`, the board is that folder alone, in its own workflow order. `--tag` filters exactly as it does everywhere else, and `-n` caps each column.

### Filtering by status

`list` and `search` both take `--status`, alongside `--tag`:

```
selfnotes list --status doing
selfnotes list --status todo --status doing   # either one
selfnotes search "login bug" --status blocked
```

Repeating `--tag` requires every listed tag, but repeating `--status` accepts any of them: a note only ever holds one status, so several can only sensibly mean alternatives. Matching is case-insensitive, and a note with no status matches no `--status` filter.

### From your editor

The commands take a path, so an editor can hand over the buffer it is on. For Zed, [`editors/zed/tasks.json`](./editors/zed/tasks.json) is a ready-made set of [tasks](https://zed.dev/docs/tasks): show the status of the open note, pick a new one, move it one step along, or open the board, each bindable to a key. Copy it into your notes repository as `.zed/tasks.json`. See [the extension's README](./editors/zed/README.md#statuses-from-the-editor) for the details.

## Mentions

Write a colleague into a note as `@handle`. Like tags and wikilinks it is plain text, so nothing has to know about it for the note to make sense later. What a roster adds is completion: list the people you work with once, and your editor offers them as soon as you type an `@`.

### The roster

People live in a `people.toml` next to the global config, at `~/.config/selfnotes/people.toml`. `selfnotes people open` creates it from a commented template and opens it.

```toml
[[people]]
handle  = "jdoe"
name    = "Jane Doe"
email   = "jane.doe@example.com"
team    = "backend"
role    = "Tech lead"
aliases = ["jane"]
links   = [
  { name = "Chat", url = "https://mail.google.com/chat/u/0/#chat/dm/AAAAqrst" },
  { url = "https://gitlab.example.com/jdoe" },
]

[[people]]
handle = "cmartin"
name   = "Chloé Martin"
team   = "platform"
```

Only `handle` is required, and it is what you actually type after the `@`, so it cannot contain spaces. Everything else feeds the description shown beside a completion, and widens what finds the person: typing `@jane` matches an alias, `@doe` matches a word of the name, and `@jane.` matches the local part of the email address. Handles match first, then aliases, then names, then emails.

#### Links

`links` attaches places a person can be reached: their chat thread, their profile page, a shared document. Each entry needs a `url`, and an optional `name` labels it (without one, the URL's host does). Order is the order you wrote, and it matters: the first link is the one a modifier-click opens.

They show up two ways in an editor. Hovering a mention lists all of them as clickable links, and holding the modifier key while clicking the mention itself follows the first one. See [Editor completion](#editor-completion) for what each editor needs.

`selfnotes people import` never touches them: an export knows usernames, not where you talk to someone, so links you add by hand survive re-importing.

`selfnotes people` lists the roster and flags any handle that could never be typed after an `@`.

```
$ selfnotes people
@jdoe      Jane Doe (Tech lead, backend)
@cmartin   Chloé Martin (platform)
```

#### Filling it from a directory

Something already knows who works with you. `selfnotes people import` reads whatever that thing prints on standard input and folds it into the roster. Fetching and authentication stay outside: `selfnotes` reads a stream, it never calls an API.

```
glab api --paginate "groups/affluences/members/all" | selfnotes people import
```

```
7 people read from standard input, 1 skipped (no handle, or not active).
  + 1 added: @nouvelle.recrue
    6 already in the roster, left untouched

Updated /Users/you/Affluences/people.toml
```

The merge only ever adds. An entry already in the roster is left exactly as written, because `team`, `role` and `aliases` are things no export knows about and re-importing must not undo them. Comments, key alignment and entry order all survive, because the file's text is edited rather than regenerated. The result is parsed again before it is written, and anything unexpected aborts instead of overwriting.

People the source no longer lists are reported, and removed only with `--prune`. `--dry-run` prints the same report and writes nothing.

`--format` accepts `json` (the default), `tsv` and `csv`. JSON may be one array, several arrays back to back, or one object per line, which covers paginated fetches whether or not they merge the pages. Handles are read from `handle`, `username`, `login` or `nickname`, names from `name`, `full_name`, `real_name` or `display_name`, and `email`, `team` and `role` likewise, so GitLab, GitHub and Slack exports all land without reshaping. A record whose `state` is anything but `active` is skipped, which keeps blocked accounts out of your completions.

Delimited input uses a header row to name the columns when it has one, and otherwise reads the first two columns as handle then name:

```
glab api --paginate "groups/affluences/members/all" \
  | jq -r '.[] | "\(.username)\t\(.name)"' \
  | selfnotes people import --format tsv
```

Without `glab`, any authenticated fetch works the same way. A personal access token with the `read_api` scope is enough:

```
curl -sS --header "PRIVATE-TOKEN: $GITLAB_TOKEN" \
  "https://gitlab.example.com/api/v4/groups/affluences/members/all?per_page=100" \
  | selfnotes people import
```

That one call stops at the first page. For a group larger than `per_page` the pages have to be walked through the `x-next-page` response header, which is what a wrapper script is for.

`people_file` points at a different roster, so work notes can use the company one while everything else uses your own. It is a top-level key, like `journal_root` or `editor`, and it belongs in whichever [config layer](#configuration) covers those notes. Usually that is a local `.selfnotes.toml` at the root of the work tree:

```toml
# ~/work/.selfnotes.toml
journal_root = "~/work/journal"
people_file = "~/work/people.toml"
```

It can also come from a [path-scoped override](#path-scoped-overrides), which is worth the extra indirection only when the work tree has no `.selfnotes.toml` of its own. That takes two files, and the `[[overrides]]` entry has to live in the **global** config, since overrides declared in a local config are ignored:

```toml
# ~/.config/selfnotes/config.toml
[[overrides]]
path = "~/work/**"
config = "~/work/selfnotes.config"
```

```toml
# ~/work/selfnotes.config, the file the entry above points at
people_file = "~/work/people.toml"
```

`people_file` goes at the top level of that referenced config, never inside the `[[overrides]]` entry itself. An override entry only understands `path` and `config`, and anything else in it is reported as an error rather than quietly ignored.

### Editor completion

`selfnotes lsp` serves the roster over the Language Server Protocol on stdin and stdout. An editor starts it; you never run it by hand. It offers two things in Markdown buffers:

- Completion after an `@`, showing each person's name, role and team, and inserting `@handle`.
- Hover over a written `@handle`, showing who it is and listing their [links](#links) as clickable ones.
- A document link on every mention of someone who has links, so cmd-clicking (ctrl-clicking on Linux and Windows) the mention opens the first of them.

The `@` in an email address is left alone, so `jane@example.com` never opens a completion popup. The roster is re-read whenever the file changes, so adding a colleague takes effect on the next keystroke rather than the next restart. Which roster applies is resolved from the workspace the editor opened, so the override above works inside the editor too.

#### Zed

The extension in `editors/zed` starts the language server for Markdown files. It carries no binary of its own: it runs the `selfnotes` already on your machine.

1. Install `selfnotes` so it is on your `$PATH` (`cargo install --path .` from this repository).
2. In Zed, open the command palette and run `zed: extensions`.
3. Click `Install Dev Extension` and pick the `editors/zed` directory.

Zed builds the extension to WebAssembly on install, which needs the `wasm32-wasip2` target (`rustup target add wasm32-wasip2`).

Completion and hover work in any Zed. Cmd-clicking a mention to open a link needs a build from 2026-05-26 or later, when Zed gained `textDocument/documentLink` support; before that the request goes unanswered and nothing else changes. The `lsp_document_links` editor setting, which is on by default, turns the feature off if you ever want it gone.

If Zed cannot find the binary, a GUI launch inherits a shorter `$PATH` than your terminal does. Point it at the binary directly in your Zed settings:

```json
{
  "lsp": {
    "selfnotes": {
      "binary": { "path": "/Users/you/.cargo/bin/selfnotes" }
    }
  }
}
```

The server logs how many people it loaded and from where when it starts, which the `debug: open language server logs` action shows.

#### Other editors

Any LSP client works. Run `selfnotes lsp` as the server command for Markdown, with no arguments beyond `lsp`, and give it your notes directory as the workspace root.

## Configuration

Configuration is merged from up to three layers, each overriding the previous:

- Global: `~/.config/selfnotes/config.toml` (platform config directory).
- Overrides: path-scoped config files declared in the global config (see below).
- Local: the nearest `.selfnotes.toml` found by walking up from the current directory.

Scalar keys (`journal_root`, `format`, `editor`, `cursor_format`, `space_replacement`, `hash_tag_min_len`, `people_file`) and the `[journal]` section are merged field by field. `default_tags` (top level and per source) is replaced by a later layer that sets a non-empty list. Each `[[custom_folders]]` entry is matched by `name`: a later entry with the same name replaces the earlier one, and any new names are appended.

`selfnotes config new` writes the local layer for you: a `.selfnotes.toml` in the current directory, holding a commented starting point rather than an empty file. Nothing has to be registered anywhere, since a run finds it by walking up. Dropping it at the root of a tree therefore configures that whole tree, and only the nearest one applies: a copy in a subdirectory replaces its ancestor rather than adding to it. An existing file is never overwritten, so re-running the command changes nothing.

### Where a folder's entries land

A custom folder's `path` (defaulting to the folder's own name) is resolved against the journal root, so a `ticket` folder in the global config writes to `<journal-root>/tickets/`.

A folder declared in a local `.selfnotes.toml` is resolved against that file's own directory instead. A `.selfnotes.toml` marks the root of a tree, so a folder declared there belongs to that tree rather than to whatever journal root the global config happens to name:

```toml
# ~/work/project/.selfnotes.toml
[[custom_folders]]
name = "idea"
path = "ideas"
```

```
selfnotes new idea "ship it"   # -> ~/work/project/ideas/ship-it.md
selfnotes                      # -> <journal-root>/2026/08/20.md, unchanged
```

The directory is created on demand, and the rule holds wherever in the tree you run from, since the config is found by walking up: running from `~/work/project/src` still writes to `~/work/project/ideas/`.

Only the folders declared locally move. The journal is not declared in a folder, so it stays at `journal_root`, and folders that come from the global config keep resolving against it too. To put a local config's folders under a root of their own, give that config a `journal_root`: setting one means the file has said where its notes live, and its folders resolve against it exactly as the global config's do.

Entries created this way sit outside the journal root, so `selfnotes list` and `selfnotes search` print them as absolute paths rather than relative ones. They are indexed, searched, tagged and opened like any other note.

### Path-scoped overrides

The global config can declare `[[overrides]]` entries, each pairing a glob `path` with a `config` file. When the current working directory matches the glob, that config is layered on top of the global config (but below any local `.selfnotes.toml`). This lets a whole directory tree pick up its own defaults without a `.selfnotes.toml` in each project.

```toml
[[overrides]]
# When run anywhere under /Affluences, layer in the referenced config.
path = "/Affluences/**"
config = "/Affluences/afl-notes/selfnotes.config"
```

`path` also takes a list, spelled `paths` when that reads better, for one config file covering trees that share no common root:

```toml
[[overrides]]
paths = ["~/Affluences/**", "~/clients/acme/**"]
config = "~/Affluences/afl-notes/selfnotes.config"
```

Any one of the globs selects the directory. `path` and `paths` are the same key, and either spelling accepts either shape.

A leading `~` is expanded in every glob and in `config`, and `**` matches across directory separators. A trailing `/**` also matches the base directory itself, so `/Affluences/**` covers `/Affluences` as well as everything under it. Overrides are applied in declaration order, and a referenced file that does not exist is skipped.

Reach for one only when the config cannot sit at the root of what it configures. A `.selfnotes.toml` at that root already covers the whole tree, without a second file, without absolute paths, and without going stale when the directory is renamed or moved. The override earns its indirection when the config has to live somewhere else, when you would rather not write into the tree at all, or when one config has to cover several trees that share no root.

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
# What spaces in a `selfnotes new` name become in the file name (omit to keep them).
space_replacement = "-"
# All-hex inline #tokens this long or longer are treated as git hashes, not tags (default 6; 0 disables).
hash_tag_min_len = 6
# Roster of people completed after an `@` (defaults to people.toml beside this file).
people_file = "~/.config/selfnotes/people.toml"
# Default workflow for folders that declare none of their own (omit to track no statuses).
statuses = ["backlog", "todo", "doing", "blocked", "done", "dropped"]
# What a new entry starts in; defaults to the first status above.
default_status = "backlog"
# Statuses that close an entry, which `selfnotes board` hides unless given --all.
terminal_statuses = ["done", "dropped"]

[journal]
# Optional template rendered into new journal entries.
template_file = "~/.config/selfnotes/templates/journal.md"
# Journal entries are also tagged #daily.
default_tags = ["daily"]
# Section of the previous entry that {{last_day.tasks}} and {{last_day.todo}} read (default "Today").
carry_over_section = "Today"

[[custom_folders]]
name = "ticket"
# Directory under the root; defaults to the folder name if omitted.
path = "tickets"
template_file = "~/.config/selfnotes/templates/ticket.md"
# Optional per-folder extension override.
format = "md"
# Tickets are also tagged #work.
default_tags = ["work"]
# Replaces the top-level statuses for this folder's entries.
statuses = ["backlog", "todo", "doing", "staging", "prod"]
terminal_statuses = ["prod"]

[[custom_folders]]
name = "idea"
# path omitted -> entries land in <journal-root>/idea/
# Optional per-folder override of the top-level space_replacement.
space_replacement = "_"
```

Running `selfnotes new` with no folder shows a picker of the configured folder names (via `dialoguer`), then prompts for the entry name:

```
? Folder ›
❯ ticket (global)
  idea (local)
```

Naming a folder that is not configured lists the ones that are, so a typo does not send you back to the config file to remember them:

```
selfnotes new note
Error: no folder `note` is configured (`ticket`, `idea` expected)
```

Once the folders come from more than one config, each name says which one declared it, since that is when a name being missing (or being not quite the one you meant) is worth tracing to a file:

```
selfnotes new note
Error: no folder `note` is configured (`ticket` (global), `idea` (local) expected)
```

The picker and the error label the folders identically, and both drop the label when a single config declares them all. The label is the config that actually won: a folder declared globally and redeclared in a local `.selfnotes.toml` is reported as `local`, because that is the declaration in effect. `--folder` on `list`, `search` and `tags` reports the same list, with `journal` in it.

### Spaces in entry names

Entry names are allowed to contain spaces, and by default the file keeps them: `selfnotes new ticket "login bug"` writes `login bug.md`. Set `space_replacement` to file those names under a separator instead.

```toml
space_replacement = "-"
```

With that set, `selfnotes new ticket "login bug"` writes `login-bug.md`. Only the file name changes. The template still receives the name exactly as typed, so the entry opens on `# login bug` rather than on its file name. A folder can override the top-level value with its own `space_replacement`, and the key is unset by default, which leaves every name as typed.

Runs of whitespace collapse into a single replacement and any leading or trailing whitespace is dropped, so `"  login   bug "` and `"login bug"` both land at `login-bug.md`. The empty string is a meaningful value: `space_replacement = ""` files that entry as `loginbug.md`.

A name that already contains no spaces is untouched, so switching the key on does not rename or shadow anything you created before. Existing files are never renamed either: the key only shapes names at creation time, and an entry whose file already exists is reopened rather than rewritten.

### Validating the configuration

`selfnotes config validate` checks every config file that contributes to the effective configuration and prints a verdict (`valid` / `INVALID`) per file, with each problem attributed to the file it came from. It exits non-zero when any file is invalid, so it fits in scripts and pre-flight checks. The blocking problems are: an unset `journal_root`, a custom folder whose directory would escape the root it resolves against (via its `path` or a name containing `..`), a folder `name` containing a path separator (`/` or `\`), a `space_replacement` (top level or per folder) containing a path separator, a referenced `template_file` (journal or folder) that does not exist, and a workflow that does not hold together: a blank or repeated status, or a `default_status` or `terminal_statuses` naming a step its `statuses` does not have. A `field_order` entry naming a field that no field declares is reported as a warning rather than an error, as is a `default_status` or `terminal_statuses` set where no `statuses` are declared at all.

Each folder is checked on the status keys it declares itself, against the ladder it ends up with, so a key it merely inherits from the top level is reported once, where it is written, rather than once per folder.

Each file is judged on what actually takes effect: a folder or journal template shadowed by a higher-priority layer is not re-checked, and folder directories are resolved against the effective journal root even when a given file does not set its own root. The set of files checked mirrors what loading merges: the global config, each matching override's referenced config, and the nearest local `.selfnotes.toml`.

The command also inspects the `[[overrides]]` declared in each config file. An invalid glob is an error, and every glob of an entry is checked even once one of them has matched, so a typo cannot hide behind a sibling that works. An entry declaring no glob at all is reported as a warning, since it never applies. For an override in the global config, it reports whether the glob matches the current directory and checks the referenced config file exists (an error when the override matches here, a warning otherwise); the referenced config's own folders and templates are validated as one of the checked files. An override declared in a local config is reported as ignored, since only the global config's overrides are applied, along with whether its glob would even have matched, which is the usual reason such an override looks like it does nothing.

Entry names are held to the same rule at creation time: a `name` containing a path separator, `..`, or an absolute path is rejected before anything is written, so an entry can never be created outside the journal root. The name is checked again after `space_replacement` has been applied, so a replacement that would push the entry out of its folder fails there too rather than at the write.

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
| `{{date}}`     | `2026-07-13`       | the entry's date           |
| `{{datetime}}` | `2026-07-13 09:05` | the entry's date, the current time |
| `{{time}}`     | `09:05`            | always the current time    |
| `{{year}}`     | `2026`             | from the entry's date      |
| `{{month}}`    | `07`               | zero-padded                |
| `{{day}}`      | `13`               | zero-padded                |
| `{{weekday}}`  | `Monday`           | from the entry's date      |
| `{{name}}`     | `login-bug`        | custom-folder entries only |
| `{{<folder-name>.<field>}}` | `high` | custom folder fields, e.g. `{{ticket.priority}}` |
| `{{last_day.tasks}}` | `- [x] ship it` | journal entries only, see [Carrying the last day forward](#carrying-the-last-day-forward) |
| `{{last_day.todo}}`  | `- [ ] ship it` | journal entries only, the unfinished half of the same list |
| `{{last_day.weekday}}` | `Wednesday`  | journal entries only, the day that list came from |
| `{{last_day.date}}`  | `2026-08-12`     | journal entries only, that same day as a date |

"The entry's date" is today unless `selfnotes journal --date` asked for another day (see [Journal dates](#journal-dates)); for custom-folder entries it is always today.

Unknown placeholders are left untouched so typos stay visible.

A placeholder that resolves to several lines keeps its own indentation on every line, as long as nothing but whitespace precedes it. That is what lets a checklist sit under the list item it belongs to.

### Carrying the last day forward

A day rarely ends with everything ticked off, so a new journal entry can start from the last one. Four placeholders describe the most recent entry *before* the day being created:

| Placeholder            | What it renders |
| ---------------------- | --------------- |
| `{{last_day.tasks}}`   | every checkbox item of its carry-over section, ticked or not |
| `{{last_day.todo}}`    | only the items still unticked |
| `{{last_day.weekday}}` | the weekday that entry is for, e.g. `Wednesday`, so the section is headed by a day |
| `{{last_day.date}}`    | that same day as `YYYY-MM-DD` |

```markdown
# Daily {{date}}

- {{last_day.weekday}}:
  {{last_day.tasks}}

- Today:
  {{last_day.todo}}
```

Creating Tuesday the 18th after an entry for Monday the 17th whose `Today` section held `- [x] ship it` and `- [ ] write the postmortem` gives:

```markdown
# Daily 2026-08-18

- Monday:
  - [x] ship it
  - [ ] write the postmortem

- Today:
  - [ ] write the postmortem
```

The details, in short:

- **Which entry.** The most recent one before the new day, whatever the gap: a Monday picks up the Friday before it. A backfilled `--date` reads the entry before *that* day, so backfilling stays consistent.
- **Which day is named.** `{{last_day.weekday}}` and `{{last_day.date}}` describe the entry the checklist came from, not the calendar day before, so the heading always says where the list under it was taken from. After a week away, the heading names the day a week ago.
- **Which section.** `Today` by default, matched on its text alone, so a `- Today:` list item and a `## Today` heading both work. Rename it with `carry_over_section` under `[journal]`. Only that section is read, which is what keeps the carried section from being copied forward twice, whatever it is headed with.
- **Nesting.** Sub-items are kept. An unfinished child of a ticked parent moves up to the parent's level in `{{last_day.todo}}` rather than dangling under an item that is not there.
- **Marks.** `- [ ]` (and `- []`) is unfinished. `- [x]`, `- [X]`, and one-character conventions like `- [-]` for cancelled all count as finished: they show up in `tasks` but are not carried into `todo`.
- **Nothing to carry.** With no previous entry, no such section, or no checkboxes in it, `tasks` and `todo` render a single empty bullet (`-`), exactly what a template without them would have left. The heading is still a day: with no earlier entry to name, `{{last_day.weekday}}` and `{{last_day.date}}` fall back to the last working day before the new one, so a Monday says `Friday` rather than `Sunday`.

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

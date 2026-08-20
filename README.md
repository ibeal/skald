# skald

`skald` is a CLI for **ticket stores** — directories of markdown tickets whose YAML frontmatter is
machine-read.

The name is Old Norse for the poet who kept the saga. Its sibling `rata`
([ratatoskr](https://github.com/ibeal/ratatoskr)) is named for the *messenger*; naming this one for
the *chronicler* rather than the chronicle keeps that metaphor consistent.

Its eventual job is to be the **only** read/write path to a ticket directory: the enforcement
mechanism is a blanket permission deny on those directories, so an agent that can't express an
operation through `skald` can't perform it at all. That makes coverage a correctness requirement, not
a convenience.

## Install

```text
nix run .#
nix build .#
nix profile install .#
nix develop          # dev shell
```

## The active store

The store is `$SKALD_STORE`, an **absolute** path to a ticket directory:

```nu
$env.SKALD_STORE = "/Users/you/dotfiles/agents/tickets"
```

Exactly one store is active per invocation. There is no store list and no `--store` flag —
per-directory switching (direnv) covers that need.

**If `$SKALD_STORE` is unset, `skald` fails.** It never guesses a default, never falls back to the
current directory, and never searches upward. Writing a ticket into the wrong store is a worse
outcome than refusing to run.

## Commands

```text
skald show <id> [--section NAME] [--json]
skald list [--phase P] [--project X] [--json]
```

- `show` with no `--section` prints the whole ticket, byte for byte, so a resuming agent orients in
  one call. `<id>` resolves with or without the `.md` extension.
- `--section` matches loosely: `Build log`, `build-log`, and `BUILD LOG` are the same section. A
  section is a heading and everything beneath it, so `--section Journal` returns the whole journal.
  Headings are ATX only (`## Title`) — a Setext underline is not a section. Every section has a
  distinct address, so two headings that would collide get `notes`, `notes-2`; an error that lists the
  available sections names the address whenever it differs from the title.
- `list` filters on declared frontmatter keys only. `--json` is the complete data; without it you get
  a compact aligned table.

Three names in a store are not tickets: dotfiles (`.schema.toml`) are store metadata, `_`-prefixed
files are templates, and `README.md` documents the store. Everything else is a ticket even if it is a
malformed one — a broken ticket that quietly vanished from `list` would be the exact failure this tool
exists to catch.

## The store schema

Each store declares its own frontmatter contract in `.schema.toml` at its root — the valid keys,
which are required, and the permitted values for each enum field:

```toml
#:schema ../schema/skald.schema.json

[keys.phase]
type = "enum"
required = true
allow_empty = false
values = ["intake", "build", "review", "merged"]

[keys.pr]
type = "url"
required = true
```

Stores are independent: a personal store's `spec`/`projects` and a work store's `linear`/`services`
share no configuration and neither knows the other exists. `schema/skald.schema.json` describes the
file's shape, so an editor carries the contract as hover text where it's authored.

The schema is read from the store root only — there is no upward search, for the same reason there is
no fallback for the store itself. A store with **no** `.schema.toml` still reads fine; only the
operations that need a contract report the one clear error.

## Round-tripping is the load-bearing property

A ticket is held as its bytes plus byte spans into them. Reading and rewriting one preserves
**everything not explicitly modified** — frontmatter comments, key order, blank lines, prose,
alignment padding, trailing annotations. An agent appending a single Build log line must not silently
reformat the file it appended to, and the guarantee is a property of the representation rather than
of careful re-serialization.

## Status

Read side and the round-trip foundation. `check` (validation), the write commands (`new`, `set`,
`append`, `set-section`), the `docs` subcommand, and the `docket` alias are separate slices; see
`tickets/`.

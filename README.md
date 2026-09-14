# skald

`skald` is an opinionated CLI for a **ticket store** — a directory of markdown tickets whose job is to
keep an AI agent's context organized across sessions.

It is not a Linear clone and does not try to mirror one. If a ticket came from somewhere else, `link`
points back at it; everything else is skald's own small fixed shape.

The name is Old Norse for the poet who kept the saga. Its sibling `rata`
([ratatoskr](https://github.com/ibeal/ratatoskr)) is named for the *messenger*; naming this one for the
*chronicler* rather than the chronicle keeps that metaphor consistent.

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

## Global configuration

Skald reads an optional `~/.config/skald/config.toml` (or
`$XDG_CONFIG_HOME/skald/config.toml`) for global defaults:

```toml
store = "/Users/you/.skald/tickets"
state_change_webhook = "https://example.com/skald-events"
```

`store` is required through either this file or `$SKALD_STORE`; it must be an absolute path.
`SKALD_STORE` and `STATE_CHANGE_WEBHOOK` override their configuration-file counterparts for one
process. An empty `STATE_CHANGE_WEBHOOK` disables webhook delivery for that process.

Exactly one store is active per invocation. There is no store list and no `--store` flag. If neither
configuration source supplies a store, skald fails rather than guessing. A symlink is likewise not a
way out of the store.

## Commands

```text
skald new <id> [--title T] [--status S] [--repo R]... [--link U] [--parent ID]
skald claim <id> [--title T] [--status S] [--repo R]... [--link U] [--parent ID]
skald set <id> [--title T] [--status S] [--paused R] [--repo R]...
               [--branch B] [--link U] [--pr U] [--parent ID]
skald ac   <id> [<text> | --stdin]
skald log  <id> [<text> | --stdin]
skald show <id> [--section NAME] [--json]
skald list [--status S] [--repo R] [--parent ID] [--paused] [--json]
skald check [--fix]
skald docs [agent]
```

`set` takes several fields at once, which is one write, one `updated` bump, and one log line rather
than three. An empty string clears any field uniformly: `--paused ""` resumes, `--pr ""` unsets.

`claim` is the race-safe creation path for a queue. Give it an id deterministically derived from the
thing being claimed (for example, a canonical PR); it prints `created <id>` for the winner or
`existing <id>` for every loser, without changing the existing ticket.

`check` exits `2` when the store is invalid and `1` when skald itself failed, so a pre-commit hook can
tell them apart:

```text
skald check || exit 1
```

## State-change webhook

Set `state_change_webhook` in the global config (or use `STATE_CHANGE_WEBHOOK` for one process) to
receive best-effort JSON notifications when a ticket's visible state changes. A non-empty `paused` reason derives the visible state `paused`;
changing that reason while it remains non-empty does not emit another event. Delivery is attempted
three times after the ticket write commits, using one stable UUID event id across those attempts.
Webhook failures warn on stderr but do not roll back the ticket write.

`--fix` repairs shape only — it never guesses at a status and never renames an old key, because those
are meaning rather than shape and getting them wrong discards information.

`skald docs` prints the agent-facing guide, and resolves no store on purpose: it is the recovery path
for an agent that cannot work out the interface, and "the store is not configured" is one of the things
it explains.

- `show` with no `--section` prints the whole ticket, byte for byte, so a resuming agent orients in
  one call. `<id>` resolves with or without the `.md` extension.
- `--section` matches loosely: `Build log`, `build-log`, and `BUILD LOG` are the same section, and `ac`
  addresses `Acceptance criteria`. A section is a heading and everything beneath it. Headings are ATX
  only (`## Title`) — a Setext underline is not a section. Every section has a distinct address, so two
  that would collide become `notes` and `notes-2`.
- `list` renders `ID · STATUS · TITLE · UPDATED`, marking a parked row with `!`. `--json` is the
  complete data.

Three names in a store are not tickets: dotfiles are store metadata, `_`-prefixed files are templates,
and `README.md` documents the store. Everything else is a ticket even if it is a malformed one — a
broken ticket that quietly vanished from `list` would be the exact failure this tool exists to catch.
For the same reason, **reads never require a valid ticket**: one unreadable file is named on stderr and
the rest of the listing still prints.

## The contract

skald owns the field set. There is no schema file, and an unknown key is a violation rather than an
extension point.

```yaml
title:                  # required, non-empty
status:                 # required. refining | designing | building | reviewing | done | cancelled
paused:                 # optional free text; present means parked
repos: []               # required, may be empty
branch:                 # optional
link:                   # optional — the upstream ticket or issue, if any
pr:                     # optional
parent:                 # optional ticket id
created:                # YYYY-MM-DD, managed by skald
updated:                # YYYY-MM-DD, managed by skald
```

There is **no `id` field** — the filename is the identity, and a field whose only job is to agree with
the filename is a field that can disagree with it. There is **no H1** either, for the same reason:
`title` lives in the frontmatter.

`status`, `paused`, `repos`, `pr`, and `parent` are there to be filtered on, `title` to be displayed,
and the dates to sort by. `branch` and `link` are neither — they're pointers, and they earn their place
because a resuming agent needs them and prose is a bad home for a pointer.

## Status

```text
refining ⇄ designing ⇄ building ⇄ reviewing ──▶ done
    ⇅          ⇅          ⇅
    └──────────┴──────────┴───────▶ cancelled
```

A status names the phase that **owns** the ticket, and it advances the moment the previous phase
finishes — not when work starts. `designing` means "refining is done and design is the outstanding work",
whether or not anyone has begun. That's the only reading an agent can apply without guessing at intent.

Movement among the four live states is free in any direction: whether a review finding is a small fix,
a large hole in the implementation, or a problem with the AC itself is judgment, not something a tool
can adjudicate. `done` and `cancelled` are terminal — follow-up work is a new ticket with `parent:`
pointing back.

`paused` is deliberately **not** a status. A `status: paused` would destroy which phase the ticket was
paused from, which is the same defect as writing `reviewing (pending Ian)`. Blocked is simply a pause
whose reason is external, so it needs no state of its own.

## Body

The body is **closed**, exactly like the field set: two sections and nothing else at that level.

```markdown
## Acceptance criteria

Written while refining. Frozen once the ticket leaves refining.

### Subsections belong to the author

## Log

- 2026-08-20: status refining → building
- 2026-08-20: chose bytes-plus-spans; comments and key order survive by construction
```

Only the top level is fixed — structure *inside* a section is yours, so the acceptance criteria can
carry as many `###` subsections as you like.

The acceptance criteria may only be written while `status: refining`, and there is no override flag. To
change them you move the ticket back to `refining` — which is precisely the act you're performing, and
it leaves a trace. The log is append-only, because rewriting an audit trail destroys the thing that
makes it worth reading on resume.

`designing` is the architect's planning phase, not a new artifact type. Decisions and the build
approach go in append-only log entries, so the body remains closed to acceptance criteria and log.
A controller grants a design worker `skald log` but retains `skald set --status`; the worker can
record its reasoning but cannot transition itself to building.

There is no general-purpose section writer, which is why the body can be closed honestly rather than
nominally: a section nothing can write is a section that doesn't exist. Anything the fixed shape can't
express goes in a log entry — which is where a reader resuming the ticket is already looking.

## Round-tripping is the load-bearing property

A ticket is held as its bytes plus byte spans into them. Reading and rewriting one preserves
**everything not explicitly modified** — frontmatter comments, key order, blank lines, prose, alignment
padding, trailing annotations. An agent appending a single log line must not silently reformat the file
it appended to, and the guarantee is a property of the representation rather than of careful
re-serialization.

## `docket`

`docket` is a supported second name for the same binary — "a register of matters awaiting action",
for anyone who doesn't want the Norse theming. It lost as the primary name because it shares a
four-character prefix with `docker`, so shell completion can't disambiguate until the fifth keystroke
and the two misread for each other at a glance. **`skald` stays canonical** in the docs and in agent
instructions, so there's one name in the corpus.

## Rules skald enforces

1. Enums take bare values — no qualifier, date, or parenthetical.
2. `done` and `cancelled` are terminal.
3. Acceptance criteria may only be written while `refining`. No override flag.
4. The log is append-only; `skald log` is its only writer.
5. A status change appends a dated log line by itself.
6. `created` and `updated` are skald-managed.
7. Every mutation preserves the rest of the file byte-for-byte.
8. A mutation that would fail `skald check` is refused instead. skald cannot create a violation it
   would later report.
9. New and updated titles may not contain `:`; legacy titles remain readable until deliberately
   migrated.

## Status of the tool

Complete: the read side, `check`, the write side, and `docs`. Remaining work is the rollout in the
dotfiles repo — migrating the existing stores, and the permission deny that makes skald the only path
in. See `tickets/`.

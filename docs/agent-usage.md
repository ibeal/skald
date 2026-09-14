# skald — for agents

You are reading this because a ticket store is the only durable memory you have across sessions, and
`skald` is the only way in or out of it. The ticket directory is denied to direct file access, so
`cat`, `Read`, `Edit`, and `grep` will not work on it. That is deliberate. Everything you need is here.

The point of a ticket is **resume**: someone — probably you, in a session that does not remember this
one — should be able to read one ticket and know what "done" means, where the code is, and what has
happened so far.

## The store

One store per invocation, from `~/.config/skald/config.toml` (`store = "/absolute/path"`) or the
per-process `$SKALD_STORE` override. If neither is configured, skald refuses to run rather than guess.

## Reading

```text
skald list                                  # everything, as a table
skald list --status building                # what needs building
skald list --paused                         # what is parked, and why
skald list --parent <id>                    # slices of a parent ticket
skald list --json                           # the complete data
skald show <id>                             # the whole ticket, verbatim
skald show <id> --section log               # just the log
skald show <id> --section ac                # just the acceptance criteria
```

**On resume, run `skald show <id>` first.** It returns the whole ticket in one call, which is faster
and less error-prone than three narrower reads.

## Writing

```text
skald new <id> [--title T] [--status S] [--repo R]... [--link U] [--parent ID]
skald claim <id> [--title T] [--status S] [--repo R]... [--link U] [--parent ID]
skald set <id> [--title T] [--status S] [--paused R] [--repo R]...
               [--branch B] [--link U] [--pr U] [--parent ID]
skald ac  <id> [<text> | --stdin]
skald log <id> [<text> | --stdin]
```

Five write commands, and there are no others. A ticket is frontmatter, acceptance criteria, and a log;
each has exactly one writer. If you are looking for a way to add a section, there isn't one — see
*The body is closed* below.

`set` takes several fields at once, and you should prefer that: it is one write, one `updated` bump,
and one log line instead of three.

```text
skald set my-ticket --status reviewing --branch my-branch --pr https://github.com/o/r/pull/1
```

An empty string clears any field: `--paused ""` resumes a parked ticket, `--pr ""` unsets a PR.

Use `claim` when concurrent callers must share one deterministic ticket identity, such as a review
queue keyed by a canonical PR. It prints `created <id>` for the winner or `existing <id>` for a
loser, and a loser never changes the ticket it found.

Use `--stdin` for anything multi-line. Shell quoting is the single most common way to write a mangled
log entry.

**Feed `--stdin` from a file, not a heredoc.** Write the text to a scratch file, then redirect:

```text
skald log my-ticket --stdin < /path/to/entry.md
skald ac  my-ticket --stdin < /path/to/criteria.md
```

`cat <<'EOF' | skald log …` does the same job and is worse in one specific way: a permission
allowlist entry for `skald` matches a line whose command *is* skald, and that line's command is
`cat`. A pipeline needs every segment permitted and a heredoc often can't be analysed statically, so
the agent stops for an approval it did not need. A redirect adds no command, so it still matches.

Most log entries are one line and need none of this — pass the text as an argument.

## Status

```text
refining ⇄ designing ⇄ building ⇄ reviewing ──▶ done
    ⇅          ⇅          ⇅
    └──────────┴──────────┴───────▶ cancelled
```

A status names the phase that **owns** the ticket, and it advances the moment the previous phase
finishes — *not* when work starts. Set it to `designing` as soon as refining is done, even if nobody
is going to design it today. `designing` means "refining is finished and design is the outstanding
work"; `building` means the design handoff is complete and implementation is outstanding.

Movement among the four live states is free in **any** direction. If review turns up a small fix,
stay in `reviewing`. If it turns up a large hole in the implementation, go back to `building`. If the
acceptance criteria themselves are wrong, go back to `refining`.

`done` and `cancelled` are terminal. Nothing moves out of them — follow-up work is a **new ticket**
with `--parent` pointing back.

### A status takes one bare value

Never `building (pending Ian)`. Never `— (no PR yet)` in `pr`. Decoration makes the ticket invisible
to every filter, which defeats the reason the field exists. skald will refuse it.

When the state needs explaining, **set the bare value and put the explanation in the log**:

```text
skald set my-ticket --status reviewing
skald log my-ticket "PR is up but waiting on Ian to look before merging"
```

That is not a workaround. It is the design: enforcement plus somewhere to put the thing being enforced
away.

### Designing

`designing` is the architect's planning phase, not a new ticket artifact. Record design decisions and
the build approach in append-only log entries. A controller can grant a design worker `skald log` but
retain `skald set --status`, so the worker can record reasoning without moving the ticket to building.

### Pausing

`paused` is a field, not a status, so a paused ticket keeps its phase and you never have to guess
where to resume it.

```text
skald set my-ticket --paused "blocked on the upstream API change"
skald set my-ticket --paused ""      # resume; still `building`
```

"Blocked" is just a pause whose reason is external. It needs no state of its own.

## The acceptance criteria are frozen outside refining

`skald ac` works **only** when `status: refining`. Any other status and it refuses.

There is no `--force`, and looking for one is the wrong instinct. If you have found a real reason the
criteria are wrong — a constraint in the codebase, a contradiction in the requirements — then what you
are doing *is* re-refining, so say so:

```text
skald log my-ticket "the AC assume a sync API; the client is async only"
skald set my-ticket --status refining
skald ac  my-ticket --stdin < new-criteria.md
skald set my-ticket --status designing
```

That leaves a trace. Quietly editing the criteria to match what you built would not, and it is the
specific failure this rule exists to prevent: **do not bend the acceptance criteria to fit the code.**

## The log is append-only

`skald log` is its only writer. Nothing can rewrite or trim it, because a trail you can edit is not
worth reading.

Log **as you go**, not at the end. The value of an entry is highest for the session that did not write
it. Worth logging: a decision and why, a surprise in the code, something you tried that did not work,
a thing you deliberately left out. A status change logs itself, so you do not need to.

## The body is closed

A ticket has `## Acceptance criteria` and `## Log`. That is all. There is no command to create another
section, and an unrecognized `##` heading is a violation that `skald check` reports.

Deeper headings *inside* a section are yours — `###` and below in the acceptance criteria are fine.

If something does not seem to fit either section, it goes in the log. That is where a reader resuming
the ticket is already looking, and it is the reason the body can be closed without stranding you.

## Checking

```text
skald check          # exits 2 if anything is wrong, silent when clean
skald check --fix    # repairs shape only
```

`--fix` never guesses at a status and never renames an old key — those are meaning, not shape, and it
would be discarding information. It exits non-zero when anything is left.

## When a write is refused

Read the error. Every refusal names the way through, and the way through is never to route around
skald:

| Refusal | What it means |
|---|---|
| `is not a status` | Set the bare value; put the qualifier in the log. |
| `frozen outside refining` | Go back to `refining` if the change is deliberate. |
| `is terminal` | Make a new ticket with `--parent`. |
| `would not pass skald check` | The write would introduce a violation — usually a URL field holding prose. What was *already* wrong does not block you. |
| `may not contain a line break` | A frontmatter value is one line. Prose goes in the log. |
| `must be a single value` | You passed something list- or map-shaped where one value belongs. |
| `refusing to write empty …` | Usually a `--stdin` redirect that read nothing. |
| `already exists` | Ids are permanent; there is no rename. Pick a new one. |
| no active ticket store | Configure `store` in `~/.config/skald/config.toml` or set `$SKALD_STORE`; do not guess a path. |

A write is refused only for a violation it would *introduce*. A ticket that is already
non-conforming — a half-migrated one, say — can still be logged to and advanced, so you are never
locked out of recording progress.

Two things you cannot do at all: fill `created`/`updated` (skald manages them), and delete or rename a
ticket. If you need either, or you genuinely cannot express something through these commands, say so to
a human. It is a gap in the tool, not something to work around — there is no way around.

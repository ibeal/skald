---
title: "The write side: new, set, log, append, set-section"
status: building
paused:
repos:
  - skald
branch:
link:
pr:
parent: ask-2026-08-19-skald-ticket-cli
created: 2026-08-20
updated: 2026-08-20
---

## Acceptance criteria

**Context.** The mutation half. Once the permission deny lands these commands are the *only* way an
agent can write a ticket, so their coverage determines whether agents can work at all. This is also
where the two structural guarantees live: the acceptance criteria cannot drift to fit the code, and
the log cannot be rewritten.

### Commands

```
skald new <id> [--title T] [--status S] [--repo R]... [--link U] [--parent ID]
skald set <id> [--title T] [--status S] [--paused R] [--repo R]...
               [--branch B] [--link U] [--pr U] [--parent ID]
skald log <id> <text>
skald append <id> --section NAME <text | --stdin>
skald set-section <id> --section NAME [--stdin]
```

- **`new`** scaffolds a ticket with the full frontmatter shape, fills `created`/`updated`, defaults
  `status` to `refining`, and writes empty `## Acceptance criteria` and `## Log` sections. Refuses to
  overwrite an existing ticket. There is no template file — the shape is compiled in, so there is
  nothing to keep in sync.
- **`set`** takes a named flag per field, so several fields land in **one atomic write with one
  `updated` bump**: `set ID --status reviewing --pr <url>`. It rejects any `status` outside the enum,
  **including a valid value with anything appended**. An empty string clears a field uniformly —
  `--paused ""` resumes, `--pr ""` unsets, `--repo ""` empties the list.
- **`log`** appends a dated bullet to `## Log`, creating the section if absent. This is the outlet
  that makes the enum rule livable: an agent that wants to record "reviewing, but pending Ian" sets
  the bare value and logs the qualifier. Enforcement needs somewhere to put the thing it rejected.
- **`append`** adds to any section verbatim, with no date. `--stdin` for multi-line content, because
  a Build-log-sized entry through shell quoting is how agents get it wrong.
- **`set-section --stdin`** replaces a section body wholesale. This is the deliberate escape hatch for
  content the typed commands cannot express; without it, a blanket deny strands any operation nobody
  anticipated.
- Section names resolve case- and punctuation-insensitively, and `ac` addresses
  `## Acceptance criteria`, because agents will not reproduce a heading exactly.

### The rules skald enforces

1. **Enums take bare values.** No qualifier, date, or parenthetical on `status`.
2. **`done` and `cancelled` are terminal.** `set --status` refuses to move a ticket out of either;
   follow-up work is a new ticket with `parent:` pointing back. This is the only transition rule —
   movement among `refining`, `building`, and `reviewing` is free in any direction, because a review
   finding a small fix versus a large hole is judgment, not something a tool can adjudicate.
3. **Acceptance criteria are frozen outside `refining`.** Any write to that section — `append` or
   `set-section` — is refused unless `status: refining`, with an error that names the way through:
   move the ticket back to `refining`. There is deliberately **no `--force`**. An override flag
   becomes muscle memory and leaves no trace, whereas a status round-trip is recorded, bumps
   `updated`, and appears in the log. This is what makes "agents cannot bend the AC to fit the code"
   mechanical rather than advisory, while still allowing a deliberate scope change from a real
   finding.
4. **The log is append-only.** `set-section` refuses `## Log`; `log` is the only writer. Rewriting an
   audit trail destroys the thing that makes it worth reading on resume.
5. **A status change appends a dated log line automatically** (`status building → refining`). The
   phase history builds itself, so a session that dies mid-build still leaves a trail. Only status
   changes do this — logging every mutation was considered and rejected, because `set pr` lines would
   dilute the resume surface that is the log's whole purpose.
6. **`created` and `updated` are skald-managed** and cannot be set by hand.
7. Every mutation preserves the rest of the file byte-for-byte, per the round-trip guarantee from the
   core-read slice.
8. A mutation that would produce a ticket failing `skald check` fails instead, with the same message
   `check` would give. **The tool must not be able to create a violation it would later report.**

### Coverage requirement (blocking for the parent's rollout)

Every operation the workflow requires must be performable through these commands with **no direct file
access**: create a ticket; read the whole thing on resume; write the AC while refining; move status at
each boundary; record the branch and the PR URL; pause with a reason and resume; append narrative
progress; and write free-form sections for anything the fixed shape does not cover. **Walk the
workflow end to end against the built CLI and record the result in this log** — that walkthrough is
what unblocks the deny, and a gap found here is far cheaper than one found after the deny lands.

**Explicitly out of scope:**

- The permission deny and the instruction rewrite — parent ticket, dotfiles side.
- Interactive editing. `$EDITOR` integration is for humans and is not needed for enforcement.

**Verification:** the walkthrough above completes with no direct file access. A ticket mutated by
every command still passes `skald check` and differs from the original only in the intended lines.
Writing the AC at `status: building` is refused; the same write after `set --status refining`
succeeds. `set-section` on the log is refused. A `set --status` leaves a log line without being asked.

## Log

- 2026-08-20: status refining → building. AC authored, then substantially revised once the schema
  became compiled-in and the body gained a fixed shape.
- 2026-08-20: `set` moved from `set <id> <key> <value>` to a named flag per field. With the field set
  closed there is no reason to pass a key as data, and flags buy atomic multi-field writes with a
  single `updated` bump plus shell completion.
- 2026-08-20: Dating resolved by splitting `log` out of `append` rather than adding a `--date` flag or
  sniffing section names for the word "log". A heuristic would silently fail to date a section called
  "Timeline", and an opt-in flag is one agents forget inconsistently. A separate command for the one
  section skald owns has no ambiguity to get wrong.
- 2026-08-20: The AC freeze uses the state machine rather than a new mechanism, and deliberately ships
  without an override. Resolved at the same time: the old open question about whether `append` should
  create a missing section — yes, create. An agent should not have to know whether the shape happened
  to include the section it needs.

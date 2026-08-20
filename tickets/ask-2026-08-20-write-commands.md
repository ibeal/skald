---
id: ask-2026-08-20-write-commands
source: plain-ask
spec: authored here
mode: direct
phase: intake
pr:
projects:
  - skald
created: 2026-08-20
updated: 2026-08-20
---

# ask-2026-08-20-write-commands — `new`, `set`, `append`, `set-section`

> Slice 3 of 4. Parent spec: `dotfiles/agents/tickets/ask-2026-08-19-skald-ticket-cli.md`.
> Depends on `ask-2026-08-20-core-read`.

---

## Spec — acceptance criteria

**Context.** The mutation half. Once the permission deny lands, these commands are the *only* way an
agent can write a ticket — so their coverage determines whether agents can work at all. Frontmatter
is typed and validated; prose is section-addressed.

**Refined (after intake):**

```
skald new <id> [--source S] [--project P]...
skald set <id> <key> <value>
skald append <id> <section> <text>
skald set-section <id> <section> [--stdin]
```

- **`new`** scaffolds from the store's template, fills `id`/`created`/`updated`, and applies the
  store's defaults (`mode: direct`, `phase: intake`). Refuses to overwrite an existing ticket.
- **`set`** validates against `.schema.toml` and **rejects any value outside the enum, including a
  valid value with anything appended.** Bumps `updated` on success. `pr` takes a URL or empty,
  nothing else.
- **`append`** adds to a section, dating the entry where the section is a log. This is the outlet
  that makes the enum rule livable: an agent that wants to record "review, but pending Ian" sets the
  bare value and appends the qualifier. Enforcement needs somewhere to put the thing it rejected.
- **`set-section --stdin`** replaces a section body wholesale. This is the deliberate escape hatch
  for content the typed commands can't express — the slice manifest table, cross-repo contracts, a
  rewritten AC list. Without it, a blanket deny strands any operation nobody anticipated.
- Every mutation preserves the rest of the file byte-for-byte (the round-trip guarantee from slice 1).
- Section names resolve case- and punctuation-insensitively (`build-log`, `"Build log"`), because
  agents will not reproduce the heading exactly.
- A mutation that would produce a ticket failing `skald check` fails instead, with the same message
  `check` would give. **The tool must not be able to create a violation it would later report.**

### Coverage requirement (blocking for the parent's rollout)

Every operation `workflow/sdlc.md` requires must be performable through these four commands: create
from template; read the whole ticket on resume; write the Intake block; refine the AC; set `phase` at
each boundary; record the PR URL; append dated Build log entries; write Review findings, Open
questions, and Checkpoints; and in orchestrated mode write the slice manifest and cross-repo contract
tables. **Walk `sdlc.md` end to end against the built CLI and record the result in this journal** —
that walkthrough is what unblocks the deny, and a gap found here is far cheaper than one found after
the deny lands.

**Explicitly out of scope:**

- The permission deny and instruction rewrite — parent ticket, dotfiles side.
- Interactive editing. `$EDITOR` integration is for humans and isn't needed for enforcement.

**Verification:** the `sdlc.md` walkthrough above completes with no direct file access; a ticket
mutated by every command still passes `skald check` and differs from the original only in the
intended lines.

---

## Journal

### Intake — "should we build this?"

- **Alignment:** required before the deny can land; without it the enforcement mechanism is a wall
  with no door.
- **AC sanity:** the honest risk is that these commands get built and then agents keep editing files
  directly until the deny forces them. That's expected — the deny is the forcing function, and the
  parent sequences it last on purpose.
- **Recommendation:** go, after `check`.
- **Human's decision:** pending.

### Build log

- 2026-08-20: Spec authored. Not started.

### Open questions

- Should `append` to a non-existent section create it, or fail? Leaning create — an agent shouldn't
  have to know whether the template happened to include "Open questions".

### Checkpoints (memory boundaries)

- **PR-up:** —
- **Merge:** —

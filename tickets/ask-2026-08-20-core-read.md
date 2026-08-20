---
title: Bootstrap, store resolution, and the read side
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

**Context.** `skald` is an opinionated CLI for a local ticket store whose job is to keep an AI agent's
context organized across sessions — not a Linear clone. Its eventual role is to be the *only*
read/write path to a ticket directory, because the enforcement mechanism is a blanket permission deny
on those directories. This slice builds the foundation and the read half.

### Bootstrap

- Rust binary named `skald`, structured like `rata`: `Cargo.toml`, `flake.nix`, `README.md`,
  `src/cli.rs` + focused modules.

### Store resolution

- The active store is `$SKALD_STORE`, an absolute path to a ticket directory.
- **If unset, fail** with a message naming the variable and how to set it. Never guess a default,
  never fall back to cwd, never search upward. Writing a ticket into the wrong store is a worse
  outcome than refusing to run.
- If set but not absolute, not a directory, or not readable, fail with the same clarity.
- **Exactly one active store per invocation.** No multi-store flag, no store list.
- A symlink is not a way out of the store, on the way in or in a listing.

### The contract is compiled in

skald owns the field set and the status vocabulary; there is **no `.schema.toml`** and no per-store
schema. An opinionated tool that declares nothing is just a validator.

```yaml
title:                  # required, non-empty
status:                 # required. refining | building | reviewing | done | cancelled
paused:                 # optional free text; present means parked
repos: []               # list
branch:                 # optional
link:                   # optional — upstream ticket/issue URL, if any
pr:                     # optional
parent:                 # optional ticket id
created:                # YYYY-MM-DD, skald-managed
updated:                # YYYY-MM-DD, skald-managed
```

- `status`, `paused`, `repos`, `pr`, and `parent` exist to be filtered on; `title` to be displayed;
  `created`/`updated` to sort by. `branch` and `link` are pointers rather than filters — they earn
  their place because a resuming agent needs them and prose is a bad home for a pointer.
- An unknown key is a violation, not an extension point.

### Ticket model

- A ticket is one markdown file: YAML frontmatter, then `##`-delimited sections. **No H1** — `title`
  is frontmatter, and a second copy in the body would drift from it.
- **The body is closed**, exactly like the field set: `## Acceptance criteria` (addressable as `ac`)
  and `## Log`, and nothing else at that level. Structure *inside* a section belongs to the author, so
  the acceptance criteria may carry as many `###` subsections as they like.
- Parse both halves into a structured value. **Round-tripping must preserve byte-for-byte anything
  not explicitly modified** — frontmatter comments, key order, blank lines, prose, alignment padding.
  This is the property everything else in the series depends on; an agent that appends a log line must
  not silently reformat the file.
- Sections are indexed at every heading level, are fence-aware, and every section has a distinct,
  non-empty address.

### Commands

```
skald show <id> [--section NAME] [--json]
skald list [--status S] [--repo R] [--parent ID] [--paused] [--json]
```

- `show` with no `--section` returns the whole ticket byte-for-byte, so a resuming agent orients in
  one call.
- `list` renders `ID · STATUS · TITLE · UPDATED`, with a `!` marker on a paused row because `status`
  alone no longer says a ticket is parked. `--json` is the complete data.
- `<id>` resolves with or without the `.md` extension.
- **Reads must work on a store that does not satisfy the contract.** Migration has to read a
  non-conforming ticket before it can fix one, and a broken ticket must never vanish from a listing.

**Explicitly out of scope:**

- All mutation (`new`, `set`, `append`, `set-section`, `log`) — the write-commands slice.
- Validation and `check` — the check slice.
- Packaging, `docs`, the `docket` alias — the packaging slice.
- Any tag or full-text query.

**Verification:** against this repo's own store, `skald list --status building` returns the expected
set and `skald show <id>` reproduces a file exactly. Against `dotfiles/agents/tickets`, which has not
been migrated, `show` is still byte-identical and `list` still lists — reads do not require a valid
contract. Unsetting `$SKALD_STORE` produces an actionable error and a non-zero exit.

## Log

- 2026-08-20: status refining → building. AC authored; repo had VCS and this ticket store only.
- 2026-08-20: Round-trip mechanism chosen — a ticket holds its source verbatim plus **byte spans**
  into it, and a mutation is a span replacement spliced in one pass. Byte preservation is a property
  of the representation rather than of careful re-serialization, so a later slice cannot accidentally
  reformat a file. The span-edit primitive ships here with tests but no mutating command, because the
  guarantee is only testable against a change.
- 2026-08-20: Sections index every ATX heading level, not just `##` — the sections agents address
  most are `###` in the old template, and a `##`-only index would make them unaddressable.
- 2026-08-20: A trailing `# comment` on a frontmatter line is not part of the value. Implemented
  YAML's actual rule — a `#` starts a comment only when whitespace precedes it — so `pr:` keeps a
  URL's `#fragment` while an annotated key keeps its annotation through a `set`.
- 2026-08-20: A store's `README.md` is not a ticket, alongside dotfiles and `_`-prefixed templates.
  Three named exclusions, not a heuristic: a *malformed* ticket stays a ticket so `check` can report
  it, because one that quietly vanished from `list` is the exact failure this tool exists to catch.
- 2026-08-20: First implementation verified — 46 tests, clean clippy, `nix build .#` produces a
  working binary, and all three original AC checks passed.
- 2026-08-20: Fresh-eyes review (subagent fed only the code, the AC, and the checklist). Fuzzed the
  round-trip over 200k hostile inputs: no panics, and rendering an unmodified ticket matched its
  source byte-for-byte every time. The representation held; every defect was in the layers above it.
  Three blocking, all fixed: a value `set` spliced a scalar *above* a block list's items and returned
  success; a nested fence marker closed the outer fence, so the real closing fence re-opened one and
  every later heading vanished; any `--section` name slugifying to nothing matched the first
  punctuation-only heading. Six should-fix, all fixed: duplicate frontmatter keys answered
  differently through `list` than through `--json`; duplicate section slugs made the second
  unreachable; one unreadable file aborted the whole listing; a symlink was followed out of the store;
  unterminated frontmatter was detected but never surfaced; 4-space-indented headings were indexed.
- 2026-08-20: Review lesson worth keeping — four of the nine findings were cases
  `rata/src/headings.rs` had already solved. This section parser was a re-derivation rather than a
  port, and lost those guards. **Port from rata, don't re-derive.**
- 2026-08-20: Committed locally in three changes; not pushed. `origin` is an empty repo with no
  default branch, so there is nothing to open a PR against. Ian's call: hold the push. `pr:` stays
  empty and the reason lives here, which is the discipline this series exists to enforce.
- 2026-08-20: **AC revised — deliberate scope change, agreed with Ian.** The store schema had never
  been reviewed before it was built. Settled: skald is opinionated, so the field set and status
  vocabulary are compiled in and `.schema.toml` is deleted; `title` moves to frontmatter and the H1
  goes away; `phase` becomes `status` over refining → building → reviewing → done plus `cancelled`,
  with `paused` as an orthogonal free-text reason so a pause never loses its phase; `source` + `spec`
  collapse to `link`; `projects` becomes `repos`; `id` and `mode` are dropped — `id` because the
  filename already is the identity, and a field whose only job is to agree with the filename can
  disagree with it. `parent` and `branch` added. Outstanding build work: delete `schema.rs` and the
  JSON schema, compile the contract in, rework the `list` filters into named flags, and teach `list`
  about `title`.

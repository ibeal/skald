---
id: ask-2026-08-20-core-read
source: plain-ask
spec: authored here
mode: direct
phase: apply
pr:
projects:
  - skald
created: 2026-08-20
updated: 2026-08-20
---

# ask-2026-08-20-core-read — Bootstrap, store resolution, and the read side

> Slice 1 of 4. Parent spec: `dotfiles/agents/tickets/ask-2026-08-19-skald-ticket-cli.md`.
> Everything else in this series depends on this one.

---

## Spec — acceptance criteria

**Context.** `skald` is a CLI for ticket stores. Its eventual job is to be the *only* read/write path
to a ticket directory, because the enforcement mechanism is a blanket permission deny on those
directories. This slice builds the foundation and the read half.

**Refined (after intake):**

### Bootstrap

- Rust binary named `skald`, structured like `rata`: `Cargo.toml`, `flake.nix`, `README.md`,
  `src/cli.rs` + focused modules, and a JSON schema for the store config.

### Store resolution

- The active store is `$SKALD_STORE`, an absolute path to a ticket directory.
- **If unset, fail** with a message naming the variable and how to set it. Never guess a default,
  never fall back to cwd, never search upward. Writing a ticket into the wrong store is a worse
  outcome than refusing to run.
- If set but not a directory, or not readable, fail with the same clarity.
- **Exactly one active store per invocation.** No multi-store flag, no store list — decided
  2026-08-20. Per-directory switching via direnv covers the need.

### The store schema

- Each store carries `.schema.toml` beside its tickets, declaring the frontmatter contract: valid
  keys, which are required, and the permitted values for each enum field.
- Stores are independent. The personal store (`spec`, `projects`) and the work store (`linear`,
  `services`) share no configuration and neither knows the other exists.
- A store with no `.schema.toml` is usable for reads but reports one clear error on any operation
  that needs validation — don't silently invent a schema.

### Ticket model

- A ticket is one markdown file: YAML frontmatter plus `##`-delimited sections.
- Parse both halves into a structured value. **Round-tripping must preserve byte-for-byte anything
  not explicitly modified** — comments in frontmatter, section ordering, blank lines, prose. This is
  the property everything else in the series depends on; an agent that appends a log line must not
  silently reformat the file.

### Commands

```
skald show <id> [--section NAME] [--json]
skald list [--phase P] [--project X] [--json]
```

- `show` with no `--section` returns the whole ticket, so a resuming agent can orient in one call.
- `list` filters on declared frontmatter keys only. `--json` for machine consumption; a compact
  aligned table otherwise.
- `<id>` resolves with or without the `.md` extension.

**Explicitly out of scope:**

- All mutation (`new`, `set`, `append`, `set-section`) — slice 3.
- Validation and `check` — slice 2.
- Packaging, `docs` subcommand, the `docket` alias — slice 4.
- Any tag or full-text query.

**Verification:** with `$SKALD_STORE` pointed at `dotfiles/agents/tickets`, `skald list --phase build`
returns the expected set and `skald show ask-2026-08-19-skald-ticket-cli` reproduces the file exactly;
unsetting the variable produces an actionable error.

---

## Journal

### Intake — "should we build this?"

- **Alignment:** the whole series is gated on this. Nothing else can be built or tested first.
- **AC sanity:** byte-preserving round-trip is the requirement most likely to be under-built and most
  expensive to retrofit. Calling it out here rather than discovering it in slice 3.
- **Recommendation:** go.
- **Human's decision:** go (2026-08-20). Ian directed the four slices be built 1 → 2 → 3 → 4, with
  `check` deliberately before the write commands.

### Build log

- 2026-08-20: Spec authored. Repo exists with VCS and this ticket store only.
- 2026-08-20: Round-trip mechanism chosen: a ticket holds its source verbatim plus **byte spans**
  into it (frontmatter block, per-entry lines and value ranges, per-heading bodies). Rendering
  returns the source; a mutation is a span replacement applied in one pass. Byte preservation is a
  property of the representation rather than of careful re-serialization, so slice 3 cannot
  accidentally reformat a file. The span-edit primitive ships here (with tests); no mutating
  commands do.
- 2026-08-20: Sections index **every** ATX heading level, not just `##`. Agents address `Build log`
  and `Intake`, which are `###` in the template; a `##`-only index would make them unaddressable.
  Fence-aware, so a `#` comment inside a code block is not a section.
- 2026-08-20: `.schema.toml` shape settled as `[keys.<name>]` tables with `type` / `required` /
  `values` / `allow_empty`. Key *order* is deliberately not in the schema — `new` scaffolds from the
  store's template (slice 3), so the template owns order and there is one source of truth for it.
- 2026-08-20: `list`'s default table is ID / PHASE / UPDATED / PROJECTS with `--json` as the complete
  data. Those column names are a store convention rather than a universal, so blank renders where a
  store doesn't declare the key instead of erroring.
- 2026-08-20: **A trailing `# comment` on a frontmatter line is not part of the value.** Discovered by
  running against `_TICKET_TEMPLATE.md`, which annotates every enum key (`mode: direct  # ENUM: direct
  | orchestrated`). Implemented YAML's actual rule — a `#` starts a comment only when whitespace
  precedes it — so `pr:` keeps a URL's `#fragment` while `mode:` keeps its annotation through a `set`.
  Without this, slice 2 would report every annotated template-shaped ticket as an enum violation, and
  slice 3's `set` would delete the annotation.
- 2026-08-20: A store's `README.md` is not a ticket, alongside dotfiles (`.schema.toml`) and
  `_`-prefixed templates. Three named exclusions, not a heuristic: a *malformed* ticket stays a ticket
  so `check` can report it, because one that quietly vanished from `list` is the exact failure this
  tool exists to catch. Found by running `list` against `dotfiles/agents/tickets`, which has a README.
- 2026-08-20: Verification done. `list --phase build` against `dotfiles/agents/tickets` returns the
  expected three; `show ask-2026-08-19-skald-ticket-cli | diff` against the file is byte-identical;
  unsetting `$SKALD_STORE` prints the actionable error and exits 1. 36 unit tests, no clippy warnings,
  and `nix build .#` produces a working binary.
- 2026-08-20: Running `list` over the real personal store also surfaced pre-existing store dirt for
  slice 2 to report: `phase: reviewed` (not in the enum) on
  `ask-2026-08-18-one-time-playlist-import`, and a `projects` value that is prose rather than a list
  (`` `dotfiles` — `modules/nixos/desktop-common.nix` ``) on `ask-2026-07-24-sddm-astronaut`. Noted
  here rather than fixed: this slice does not validate.

### Review findings — fresh eyes, 2026-08-20

A subagent fed only the code, the AC, and the checklist (never the build reasoning). It fuzzed the
round-trip over 200k hostile inputs: no panics, and `render() == source` byte-for-byte on every one.
The representation held; the defects were all in the lookup and edit layers above it.

**Blocking — all three fixed.**

1. `set_value_edit` corrupted any key whose value continued onto later lines. `value_span` covers the
   key line only, so setting `projects` spliced `projects: zzz` in *above* `  - a` and returned
   `true`. Now refuses a multi-line value (inline `[a, b]` is still replaceable, since it lives on the
   key line). This was the exact "silently corrupt a ticket" failure the round-trip property exists to
   prevent, in the primitive whose correctness this slice owns.
2. A `~~~` line inside a ```-fence closed it, so the real closing fence re-opened one and every
   heading afterwards vanished — unaddressable, and an append to the previous section landing at EOF.
   Fences now track *which* marker opened them. Build logs quote fenced markdown, so this is ordinary
   content.
3. Any `--section` name that slugified to nothing (`!!!`, `🎉`, `''`) matched the first
   punctuation-only heading. Empty slugs now get the address `section`, and a name that slugifies to
   nothing resolves to nothing.

**Should-fix — all six fixed.** Duplicate frontmatter keys answered `build` via `list` and `review`
via `--json` (first-wins now, matching what a `set` rewrites); duplicate section slugs made the second
section unreachable (now `notes-2`); one unreadable file aborted the whole `list`, losing every good
ticket — the inverse of the invariant the README states (now warns on stderr and continues); a symlink
in the store was followed out of it, a containment hole under the very threat model that motivates the
deny (refused, and no longer listed); `unterminated` frontmatter was detected but never surfaced, so
an agent read "no frontmatter" instead of "broken frontmatter" (now a stderr warning and a JSON flag);
4-space-indented `##` lines indexed as headings.

**Nits:** took the missing-vs-not-a-directory error split, a `debug_assert` on span char-boundaries,
and an ATX-only note in the README. Declined the char-width table alignment (ids and dates are ASCII)
and the `flake.nix` platform list (matches rata). The `flake.lock` the reviewer flagged as absent now
exists — `nix build` created it after the review started.

The most useful line in the report: findings 2, 3, 5, and 9 are all cases `rata/src/headings.rs`
already solved. The section parser was written as a simplified re-derivation rather than a port, and
lost four guards in the process. **Lesson for slice 3: port from rata, don't re-derive.**

Verified after the fixes: 46 tests, no clippy warnings, every reviewer reproduction now behaves, and
the three AC verification checks still pass.

### Open questions

- **Should `.schema.toml` be discovered anywhere other than the store root? No** (2026-08-20). Store
  root only. Upward search is the same class of guess as falling back to cwd for the store itself.

### Checkpoints (memory boundaries)

- **PR-up:** Not pushed. `origin` (`git@github.com:ibeal/skald.git`) is an empty repo — no commits, no
  default branch, so there is nothing to open a PR against. Ian's call (2026-08-20): commit locally
  and hold the push. `pr:` therefore stays empty, and this note is where the reason lives — which is
  the discipline the parent ticket exists to enforce.
  - Resume state: all of slice 1 is on the local jj change; `cargo test` (46) and `cargo clippy` are
    clean; `nix build .#` produces a working binary. Nothing is in flight.
- **Merge:** —

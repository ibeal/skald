---
id: ask-2026-08-20-packaging-docs
source: plain-ask
spec: authored here
mode: direct
phase: intake
pr:
projects:
  - skald
  - dotfiles
created: 2026-08-20
updated: 2026-08-20
---

# ask-2026-08-20-packaging-docs — Packaging, built-in docs, `docket` alias

> Slice 4 of 4. Parent spec: `dotfiles/agents/tickets/ask-2026-08-19-skald-ticket-cli.md`.
> Depends on slices 1–3.

---

## Spec — acceptance criteria

**Context.** `skald` is only useful once it's installed on the machines that need it and once agents
can discover how to drive it without a human pasting instructions. `rata` already solves both; mirror
it.

**Refined (after intake):**

### Packaging

- `flake.nix` exposing the package and a dev shell, wired into the dotfiles home-manager config so
  `skald` is on PATH.
- Ship **`docket` as a supported alias** — a second binary name (or installed shell alias) for anyone
  who doesn't want the Norse theming. `docket` was the runner-up: "a register of matters awaiting
  action" is fully self-explanatory. It lost as primary because it shares a four-character prefix with
  `docker`, so shell completion can't disambiguate until the fifth keystroke and the two misread for
  each other at a glance. **`skald` stays canonical** in all docs and agent instructions so there's
  one name in the corpus.
- Document setting `$SKALD_STORE` per store via direnv, with the personal and work stores as worked
  examples.

### Docs

- `skald docs` subcommand printing built-in guidance, as `rata docs` does — so an agent can discover
  the interface from inside a session without a file read.
- `README.md` for humans; `docs/agent-usage.md` as the copy an agent is pointed at.
- A JSON schema for `.schema.toml`, referenced by `#:schema`, so editor tooltips carry the contract
  at the point of authoring — the same trick `rata.toml` uses.
- Document the pre-commit hook invocation for `skald check`.

**Explicitly out of scope:**

- The `tools-mcp` wrapper. Deferred; revisit once the CLI has been in use.
- Publishing anywhere public.

**Verification:** `skald` and `docket` both resolve on a rebuilt machine; `skald docs` prints usable
guidance; a `.schema.toml` gets completions and hover text in the editor.

---

## Journal

### Intake — "should we build this?"

- **Alignment:** the discoverability half matters more than usual here. After the deny lands, an
  agent that can't figure out the interface has no fallback — it can't read the directory to work it
  out. `skald docs` is the recovery path.
- **AC sanity:** the `docket` alias is cheap now and awkward later (a second name introduced after
  docs exist means two names in the corpus). Doing it in the same slice as the docs keeps the "one
  canonical name" rule enforceable.
- **Recommendation:** go, last in the series.
- **Human's decision:** pending.

### Build log

- 2026-08-20: Spec authored. Not started.

### Open questions

- None.

### Checkpoints (memory boundaries)

- **PR-up:** —
- **Merge:** —

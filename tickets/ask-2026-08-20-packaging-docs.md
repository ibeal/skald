---
title: "Packaging, built-in docs, and the docket alias"
status: building
paused:
repos:
  - skald
  - dotfiles
branch:
link:
pr:
parent:
created: 2026-08-20
updated: 2026-08-20
---

## Acceptance criteria

**Context.** `skald` is only useful once it is installed on the machines that need it and once agents
can discover how to drive it without a human pasting instructions. `rata` already solves both; mirror
it. The discoverability half matters more than usual here: after the deny lands, an agent that cannot
work out the interface has no fallback, because it cannot read the directory to figure it out.

### Packaging

- `flake.nix` exposing the package and a dev shell, wired into the dotfiles home-manager config so
  `skald` is on PATH.
- Ship **`docket` as a supported alias** — a second binary name for anyone who does not want the Norse
  theming. `docket` was the runner-up: "a register of matters awaiting action" is fully
  self-explanatory. It lost as primary because it shares a four-character prefix with `docker`, so
  shell completion cannot disambiguate until the fifth keystroke and the two misread for each other at
  a glance. **`skald` stays canonical** in all docs and agent instructions so there is one name in the
  corpus.
- Document setting `$SKALD_STORE` per store via direnv, with the personal and work stores as worked
  examples.

### Docs

- `skald docs` printing built-in guidance, as `rata docs` does — so an agent can discover the
  interface from inside a session without a file read. This is the recovery path once the deny is in
  place, so it must cover the whole write side, the status vocabulary, the two structural rules (the
  AC freeze and the append-only log), the fact that the body is closed, and what to do when a write is
  refused. That last part matters most: with six commands and no escape hatch, an agent that hits a
  refusal and cannot work out the way through is stuck, and it cannot read the directory to find out.
- `README.md` for humans; `docs/agent-usage.md` as the copy an agent is pointed at.
- Document the pre-commit hook invocation for `skald check`.

**Explicitly out of scope:**

- **A JSON schema for the store config.** Deleted from this slice: the contract is compiled into
  skald, there is no `.schema.toml`, and so there is no config file for an editor to validate.
- The `tools-mcp` wrapper. Deferred; revisit once the CLI has been in use.
- Publishing anywhere public.

**Verification:** `skald` and `docket` both resolve on a rebuilt machine. `skald docs` is sufficient
on its own to drive a ticket end to end — check this by handing it to an agent with no other context
and having it create, refine, advance, and log a ticket.

## Log

- 2026-08-20: status refining → building. AC authored.
- 2026-08-20: AC revised alongside the schema decision. The JSON schema deliverable is gone with
  `.schema.toml` — nothing declarative is left to describe. `skald docs` grew in importance in the
  same move: with the contract compiled in and unreadable from the filesystem, the built-in docs are
  now the only way an agent can discover the field set and the two structural rules, so the
  verification became "an agent can drive a ticket from `docs` alone" rather than "docs print
  something usable".
- 2026-08-20: Built `skald docs` and `docs/agent-usage.md`, compiled in with `include_str!`.
  **`docs` deliberately resolves no store.** It is the recovery path for an agent that cannot work out
  the interface, and "`$SKALD_STORE` is not set" is one of the things it explains — so failing on that
  very condition would be self-defeating. It returns before store resolution rather than after.
- 2026-08-20: The guide is written for the reader's actual situation: it opens by saying the directory
  is denied and that `cat`/`Read`/`grep` will not work, because an agent's first instinct on a refusal
  is to route around the tool. It ends with a table mapping each refusal to the way through, and says
  plainly that a genuine gap is worth reporting to a human rather than working around — since there is
  no way around.
- 2026-08-20: `docket` ships as a symlink from `flake.nix`'s `postInstall` rather than a second
  `[[bin]]`, which would compile the same crate twice for no gain.
- 2026-08-20: cleared parent: the parent ticket lives in the dotfiles store, and parent is same-store by design since that is what --parent filters on. The relationship is stated in the AC context instead.
- 2026-08-20: Review found the guide false in two places -- it claimed new would refuse a decorated status when it did not, and miscounted the commands. Both fixed alongside the code. Added the refusal that will actually be most common during rollout, which is a write refused for a violation it would introduce, plus the two things the tool genuinely cannot do: fill a managed date, and delete or rename a ticket.

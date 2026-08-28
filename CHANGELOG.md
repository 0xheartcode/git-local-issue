# Changelog

All notable changes to `gli` (git-local-issue) are documented in this file.

The format is based on Keep a Changelog (https://keepachangelog.com/en/1.1.0/),
and this project adheres to Semantic Versioning (https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Work continues toward 0.2.0. Multi-machine sync and CRDT merge across actors are
the next focus (see ROADMAP.md).

A batch of single-user UX features landed. These are additive: no existing
command changes behaviour, and the on-disk Format-Version stays at `1`.

### Added

- `gli archive <id>` / `gli restore <id>`: soft-hide an issue and bring it back.
  Archiving is reversible and sync-safe (a new `set-archived` operation that
  folds as Last-Writer-Wins). `gli rm` is a visible alias for `archive`.
  `gli archive <id> --purge` permanently deletes the underlying ref
  (irreversible and not sync-safe).
- Richer `gli ls` filters, all combining with AND: repeatable `-l/--label`
  (AND), `--assignee`, `--priority`, `--creator` (alias `--author`),
  `-s/--search` (free-text over title, description, labels, and comments),
  `--state`, plus `--sort newest|oldest|title`. Archived issues are hidden by
  default: `--archived` shows only archived issues, `--all` shows both.
- Machine-readable output: `--format json` on `gli ls` and `--json` on
  `gli show`. The stable schema includes uuid, nonce, short, title, description,
  state, reason, fixed_by, assignee, priority, labels, archived, creator,
  comment_count, and comments.
- `$EDITOR` integration: `gli comment <id>` with no text argument opens
  `$EDITOR` (falling back to `$VISUAL`, then `vi`) to compose the comment.
- `gli config get|set|list`: per-repository defaults stored in git config under
  `gli.default.*`. Keys are priority, assignee, labels (comma-separated), and
  format. `create` and `ls` fall back to these when the matching flag is absent
  (an explicit flag always wins).
- `gli completions <bash|zsh|fish|powershell|elvish>` prints a shell completion
  script, and `gli man` prints the man page (roff).

### Fixed

- SIGPIPE is reset to its default on Unix, so `gli ls | head` and
  `gli man | head` no longer error or panic on a closed pipe.

## [0.1.0] - 2026-08-28

Initial release: the local, single-user core. Issues are stored natively in Git
under `refs/issues/<uuidv7>`, with no database and no server. Everything runs
offline.

### Added

- Command set for day-to-day issue tracking:
  - `gli init` to set up issue tracking in a repository.
  - `gli create "title"` with optional `-l label` (repeatable), `-a assignee`, `-p priority`, and `-d description`.
  - `gli ls` (alias `gli list`) with `--state open|closed`, `-l label`, and `--format short|full`.
  - `gli show <id>` to view a single issue and its history.
  - `gli comment <id> "text"` to add a comment.
  - `gli edit <id>` with `--title`, `--desc`, `--add-label`, `--remove-label`, and `-a` (assignee, cleared with `-a ""`).
  - `gli state <id> open|closed` with optional `--reason` and `--fixed-by <sha>`.
  - `gli status` for a quick summary of the tracker.
  - `gli fsck` for integrity checks.
  - `gli --version` and `gli --help` (help prints the version plus examples; every subcommand has its own rich `--help`).
- Operation-sourced storage model: each change is an immutable operation commit
  (create, comment, set-title, set-description, set-state, set-assignee,
  set-priority, add-label, remove-label). Every operation carries a stable
  op-id, a Lamport clock, and an actor slug. Current state is the fold of the
  operation log.
- CRDT-ready fold: scalar fields use a Last-Writer-Wins register, labels use an
  OR-Set, and comments use a grow-only, deduplicated log. This makes each issue
  ready for conflict-free merge once sync lands.
- UUIDv7 identity: the truth id is a UUIDv7 (the ref is `refs/issues/<uuid>`),
  while display uses actor-scoped numbers (like `alice-1`, `bob-1`) plus a short
  uuid prefix. Display numbers are drift-allowed, so they can differ per machine
  without changing the underlying id.
- Forward-compatible, versioned on-disk format so future versions can read today's
  data (see FORMAT.md).
- Backup guidance using Git bundles, so a tracker can be captured and restored
  with standard Git tooling.
- Integrity checks via `gli fsck`.
- A disposable, rebuildable cache under `.git/gli`. If anything looks off, delete
  the cache and it rebuilds on the next run.

This release is local and single-user only. Multi-machine sync and CRDT merge are
planned for later (see ROADMAP.md).

[Unreleased]: https://github.com/0xheartcode/git-local-issue/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/0xheartcode/git-local-issue/releases/tag/v0.1.0

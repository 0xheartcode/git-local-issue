# Changelog

All notable changes to `gli` (git-local-issue) are documented in this file.

The format is based on Keep a Changelog (https://keepachangelog.com/en/1.1.0/),
and this project adheres to Semantic Versioning (https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Work continues toward 0.2.0. Multi-machine sync and CRDT merge across actors are
the next focus (see ROADMAP.md).

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

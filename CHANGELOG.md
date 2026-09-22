# Changelog

All notable changes to `git-local-issue` (the `gli` command) are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Entries are kept on a single line each: cargo-dist injects this file into the GitHub Release body, where GitHub renders every newline as a hard break, so wrapped lines would show as ragged mid-sentence breaks.

## [Unreleased]

Work continues toward 0.2.0; multi-machine sync and CRDT merge across actors are the next focus (see ROADMAP.md). The features below are additive: no existing command changes behaviour, and the on-disk Format-Version stays at `1`.

### Added

- `gli edit <id> -p/--priority <value>`: change an issue's priority after creation (pass `""` to clear it). Previously priority could only be set at `create` time, though the `SetPriority` operation already existed. Found by dogfooding.
- `gli archive <id>` / `gli restore <id>`: soft-hide an issue and bring it back. Archiving is reversible and sync-safe (a new `set-archived` operation that folds as Last-Writer-Wins). `gli rm` is a visible alias for `archive`. `gli archive <id> --purge` permanently deletes the underlying ref (irreversible and not sync-safe).
- Richer `gli ls` filters, all combining with AND: repeatable `-l/--label` (AND), `--assignee`, `--priority`, `--creator` (alias `--author`), `-s/--search` (free-text over title, description, labels, and comments), `--state`, plus `--sort newest|oldest|title`. Archived issues are hidden by default: `--archived` shows only archived issues, `--all` shows both.
- Machine-readable output: `--format json` on `gli ls` and `--json` on `gli show`. The stable schema includes uuid, nonce, short, title, description, state, reason, fixed_by, assignee, assignees, priority, labels, fields, files, archived, creator, comment_count, and comments.
- `$EDITOR` integration: `gli comment <id>` with no text argument opens `$EDITOR` (falling back to `$VISUAL`, then `vi`) to compose the comment.
- `gli config get|set|list`: per-repository defaults stored in git config under `gli.default.*`. Keys are priority, assignee, labels (comma-separated), and format. `create` and `ls` fall back to these when the matching flag is absent (an explicit flag always wins).
- `gli completions <bash|zsh|fish|powershell|elvish>` prints a shell completion script, and `gli man` prints the man page (roff).
- `gli fsck --fix` quarantines refs that are not usable issues (empty chain, unparseable, or will not fold) by moving them to `refs/gli-quarantine/*`. It is non-destructive (the commits are preserved) and recovers a repository that a single bad ref had otherwise bricked.
- Multiple assignees. An issue's assignees are now an OR-Set (several allowed), matching GitHub, GitLab, and Gitea, where single-assignee is deprecated. New `gli edit <id> --add-assignee <who>` and `--remove-assignee <who>` (both repeatable). `-a/--assignee` on `edit` now sets the sole assignee (clears the set and sets one, `-a ""` clears all). Legacy single-assignee data still reads correctly, `gli ls --assignee X` matches any member of the set, and `show` and JSON expose an `assignees` array (a first-assignee `assignee` field is kept for compatibility).
- Generic custom fields: `gli field set|get|rm|list <id> [key] [value]`. Optional, free-form key/value metadata (milestone, type, severity, sprint, anything), one value per key with last write wins. `rm` (or an empty value) clears a key. Shown in `show`, `ls --format full`, and JSON (a `fields` array of [key, value] pairs).
- Related files with notes: `gli file add <id> <path> [-n note]`, `gli file note <id> <path> <note>` (edits the note), `gli file rm <id> <path>`, and `gli file list <id>`. File presence is an OR-Set of repo-relative paths, each with an optional last-writer-wins note. These are metadata pointers, not attachments. Shown in `show`, `ls --format full`, and JSON (a `files` array of {path, note}).
- `gli --version` (and the `--help` header) now include the git commit that built the binary, for example `gli 0.1.0 (169b1c5fbb 2026-08-28T12:23:23+00:00)`. The stamp uses the commit SHA and the commit timestamp (git `%cI`, not a wall-clock build time), so builds stay reproducible, and it falls back to the plain version when `.git` is absent. The build script also re-stamps when a new commit lands on the current branch, not only on a branch switch.

These metadata additions are optional and unenforced, remain additive, and keep the on-disk Format-Version at `1`.

### Fixed

- SIGPIPE is reset to its default on Unix, so `gli ls | head` and `gli man | head` no longer error or panic on a closed pipe.

## [0.1.0] - 2026-08-28

Initial release: the local, single-user core. Issues are stored natively in Git under `refs/issues/<uuidv7>`, with no database and no server. Everything runs offline.

### Added

- `gli init` to set up issue tracking in a repository.
- `gli create "title"` with optional `-l label` (repeatable), `-a assignee`, `-p priority`, and `-d description`.
- `gli ls` (alias `gli list`) with `--state open|closed`, `-l label`, and `--format short|full`.
- `gli show <id>` to view a single issue and its history.
- `gli comment <id> "text"` to add a comment.
- `gli edit <id>` with `--title`, `--desc`, `--add-label`, `--remove-label`, and `-a` (assignee, cleared with `-a ""`).
- `gli state <id> open|closed` with optional `--reason` and `--fixed-by <sha>`.
- `gli status` for a quick summary of the tracker, `gli fsck` for integrity checks.
- `gli --version` and `gli --help` (help prints the version plus examples; every subcommand has its own rich `--help`).
- Operation-sourced storage model: each change is an immutable operation commit (create, comment, set-title, set-description, set-state, set-assignee, set-priority, add-label, remove-label). Every operation carries a stable op-id, a Lamport clock, and an actor slug. Current state is the fold of the operation log.
- CRDT-ready fold: scalar fields use a Last-Writer-Wins register, labels use an OR-Set, and comments use a grow-only, deduplicated log. This makes each issue ready for conflict-free merge once sync lands.
- UUIDv7 identity: the truth id is a UUIDv7 (the ref is `refs/issues/<uuid>`), while display uses actor-scoped numbers (like `alice-1`, `bob-1`) plus a short uuid prefix. Display numbers are drift-allowed, so they can differ per machine without changing the underlying id.
- Forward-compatible, versioned on-disk format so future versions can read today's data (see FORMAT.md).
- Backup guidance using Git bundles, so a tracker can be captured and restored with standard Git tooling.
- A disposable, rebuildable cache under `.git/gli`. If anything looks off, delete the cache and it rebuilds on the next run.

This release is local and single-user only. Multi-machine sync and CRDT merge are planned for later (see ROADMAP.md).

[Unreleased]: https://github.com/0xheartcode/git-local-issue/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/0xheartcode/git-local-issue/releases/tag/v0.1.0

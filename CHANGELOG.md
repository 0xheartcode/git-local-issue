# Changelog

All notable changes to `git-local-issue` (the `gli` command) are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Entries are kept on a single line each: cargo-dist injects this file into the GitHub Release body, where GitHub renders every newline as a hard break, so wrapped lines would show as ragged mid-sentence breaks.

## [Unreleased]

Work continues toward 0.2.0: multi-machine sync (push/pull `refs/issues/*`) and CRDT merge across actors are the next focus (see ROADMAP.md).

## [0.1.0] - 2026-09-22

Initial release: the local, single-user core. Issues are stored natively in Git under `refs/issues/<uuidv7>`, with no database and no server. Everything runs offline. Sync and multi-actor merge are planned for later (see ROADMAP.md); this build is the local core, and the on-disk Format-Version is `1`.

### Added

- `gli init` to set up issue tracking in a repository.
- `gli create "title"` with optional `-l label` (repeatable), `-a assignee`, `-p priority`, and `-d description`.
- `gli ls` (alias `gli list`) with filters that all combine with AND: repeatable `-l/--label` (AND), `--assignee`, `--priority`, `--creator` (alias `--author`), `-s/--search` (free-text over title, description, labels, and comments), `--state open|closed`, `--sort newest|oldest|title`, and `--format short|full|json`. Archived issues are hidden by default; `--archived` shows only archived, `--all` shows both.
- `gli show <id>` to view a single issue and its history, with `--ops` (raw operation log) and `--json`.
- `gli comment <id> "text"` to add a comment; with no text it opens `$EDITOR` (falling back to `$VISUAL`, then `vi`).
- `gli comment edit <id> <number> [text]` and `gli comment rm <id> <number>`: edit or soft-delete a comment (referenced by its number in `gli show`). Editing supersedes the text via an append-only `edit-comment` op (the original stays in history; the comment is marked "(edited)"); `rm` appends a `hide-comment` tombstone that folds the comment to "[deleted]" while preserving its text. Both are sync-safe and never rewrite the op chain. JSON gains `edited` and `hidden` per comment.
- `gli comment purge <id> <number>`: hard-delete a comment by rewriting the issue's chain without that comment op (and any `edit-comment`/`hide-comment` ops referencing it, so no superseded copy of the text survives). This rewrites history: it is irreversible and not sync-safe, so it is an escape hatch for a leaked secret, not routine delete (prefer `rm`). It prints the `git gc` invocation to prune the now-unreachable objects.
- `gli edit <id>` with `--title`, `--desc`, `--add-label`, `--remove-label`, `-a/--assignee` (sets the sole assignee; `-a ""` clears all), `--add-assignee`/`--remove-assignee` (repeatable), and `-p/--priority` (`""` clears). Each field change is its own operation, reported in order.
- `gli state <id> open|closed` with optional `--reason` and `--fixed-by <sha>`; `gli close` / `gli reopen` shorthands.
- `gli archive <id>` / `gli restore <id>`: soft-hide an issue and bring it back (a reversible, sync-safe `set-archived` operation folding as Last-Writer-Wins). `gli rm` is an alias for `archive`; `gli archive <id> --purge` permanently deletes the ref (irreversible, not sync-safe).
- Multiple assignees: an issue's assignees are an OR-Set (several allowed), matching GitHub/GitLab/Gitea. `gli ls --assignee X` matches any member; `show` and JSON expose an `assignees` array (a first-assignee `assignee` field is kept for compatibility).
- Generic custom fields: `gli field set|get|rm|list <id> [key] [value]` — free-form key/value metadata (milestone, type, severity, sprint, anything), one value per key with last write wins (`rm` or an empty value clears). Shown in `show`, `ls --format full`, and JSON.
- Related files with notes: `gli file add|note|rm|list <id> <path>` — an OR-Set of repo-relative paths, each with an optional last-writer-wins note. Metadata pointers, not attachments. Shown in `show`, `ls --format full`, and JSON.
- `gli config get|set|list`: per-repository defaults stored in git config under `gli.default.*` (priority, assignee, labels, format); `create` and `ls` fall back to them unless a flag overrides.
- `gli status` for a quick summary of the tracker, and `gli fsck` for integrity checks. `gli fsck --fix` quarantines refs that are not usable issues (empty chain, unparseable, or will not fold) to `refs/gli-quarantine/*` — non-destructive (commits preserved) and recovers a repository that a single bad ref had otherwise bricked.
- `gli completions <bash|zsh|fish|powershell|elvish>` prints a shell completion script, and `gli man` prints the man page (roff).
- Machine-readable JSON: `--format json` on `gli ls` and `--json` on `gli show`, over a stable schema (uuid, handle, number, nonce, short, title, description, state, reason, fixed_by, assignee, assignees, priority, labels, fields, files, archived, creator, comment_count, comments).
- Operation-sourced storage model: each change is an immutable operation commit, each carrying a stable op-id, a Lamport clock, and an actor slug; current state is the fold of the operation log. The CRDT-ready fold uses a Last-Writer-Wins register for scalar fields, an OR-Set for labels and assignees, and a grow-only deduplicated log for comments, so each issue is ready for conflict-free merge once sync lands.
- UUIDv7 identity: the truth id is a UUIDv7 (the ref is `refs/issues/<uuid>`), and a short prefix is always a permanent handle. Display is a number shown as `#1`, `#2`, becoming the actor-qualified `alice-#1` only when two actors share a number (after a sync); type the bare number (`gli show 1`, since a shell eats a leading `#`), `alice-1`, or a uuid prefix. Display numbers are drift-allowed; the UUIDv7 is the permanent id.
- `gli --version` (and the `--help` header) include the git commit that built the binary, e.g. `gli 0.1.0 (169b1c5fbb 2026-09-22T12:23:23+00:00)`. The stamp uses the commit SHA and commit timestamp (git `%cI`, not a wall-clock build time), so builds stay reproducible, and re-stamps when a new commit lands on the current branch. It falls back to the plain version when `.git` is absent.
- Forward-compatible, versioned on-disk format so future versions can read today's data (see FORMAT.md); a disposable, rebuildable cache under `.git/gli`; and backup guidance using Git bundles.

### Fixed

- SIGPIPE is reset to its default on Unix, so `gli ls | head` and `gli man | head` no longer error or panic on a closed pipe.

[Unreleased]: https://github.com/0xheartcode/git-local-issue/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/0xheartcode/git-local-issue/releases/tag/v0.1.0

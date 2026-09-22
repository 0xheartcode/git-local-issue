# git-local-issue (`gli`)

A distributed, offline-first issue tracker that stores issues natively in Git.

> **Inspired by [git-native-issue](https://github.com/remenoscodes/git-native-issue) (GPL-v2).**
> `git-local-issue` is a from-scratch Rust project with its own on-disk format.
> Thank you to everyone who is frustrated with git issues not being a native feature.

## What it is

`git-local-issue` (command `gli`) keeps your issues where your code already
lives: inside Git. Issues are stored natively under `refs/issues/*`, with no
external database and no SaaS backend. It is distributed and offline-first, so
you can create, comment on, and close issues with no network, and you sync by
ordinary `git push` and `git fetch`, exactly like you sync code.

Because everything is just Git refs, the governance model is not hardcoded.
Centralized, satellite-mirror, and fully decentralized workflows all emerge for
free, the same way Git itself handles code.

## Status

**v0.0.0 (v0.1 in development): LOCAL CORE ONLY.**

There is no platform sync and there are no bridges yet. Sync with GitHub,
GitLab, or Gitea is explicitly deferred to a later version. Nothing in the
on-disk format is frozen until v0.1 ships.

## Commands (v1 local core)

```
gli init                       # set up refs/issues in the current repo
gli create "title" [-l label] [-a assignee] [-p priority] [-d desc]
gli ls [--state ...] [-l label ...] [--assignee ...] [--priority ...] [--creator ...] [-s search] [--sort newest|oldest|title] [--archived|--all] [--format short|full|json]   # alias: gli list
gli show <id> [--json]         # id = actor-nonce OR uuidv7 prefix
gli comment <id> ["text"]      # no text opens $EDITOR ($VISUAL, then vi)
gli edit <id> [--title ...] [--desc ...] [--add-label ...] [--remove-label ...] [-a ...] [--add-assignee ...] [--remove-assignee ...]
gli field set|get|rm|list <id> [key] [value]   # optional free-form key/value metadata
gli file add|note|rm|list <id> <path> [-n note]   # optional repo-relative file pointers with notes
gli state <id> open|closed [--reason ...] [--fixed-by <sha>]
gli archive <id> [--purge]     # soft-hide an issue (alias: gli rm); --purge deletes the ref
gli restore <id>               # un-archive a soft-hidden issue
gli config get|set|list        # per-repo defaults under gli.default.* (priority, assignee, labels, format)
gli status                     # renumber notices, integrity, and unpushed/local-only issues
gli fsck [--fix]               # validate integrity; --fix quarantines unusable refs
gli completions <bash|zsh|fish|powershell|elvish>   # print a shell completion script
gli man                        # print the man page (roff)
gli --version / --help         # --help also prints the version
```

`edit` supports editing the description/body (`--desc`), not just the title.

Assignees are multi-valued: an issue can have several. `--add-assignee` and
`--remove-assignee` (both repeatable) adjust the set, while `-a/--assignee` sets
the sole assignee (`-a ""` clears all). `gli ls --assignee X` matches any member
of the set.

`gli field ...` (custom fields) and `gli file ...` (related-file pointers with
notes) are optional metadata: free-form and unenforced. Both appear in `show`,
`ls --format full`, and the JSON output.

`ls` filters all combine with AND: repeatable `-l/--label` (AND), `--assignee`,
`--priority`, `--creator` (alias `--author`), `-s/--search` (free text over
title, description, labels, and comments), `--state`, and `--sort`. Archived
issues are hidden by default: `--archived` shows only archived issues and
`--all` shows both. `--format json` (on `ls`) and `--json` (on `show`) emit a
stable machine-readable schema.

`create` and `ls` fall back to the `gli config` defaults when the matching flag
is absent; an explicit flag always wins.

`<id>` resolution accepts an actor-nonce (for example `alice-4`) or a UUIDv7
prefix (for example `018f2a1c`). Prefixes expand unambiguously, and `gli`
prompts on ambiguity.

## Design

`gli` uses an **operation-sourcing** model. Each change is an immutable
operation (`Create`, `AddComment`, `SetTitle`, `SetState`, `SetAssignee`,
`AddLabel`, `RemoveLabel`) recorded as a commit in the issue's chain. The root
commit is the `Create` operation. Current state is never stored as truth: it is
computed by folding the operation log.

Every operation, and comments especially, carries a **stable, explicit
operation ID** from the first commit. This identity is not derived from
position or content, which is what keeps comments from duplicating on sync.

Every operation also carries a **Lamport clock** (a logical clock, not
wall-clock time), so the model is **CRDT-ready** from the start:

- Comments are a **grow-only log**.
- Title, state, and assignee are an **LWW-Register** (last write wins, ordered
  by Lamport clock).
- Labels and assignees are each an **OR-Set** (observed-remove set), so an issue
  can carry several assignees.
- Custom fields and related-file notes are **LWW-Registers** per key/path, added
  or removed via observed-remove membership.
- Archived is an **LWW-Register**: `gli archive` records a `set-archived`
  operation, so hiding an issue is a soft, reversible tombstone (undone by
  `gli restore`) that merges cleanly. Only `gli archive --purge`, which deletes
  the underlying ref, is irreversible and not sync-safe.

Per-repository defaults live in git config under `gli.default.*` (priority,
assignee, labels, format) and fill in for `create` and `ls` when the matching
flag is omitted, so an explicit flag always wins.

The CRDT merge algorithm itself lands with sync in a later version, but the
on-disk model is designed to be CRDT-correct now.

Following the lesson from surveying the GitHub, GitLab, and Gitea issue APIs,
`gli` keeps a small stable core (title, state, labels, assignees) and puts
everything flexible into one open, extensible layer (generic custom fields)
rather than a sprawl of one-off fields. Multiple assignees is the one genuinely
multi-valued core field all three platforms share.

Identity has two layers. The **truth** is a **UUIDv7** per issue: the ref is
`refs/issues/<uuidv7>`, which never collides and is time-sortable. The
**display** identity is an **actor-scoped nonce** (`alice-1`, `bob-1`, and so
on), namespaced per actor so cross-actor numbers never clash. A short UUIDv7
prefix is always available as a permanent handle. Display numbers are
drift-allowed: they are a convenience, not a permanent handle, and a friendly
number may change after a sync. `gli status` surfaces any renumbering as an
informational notice.

Metadata is stored as Git trailers (`State:`, `Labels:`, `Assignee:`,
`Priority:`, `Op:`, `Lamport:`, `Actor:`, `Format-Version:`). A format version
is stamped on every issue, and the parser ignores unknown trailers and never
rejects a higher version, for forward and backward compatibility.

## Cache and backup

`gli` keeps a **local query cache** in `.git/gli/`. This cache is **disposable
and rebuildable**: it is never synced and is never the source of truth. If
anything looks off, delete the cache and it rebuilds by folding the operation
log again.

To back up your issues, bundle the refs:

```
git bundle create issues.bundle --glob='refs/issues/*'
```

## License

GPL-v2. See [LICENSE](LICENSE). 

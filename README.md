# git-local-issue (`gli`)

A distributed, offline-first issue tracker that stores issues natively in Git.

> **Forked from / inspired by [git-native-issue](https://github.com/) (GPL-v2).**
> `git-local-issue` is a from-scratch Rust rewrite with its own on-disk format.
> Credit and thanks to the original `git-native-issue` project, whose design and
> hard-won lessons made this fork possible.

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
gli ls [--state open|closed] [-l label] [--format short|full]   # alias: gli list
gli show <id>                  # id = actor-nonce OR uuidv7 prefix
gli comment <id> "text"
gli edit <id> [--title ...] [--desc ...] [--add-label ...] [--remove-label ...] [-a ...]
gli state <id> open|closed [--reason ...] [--fixed-by <sha>]
gli status                     # renumber notices, integrity, and unpushed/local-only issues
gli fsck                       # validate issue data integrity
gli --version / --help         # --help also prints the version
```

`edit` supports editing the description/body (`--desc`), not just the title.

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
- Labels are an **OR-Set** (observed-remove set).

The CRDT merge algorithm itself lands with sync in a later version, but the
on-disk model is designed to be CRDT-correct now.

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

GPL-v2. See [LICENSE](LICENSE). This fork keeps the original license and
copyright notices from `git-native-issue`.

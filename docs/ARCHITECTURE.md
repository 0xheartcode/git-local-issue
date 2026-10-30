# ADR-001: Issues as Git-native operation logs

Status: accepted (0.1.0)

## Context

`gli` is an issue tracker that must live inside the repository, work with no
server and no network, travel with the code through ordinary `git` clones, and
eventually let two clones exchange and merge issues without a central authority.
Two questions force the design:

1. Where do issues live, physically, so they ship with the repo but do not
   pollute the working tree or its file history?
2. How is an issue's state represented so that two people editing the same issue
   offline can merge deterministically, with no "please resolve this conflict"?

## Decision

Each issue is a Git ref, `refs/issues/<uuidv7>`, pointing at a chain of commits.
Every commit is one immutable **operation** (`Create`, `SetTitle`, `Comment`,
`AddLabel`, `SetState`, and so on), serialized as Git trailers in the commit
message (see `FORMAT.md`). The issue's current state is the deterministic
**fold** of its operation log, with conflict-free merge semantics baked into each
field: a Last-Writer-Wins register for scalars (title, state, priority), an
OR-Set for labels and assignees, and a grow-only, op-id-deduplicated log for
comments.

The truth identity is a UUIDv7 (time-sortable, globally unique), so the ref name
never collides and never needs coordination. Human-facing display numbers (`#N`)
are computed on read and may drift; the UUIDv7 is the permanent handle.

## Why not a database

A database is a server (or at least an out-of-tree file) that does not travel
with `git`, does not replicate over `git push`, and reintroduces the very
infrastructure `gli` exists to avoid.

## Why not a tracked file in the working tree

A single `issues.json` (or a file per issue) committed to the tree conflicts on
nearly every concurrent edit, which is exactly the merge pain the operation log
removes. It also mixes issue history into the code's own diffs and blames. Issues
belong in their own ref namespace, beside the code, not inside it.

## Why operations folded as a CRDT, rather than stored state

Storing the computed state (title X, state closed) forces a merge policy the
moment two clones diverge. Storing the operations instead makes merge a fold:
concurrent op chains combine by the field semantics above and converge regardless
of order or partition. The model is built for this from day one even though the
merge algorithm itself ships later, so the on-disk format does not have to change
when sync arrives.

## Why reuse Git's object store

Content-addressed storage, replication over `push`/`fetch`, and a path to signed
authorship all come for free. Network transport stays on the `git` CLI
permanently, a deliberate choice recorded in `ROADMAP.md`.

## Consequences

- Offline-first and server-less: issues live in the repo and move with it.
- Every change is an immutable, authored, timestamped commit: a full audit log.
- Ready for conflict-free merge without a format change when sync lands.
- A read folds the whole op log each run. A disposable in-memory cache covers
  this today; a persistent on-disk cache is a planned drop-in behind the same
  fold (see `ROADMAP.md`).
- Display numbers are computed and may drift after a sync. The UUIDv7 is the
  stable id; a short prefix of it is always a valid handle.
- Comments stay inline in the issue's own op chain rather than in separate refs,
  keeping reads, writes, and merges simple (recorded in the comment-lifecycle
  design note).

## Shipped since v0.1

- The local, single-user core: create, list, show, edit, state, comment,
  archive, fsck, status.
- An optional metadata layer (multiple assignees, generic custom fields, related
  files with notes), added additively with `Format-Version` unchanged.
- `#N` display numbers, with an optional project prefix, and comment
  edit/remove/purge.

Sync (exchanging `refs/issues/*`) and CRDT merge across diverged clones are the
next milestones; see `ROADMAP.md`.

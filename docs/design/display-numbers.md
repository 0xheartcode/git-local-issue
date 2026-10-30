# Display numbers

Status: accepted (0.1.1)

Builds on `ARCHITECTURE.md` (issues as Git-native operation logs).

## Context

The truth identity of an issue is its UUIDv7, but nobody wants to type
`018f0000...` to refer to issue "the login bug". We need a short, human handle.
The hard constraint is that `gli` is offline-first and coordination-free: two
clones can create issues while disconnected, so there is no central counter to
hand out a repository-wide `#1, #2, #3` the way a server-backed tracker does.
Two offline clones would both mint `#4` and collide on sync with no deterministic
winner.

## Decision

Display numbers are **per creating actor**, computed on read. Each actor's own
issues are sorted by UUIDv7 (time order) and numbered `1..N`. This needs no
coordination: `alice` mints `alice-1, alice-2` and `bob` mints `bob-1, bob-2`
independently, and they never clash because the actor scopes the number.

The **shown handle** is the smallest form that is unambiguous:

- `#N` while the number `N` is unique across all issues in the repository. In a
  single-actor repository (the common case, and every solo repository) this is
  every issue, so the display is a clean `#1, #2, #3`, and no actor name ever
  appears.
- `alice-#N` once two actors share the number `N`. This can only happen after a
  sync pulls in another actor's issues. The actor is the only thing that
  disambiguates two issues that both want `#1` within one repository.

A same-actor clash (the same actor minted `#1` from two offline clones) resolves
deterministically: the earlier UUIDv7 keeps the lower number, the later drifts to
the next free slot.

## Project prefix

An optional project prefix (`gli config set display-prefix <name>`, or `auto` to
derive a slug from the repository directory or its remote) scopes the
unambiguous form as `myproject-#N` instead of a bare `#N`. It is off by default,
so existing behavior is unchanged. It is a namespace prefix, not a
disambiguator: within one repository the prefix is constant, so it cannot tell
two colliding actors apart. On a real cross-actor collision the handle therefore
still falls back to the actor form (`alice-#N`), not the prefix.

## Resolution

A handle given on the command line resolves in this order: a bare number or
`#N`, then the actor-qualified `alice-1` / `alice-#1`, then the shown handle
directly (including a prefixed `myproject-#1`), then a full UUID, then a UUID
hex prefix. A bare `#` is optional everywhere because a shell treats a leading
`#` as a comment. When a bare number matches several actors, resolution reports
the ambiguous candidates rather than guessing.

## Drift

Display numbers are a convenience, not a permanent identifier. They are
recomputed every run, so they can change after a sync renumbers around a
collision. The UUIDv7 never changes, and a short prefix of it is always a valid,
stable handle.

## Deferred (not in 0.1.1)

- Stable, never-drifting numbers. These need the coordination point that only
  the merge step provides, so they are planned alongside CRDT merge rather than
  here (see `ROADMAP.md`).
- Cross-repository references (`otherproject-#5`), which extend the prefix into a
  reference syntax plus a known-repository registry.

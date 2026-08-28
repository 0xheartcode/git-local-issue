# gli on-disk format specification

- Format version: **1**
- Status: draft, stabilizing toward v0.1
- Scope: how `gli` stores issues natively in Git. This is a first-class,
  versioned deliverable: readers and writers commit to the compatibility rules
  in the "Versioning and compatibility" section below.

`gli` stores every issue as a chain of Git commits under a dedicated ref
namespace. There is no external database and no separate state file. The
current state of an issue is never stored: it is computed by folding the
operation log with the CRDT rules in this document.

## 1. Ref namespace

- Each issue is one ref: `refs/issues/<uuid>`.
- `<uuid>` is a UUIDv7 rendered as 32 lowercase hex characters, no dashes
  (for example `018f2a1c4cf17003a64f16d7793d2004`).
- The ref points at the latest operation commit (the chain tip).
- UUIDv7 is the permanent, never-colliding truth identity. It is time-sortable,
  so a lexicographic sort of the hex is a chronological sort.

Nothing else lives under `refs/issues/*`. Backing up an entire tracker is:

```
git bundle create issues.bundle --glob='refs/issues/*'
```

## 2. The operation chain

- Each issue is a linear chain of commits. Each commit is exactly one immutable
  **operation**.
- The root commit (no parent) is always the `create` operation.
- Every later operation is a commit whose single parent is the previous
  operation. v1 writes strictly linear chains. The model is designed so that
  concurrent branches can be merged by CRDT rules when sync lands (v2); readers
  MUST already tolerate a chain that is not strictly linear (see fsck notes).
- Commits normally use the **empty tree**. Attachment operations MAY use a
  non-empty tree carrying blob(s); readers MUST NOT assume the empty tree.

The commit author/committer identity is ordinary Git identity. It is
convenient provenance but is NOT trusted: authorship is forgeable, and access
control is expected to live at the Git transport/hosting layer. A seam for
commit signing (GPG/SSH) is reserved for later.

## 3. Commit message encoding

A commit message has three parts, in order:

```
<subject>
<blank line>
<body>            (optional, free-form, may be multiple paragraphs)
<blank line>
<trailer block>   (the last paragraph; every line is a trailer)
```

- The **subject** is a human summary (for example `Create: Fix the parser`). It
  is not parsed for meaning; the trailers are authoritative.
- The **body** is free-form UTF-8 and carries multi-line payloads: the issue
  description (for `create` and `set-description`) and the comment text (for
  `comment`).
- The **trailer block** is the final paragraph of the message, following Git's
  own trailer convention: each line is `Key: value`. A key is a run of ASCII
  letters, digits and hyphens beginning with a letter.

### Parsing rule (important)

The trailer block is the last paragraph **only if every line in it is a
trailer**. A trailer-shaped line inside the body (for example a line beginning
`Note:`) is never mistaken for metadata, because the real trailer block is
always emitted as a separate final paragraph after the body. If the whole
message is a single paragraph, it is treated as subject/body with no trailers.

## 4. Trailers

### Standard metadata (present on every operation)

| Trailer | Meaning |
|---|---|
| `Op` | Operation type slug (see section 5). |
| `Op-Id` | Stable operation id: a UUIDv7 hex string minted when the op is created. **Never** derived from position or content. This is the identity used to dedupe operations (especially comments) across syncs. |
| `Lamport` | Logical (Lamport) clock as a non-negative integer. Not wall-clock. |
| `Actor` | Actor slug that authored the op. Namespaces display numbers. |
| `Format-Version` | The format version this op was written against (currently `1`). |

### Operation-specific metadata

| Trailer | Used by | Meaning |
|---|---|---|
| `Title` | `create`, `set-title` | Single-line issue title. |
| `Labels` | `create` | Comma-separated initial labels. |
| `Label` | `add-label`, `remove-label` | A single label element. |
| `Assignee` | `create`, `set-assignee` | Assignee; an empty value means "unassigned" (explicit clear). |
| `Priority` | `create`, `set-priority` | Priority; empty value clears. |
| `State` | `set-state` | `open` or `closed`. |
| `Reason` | `set-state` | Optional free-text reason. |
| `Fixed-By` | `set-state` | Optional commit sha that fixed the issue. |

Custom/experimental trailers SHOULD be prefixed `X-` (for example
`X-Reactions`). Any unknown trailer MUST be ignored, never rejected.

## 5. Operations

| `Op` slug | Payload | CRDT role |
|---|---|---|
| `create` | title (trailer), description (body), optional labels/assignee/priority, opens the issue | seeds all registers; adds initial labels to the OR-Set |
| `comment` | text (body) | grow-only log entry, keyed by `Op-Id` |
| `set-title` | title (trailer) | LWW-Register write |
| `set-description` | description (body) | LWW-Register write |
| `set-state` | state + optional reason/fixed-by (trailers) | LWW-Register write |
| `set-assignee` | assignee (trailer, empty = clear) | LWW-Register write |
| `set-priority` | priority (trailer, empty = clear) | LWW-Register write |
| `add-label` | label (trailer) | OR-Set add; the add-tag is the op's `Op-Id` |
| `remove-label` | label (trailer) | OR-Set remove |

## 6. Folding to current state (CRDT rules)

State is computed by applying operations in causal order, defined as ascending
`(Lamport, Op-Id)`. On a single linear chain this equals commit order; the
tie-break by `Op-Id` keeps the result deterministic across clones once
concurrent histories merge.

- **Title, description, state, assignee, priority: LWW-Register.** The write
  with the greatest `(Lamport, Op-Id)` wins. The `Op-Id` tie-break means two
  writers who happen to share a Lamport value still converge to the same value.
- **Labels: OR-Set (observed-remove).** Each `add-label` contributes a unique
  add-tag (its `Op-Id`) for that element. An element is present while it has at
  least one live add-tag. A `remove-label` removes the add-tags it has observed.
  Re-adding after a remove makes the element present again with a fresh tag.
  (v1 folds a single linear chain, so a remove clears all currently-live tags
  for the element; concurrent-merge tombstone handling arrives with sync in v2.
  The per-add tags already exist, so that is a drop-in, not a format change.)
- **Comments: grow-only log.** Every `comment` op is one entry, identified by
  its `Op-Id`. Duplicates (same `Op-Id` seen twice, as a naive sync might
  produce) collapse to one. Display order is `(Lamport, Op-Id)`.

## 7. Display identity (derived, drift-allowed)

Display numbers are computed on read and never stored as truth.

- For each creating actor, that actor's issues are sorted by UUIDv7 and numbered
  `1..N`, giving actor-scoped nonces like `alice-1`, `bob-1`. Cross-actor
  numbers never clash.
- The only way two issues collide on the same nonce is if the same actor minted
  it from two offline clones. This is cosmetic (the UUIDv7 disambiguates) and is
  resolved deterministically: the earlier UUIDv7 keeps the number, the later
  drifts to the next free slot.
- A short UUIDv7 prefix (8 hex chars by convention) is always available as a
  permanent handle. Because the first hex characters of a UUIDv7 are its
  timestamp, two issues created in the same millisecond can share a short
  prefix; tools resolve prefixes and prompt on ambiguity.

## 8. The cache

Any local index/cache (for example under `.git/gli/`) is disposable and
rebuildable, never the source of truth, and never synced. The guarantee:
**delete the cache and it rebuilds** from the op log. v1 folds in memory on each
run; a persistent on-disk cache is a later drop-in behind the same fold.

## 9. Versioning and compatibility

- Every operation stamps `Format-Version` from commit #1.
- A reader MUST NOT reject an operation solely because its `Format-Version` is
  higher than the reader knows: parse what you understand, ignore the rest.
- A reader MUST ignore unknown trailers.
- Additive changes (new optional trailers, new `X-` trailers, new operation
  types a reader can skip) do NOT bump the format version.
- The format version bumps only for a change that would make an older reader
  compute wrong state from data it appears to understand. Such a change will be
  documented here with its migration notes.

## 10. Reserved for future versions (design space, not yet built)

These are anticipated so the format can grow additively (no version bump) when
they land. They are documented now so a 0.1.0 reader already tolerates them.

- **Attachments.** New operation types carrying inline blobs (size-guarded) or
  URL references, using a non-empty commit tree. Readers already must not assume
  the empty tree (section 2).
- **Issue relations.** `relate` / `unrelate` operations linking two issue uuids
  with a relation kind (`blocks`, `relates-to`, `duplicate-of`). These will fold
  as an OR-Set of typed edges. Unknown-to-an-old-reader, so safely skipped.
- **Commit signing.** GPG/SSH signatures on operation commits for verifiable
  authorship. This rides on git's own commit signing and adds no trailer, so it
  is invisible to the fold.
- **Reactions and stable (non-drifting) display numbers.** Optional, additive.

Because all of the above are either new operation types (skippable) or new
optional trailers (ignorable) or git-native features, none of them require a
`Format-Version` bump. Format version stays at 1 until a genuinely breaking
change is unavoidable.

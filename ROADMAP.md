# gli roadmap

This maps everything planned for `gli`, grouped by release. The guiding rule:
the on-disk **format** is the only thing that is expensive to change once people
store real issues, so we freeze it carefully at 0.1.0 and add features additively
in later minor versions. Pre-1.0, minor versions carry features and may change
behaviour; patch versions are fixes. The `Format-Version` in `FORMAT.md` is a
separate number from the crate version and bumps only on a breaking on-disk
change (currently `1`).

Status legend: `[ ]` planned, `[~]` in progress, `[x]` done. Size: S/M/L.

---

## 0.1.0 - Usable single-user tool (current target)

Scope decision: 0.1.0 is a flawless **local, single-user** issue tracker. No
sync, no multi-user merge. Everything here is either part of what makes a first
release a real release, or is expensive to change after the format goes live.

### Correctness and robustness
- [x] **A1. Friendly concurrent-write handling** (S). `append` retries on a lost
  compare-and-swap (re-reading the tip) and reports a clean `Conflict` instead of
  a raw git error.
- [x] **A2. Document edit semantics** (S). `edit` reports each field change as it
  lands, in order, so partial application is visible.
- [x] **A3. Input size guards** (S). Title/description/comment are refused above
  ~1MB (`MAX_FIELD_BYTES`) until attachments are wired.
- [x] **A5. Actor-slug collision surfacing** (S). `status` warns when two git
  identities slug to the same actor and share a nonce namespace.

### CLI ergonomics
- [x] **B7. `close` / `reopen` aliases** (S). Sugar over `state <id> closed|open`.
- [x] **B10. `show --ops`** (S). Prints the raw operation log of an issue.

### Format freeze review
- [x] **FR. Format audit before tagging** (S). `FORMAT.md` trailers verified
  against the code; a "reserved for future versions" section now locks how
  attachments/relations/signing arrive additively, keeping `Format-Version` at 1.

### Release infrastructure
- [x] **F1. `CHANGELOG.md`** (S). Keep a Changelog format, seeded with 0.1.0.
- [x] **F2. `dist-workspace.toml`** (S). cargo-dist 0.32.0 config (matches the
  panekit setup: shell + powershell installers, macOS/Linux/Windows targets).
- [x] **F3. CI + release workflows** (S). `.github/workflows/ci.yml` (gate) and
  `release.yml` (cargo-dist, builds/publishes binaries on tag).
- [x] **D20. Coverage reporting** (S). `make coverage` via `cargo-llvm-cov`,
  reported in CI (currently ~82% line, core modules 95%+).

### Docs
- [x] **E26. `USAGE.md`** (S). A hands-on zero-to-productive tutorial.

### Going live (outward-facing, needs explicit go-ahead)
- [ ] **F5 / E24 / E25. Publish** (S). Create the GitHub repo, push, tag
  `v0.1.0`, cut the release with binaries. Held until you say go.

---

## 0.2.0 - Richer single-user UX

Purely additive command surface. Breaks nobody.

Note: most of these landed early, in the same release line as 0.1.0, since
nothing has been published yet.

### Optional metadata layer (done)

An open, extensible metadata layer over the stable core (title, state, labels,
assignees). All optional, unenforced, additive, and folding under existing CRDT
semantics, so `Format-Version` stays at 1.

- [x] **Multiple assignees** (S). Assignees are now an OR-Set (an issue can have
  several). `edit --add-assignee`/`--remove-assignee` (repeatable); `-a` sets the
  sole assignee. `ls --assignee X` matches any member.
- [x] **Generic custom fields** (S). `field set|get|rm|list <id> [key] [value]`:
  free-form key/value metadata, one value per key, LWW per key (a single generic
  mechanism instead of one hardcoded field per concept).
- [x] **Related files with editable notes** (S). `file add|note|rm|list`: an
  OR-Set of repo-relative paths, each with an optional LWW note you can edit.
  Metadata pointers, not attachments.

- [x] **B6. `rm` / archive** (S). Tombstone op to delete or hide a mistaken issue.
  (done) `archive`/`restore` are a soft, reversible `set-archived` op (folds LWW);
  `rm` is an alias; `--purge` deletes the ref irreversibly.
- [x] **B8. Richer search and filters** (M). Text search; multi-label AND/OR;
  filter by assignee/priority/author; sort options. (done) `ls` gained repeatable
  `-l` (AND), `--assignee`, `--priority`, `--creator`, `-s/--search`, `--sort`,
  and `--archived`/`--all`.
- [x] **B9. `--format json`** (M). Machine-readable output for scripting. (done)
  `--format json` on `ls` and `--json` on `show`, over a stable schema.
- [x] **B11. `$EDITOR` integration** (S). Open the editor when create/comment is
  called with no body, like `git commit`. (done) `comment` with no text opens
  `$EDITOR` (then `$VISUAL`, then `vi`).
- [x] **B12. `gli config`** (S). Per-repo defaults (priority, labels, format).
  (done) `config get|set|list` under `gli.default.*`; `create` and `ls` fall back
  to them unless a flag overrides.
- [x] **B13. Shell completions + man page** (S). Generated via clap. (done)
  `gli completions <shell>` and `gli man`.
- [x] **B4-repair. `fsck --fix`** (M). (done) Quarantines refs that are not
  usable issues (empty chain, unparseable, or will not fold) by moving them to
  `refs/gli-quarantine/*`. Non-destructive (commits preserved) and recovers a
  repo that one bad ref had bricked.

---

## 0.3.0 - Performance

- [ ] **C16. `gix` read backend** (M). Move local reads onto gitoxide behind the
  existing `GitBackend` trait (the locked "gix-first for reads" decision). Fewer
  subprocess spawns, faster cold reads.
- [ ] **C17. Persistent on-disk cache** (M). Replace the per-run in-memory fold
  with a rebuildable cache in `.git/gli/` for large repos. Drop-in behind the
  same fold; still disposable, still never synced.

---

## 0.4.0 - Sync (the headline gap)

- [ ] **C14. Push/pull `refs/issues/*`** (M). Wrap `git push`/`git fetch` for the
  issue refnamespace so two clones can exchange issues at all. Network stays on
  the git CLI permanently (the locked decision). This is the first multi-machine
  capability.
- [ ] **D-unpushed. Harden `status` unpushed detection** (S). Already sketched;
  make it authoritative once real remotes exist.

---

## 0.5.0 - CRDT merge

- [ ] **C15. Merge algorithm** (L). Fold concurrent, diverged op chains
  deterministically: LWW-Register by (Lamport, op-id), OR-Set observed-remove
  tombstones, grow-only comment log deduped by op-id. The model is already built
  for this; this implements it. Merge is where the format meets reality, which is
  exactly why it ships after the format has been used single-user.
- [ ] **D21. Property tests for convergence** (M). `proptest`: random op
  sequences must fold identically regardless of order and partition.

---

## 0.6.0 and beyond - Extensions

- [ ] **C18. Attachments** (L). Inline blobs (size-guarded) and URL refs; the
  format already permits non-empty trees. Leave the git-LFS pointer seam open.
- [ ] **E27. Issue relations** (M). `blocks` / `relates-to` / `duplicate-of` as
  `relate`/`unrelate` operations. Reserve the design at 0.1.0, build here.
- [ ] **D22. Commit signing** (M). GPG/SSH signatures for verifiable authorship,
  closing the "authorship is forgeable" gap. Seam left from day one.
- [ ] **Reactions, stable (non-drifting) numbers, git-LFS** (M). Optional
  quality-of-life items if wanted.

---

## 1.0.0 - Stable

Cut when: the on-disk format is frozen and proven, sync and merge work and are
tested against real divergence, and the CLI surface is stable. At 1.0 the format
compatibility promise becomes a hard commitment.

---

## Explicitly NOT planned

- Reimplementing git's network transport (SSH/known_hosts is a decade-long
  git-bug scar; network stays on the git CLI).
- A hardcoded governance model. The tool only pushes/pulls refs and CRDT-merges;
  centralized, satellite-mirror, decentralized, and hard-fork topologies all
  emerge for free.
- A web/mobile UI or a server. `gli` is a local-first CLI by design.

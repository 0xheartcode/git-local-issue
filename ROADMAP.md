# gli roadmap

This maps everything planned for `gli`, grouped by release.

**Format-freeze rule.** The on-disk **format** is the only thing that is
expensive to change once people store real issues, so it is frozen carefully and
every feature is added additively on top of it. The `Format-Version` in
`FORMAT.md` is a separate number from the crate version and bumps only on a
breaking on-disk change (currently `1`, and it stays `1` across this whole plan).

**Versioning.** `gli` shipped `0.1.0` (crates.io + tag + GitHub release). From
here we stay on a **packed `0.1.x` patch line**: each release carries a themed
batch, and every change is **additive and backward compatible** — nothing breaks,
the CLI only grows, and `Format-Version` stays `1`. There is deliberately **no
minor or major bump planned**: a `1.0.0` (the format-freeze *hard promise*) is
parked, not scheduled. Pre-1.0 this is honest — SemVer makes no stability promise
below 1.0, which is the right posture for a personal-first tool.

Status legend: `[ ]` planned, `[~]` in progress, `[x]` done. Size: S/M/L.
Origin tags: `[rm]` prior roadmap, `[qa]` quality audit, `[ux]` numbering/UX,
`[pk]` panekit parity, `[new]` newly proposed.

---

## 0.1.0 - Shipped (published 2026-09-22)

The local, single-user core, live on crates.io and GitHub. Issues stored natively
in Git under `refs/issues/<uuidv7>` as chains of immutable operation commits;
state is the CRDT fold of the op log. Full command surface (`create`/`ls`/`show`/
`edit`/`comment`/`state`/`archive`/`field`/`file`/`config`/`fsck`/`status`/
`completions`/`man`), multiple assignees, generic custom fields, related files,
JSON output, `$EDITOR` integration, `#N` display numbers, comment edit/rm/purge,
cross-platform binaries via cargo-dist. `Format-Version 1`.

---

## 0.1.1 - Walls, cleanup, and solo-friendly numbers

Theme: harden the walls, deepen `cli.rs`, and ship codebase-prefixed numbers.
Nothing touches the network or the on-disk format. The packed, front-loaded
release. Additive and backward compatible.

### Walls and repo hygiene
- [ ] **A. Gate the coverage floor** (S) `[pk]`. A `COV_FLOOR` ratchet via
  `cargo llvm-cov --fail-under-lines` in the Makefile and CI, so coverage is a
  wall, not a report. Ratchet up as slices land; never down to pass.
- [ ] **B. `cov-html` + Makefile philosophy header** (S) `[pk]`. A browsable HTML
  coverage target and the "we encode best practices as deterministic checks" note.
  Mirror panekit's wall ergonomics too: a `── <name> ──` echo per sub-wall and a
  closing `═══ make check: ALL WALLS GREEN ═══` banner, and record the `COV_FLOOR`
  ratchet history as a dated in-file comment ("never lower to pass").
- [ ] **C. Re-certify mutation** (S/M) `[qa]`. Rerun `cargo-mutants` scoped to
  the core modules (`model`/`id`/`store`/`trailer`/`git`) and add a `make mutants`
  convenience. The last run (84.8%) predates the `#N` commits and is stale.
- [ ] **D. CHANGELOG conventions** (S) `[pk]`. Per-release theme lines, the
  "additive and backward compatible" note, compare-link footers for each version,
  and the pre-1.0 all-patch scheme documented in the header.
- [ ] **E. `docs/design/`** (S) `[pk]`. A design-doc folder, one doc per
  non-trivial feature, referenced from the CHANGELOG, seeded with
  `display-numbers.md`. Use panekit's ADR template: a `# Title`, a bare
  `Status: accepted (<version>)` line, an optional "Builds on `<doc>.md`", an
  opening `## Context`, body sections, and a closing `## Deferred (not in X)` that
  scopes what was intentionally left out.
- [ ] **F. `rust-toolchain.toml`** (S) `[pk]`. Pin the toolchain (today only the
  MSRV lives in `Cargo.toml`): `channel = "stable"`, `components = ["rustfmt","clippy"]`.
- [ ] **AE. `#![deny(missing_docs)]` wall** (S) `[pk]`. Add the inner attribute to
  the lib root and document every public item, so the docs.rs page is complete and
  an undocumented export fails the build. panekit enforces this on both crate roots;
  gli does not yet. A real wall, not just polish.
- [ ] **AF. `docs/ARCHITECTURE.md` as a numbered ADR** (S) `[pk]`. Formalize the
  signature decision (issues as immutable op-commit chains under `refs/issues/*`,
  state = the CRDT fold) in panekit's ADR shape: `# ADR-001: …`, `Status:` line,
  `## Context` → `## Decision` → `## Why not …` (rejected alternatives) →
  `## Consequences` → `## Shipped since v0.1`. Today this reasoning is scattered
  across `FORMAT.md`, the README, and the external PLAN doc.
- [ ] **AG. Reshape `README.md` to panekit's layout** (S) `[pk]`. Install-first,
  a "Why this exists" narrative, runnable annotated command blocks, no badge row —
  panekit's section order. Keep gli's content; restructure the frame.

### Internal refactor (behaviour unchanged)
- [ ] **G. Extract the `ls` query** (M) `[qa]`. Lift filter/search/sort out of
  `cmd_ls` into a testable `query` module. Kills the one real interface-depth
  finding (the 1560-line `cli.rs` god-file, `cmd_ls` = 136 regions @ 54% cover)
  and makes the logic unit-testable.
- [ ] **H. Extract display/handle rendering** (S/M) `[qa]`. Move handle rendering
  out of `cli.rs` into `id` (or a small render unit) and unit-test it. Prereq for I.

### Display numbering (display-only; Format-Version stays 1)
- [ ] **I. `gli.display.prefix`** (S) `[ux]`. Codebase/project-prefixed handles
  (`myproject-#1`). Default stays bare `#N`; the **actor** only ever surfaces on a
  true post-sync collision (`alice-#1` vs `bob-#1`). Solo users never see their
  name. Per-actor numbering stays the coordination-free truth underneath.
- [ ] **J. Auto-derive the prefix** (S) `[ux]`. Fall the prefix back to the repo's
  directory basename / remote name, so it works with zero config.

### Forward-compat safety net
- [ ] **AD. `gli migrate`** (S) `[new]`. Reserve the upgrade-on-install command now,
  while it is cheap. At `Format-Version 1` it is a deliberate no-op that reads the
  version and reports "nothing to migrate", so the UX contract ("run `gli migrate`
  after upgrading") exists from the start. It is the single home for (a) the day a
  breaking on-disk change lands pre-1.0 — the reserved escape hatch that keeps the
  format-freeze rule honest — and (b) importing issues from the shell predecessor
  `git-native-issue`. Real migration logic lands only when there is something to
  migrate; the scaffold guarantees the seam is there first.

---

## 0.1.2 - Performance

Theme: faster reads, identical behaviour, the cache still disposable and never
synced. Additive and backward compatible.

- [ ] **K. `gix` read backend** (M) `[rm]`. Move local reads onto gitoxide behind
  the existing `GitBackend` trait (the locked "gix-first for reads" decision).
  Fewer subprocess spawns, faster cold reads.
- [ ] **L. Persistent on-disk cache** (M) `[rm]`. Replace the per-run in-memory
  fold with a rebuildable cache in `.git/gli/` for large repos. Drop-in behind the
  same fold; still disposable, still never synced.

---

## 0.1.3 - Sync (the headline gap, first multi-machine)

Theme: two clones can finally exchange issues. Network stays on the git CLI
permanently (the locked decision). Additive and backward compatible.

- [ ] **M. Push/pull `refs/issues/*`** (M) `[rm]`. Wrap `git push`/`git fetch` for
  the issue refnamespace so two clones can exchange issues at all.
- [ ] **N. Harden `status` unpushed detection** (S) `[rm]`. Make the already-
  sketched unpushed detection authoritative once real remotes exist.

---

## 0.1.4 - CRDT merge

Theme: fold concurrent, diverged op chains deterministically, then build the
recovery and stability features that merge unlocks. Additive and backward
compatible.

- [ ] **O. Merge algorithm** (L) `[rm]`. LWW-Register by (Lamport, op-id), OR-Set
  observed-remove tombstones, grow-only comment log deduped by op-id. The model is
  already built for this; this implements it.
- [ ] **P. Property tests for convergence** (M) `[rm]`. `proptest`: random op
  sequences must fold identically regardless of order and partition.
- [ ] **Q. Superseded-write surfacing ("drafts")** (M) `[rm]`. Surface the LWW
  loser after a merge ("your title change X was superseded by Y from Alice;
  re-apply?"). Turns LWW's silent loss into a visible, recoverable prompt. Depends
  on O.
- [ ] **R. Stable (non-drifting) display numbers** (M) `[rm→promoted]`. Now
  buildable, because merge is the coordination point stable numbers always needed.
  Depends on O.

---

## 0.1.5 - Relations, threading, promote

Theme: issues and comments gain structure. All additive ops on the chain.

- [ ] **T. Issue relations** (M) `[rm]`. `blocks` / `relates-to` / `duplicate-of`
  as `relate`/`unrelate` operations.
- [ ] **U. Promote a comment to an issue** (S) `[rm]`. New issue whose body is the
  comment's text, plus a `relates-to` back to the original. Depends on T.
- [ ] **V. `Reply-To` comment threading** (S) `[rm]`. Optional `Reply-To: <op-id>`
  trailer so replies nest, without promoting comments to separate refs. Comments
  already carry a stable op-id, so this is additive.

---

## 0.1.6 - Heavy extensions

Theme: attachments and everything riding their blob seam, plus verifiable
authorship. Additive; `Format-Version` still `1`.

- [ ] **S. Attachments** (L) `[rm]`. Inline size-guarded blobs and URL refs; the
  format already permits non-empty trees. Leave the git-LFS pointer seam open.
- [ ] **W. Content-addressed comment bodies** (M) `[rm]`. Store a comment's text as
  a git blob referenced by hash, so the bytes can be erased (drop the blob) without
  rewriting the chain — the clean alternative to today's chain-rewrite `comment
  purge`. Depends on S's seam. Only worth building if guaranteed byte-erasure
  becomes a real requirement.
- [ ] **X. Commit signing** (M) `[rm]`. GPG/SSH signatures for verifiable
  authorship, closing the "authorship is forgeable" gap. Seam left from day one.
- [ ] **Y. Reactions + git-LFS pointers** (M) `[rm]`. Optional quality-of-life.

---

## 0.1.7 - Interop

Theme: reference and exchange issues beyond one repo.

- [ ] **Z. Cross-repo references** (M) `[new]`. A repo-qualified reference syntax
  (`otherproject-#5`) plus a known-repo registry. The natural extension of the
  0.1.1 prefix work.
- [ ] **AA. Bridges** (L) `[new]`. `gli export`/`import` (bulk JSON) and a
  GitHub/GitLab issues bridge. Was an explicit v0.1 non-goal; revisited here if
  interop is wanted.

---

## 0.1.8 - Optional interactive TUI

Theme: a visual browse/triage layer, dogfooding panekit. Optional — drop it and
the plan ends at 0.1.7.

- [ ] **AB. `gli` TUI** (L) `[new]`. An optional terminal UI using **panekit** as
  its headless test harness. Not in the original roadmap and arguably against the
  CLI-only stance — kept separate so it never bloats a real release and can be cut
  entirely.

---

## Parked (not on the current plan)

- **1.0.0 - format-freeze hard promise** (old item AC). Cut when the format is
  frozen and proven, sync and merge are tested against real divergence, and the CLI
  surface is stable. Deliberately **not scheduled**: staying pre-1.0 means `gli`
  never owes a hard compatibility promise, which suits a personal-first tool. Revisit
  only if `gli` takes on outside users who need that guarantee.

## Explicitly NOT planned

- Reimplementing git's network transport (SSH/known_hosts is a decade-long git-bug
  scar; network stays on the git CLI).
- A hardcoded governance model. The tool only pushes/pulls refs and CRDT-merges;
  centralized, satellite-mirror, decentralized, and hard-fork topologies all emerge
  for free.
- A server or web/mobile UI. `gli` is a local-first CLI by design (an optional TUI,
  item AB, is the one visual surface considered, and only as a thin local layer).

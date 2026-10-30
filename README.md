# 🗂️ git-local-issue (`gli`)

A distributed, offline-first issue tracker that stores issues natively in Git.

> **Inspired by [git-native-issue](https://github.com/remenoscodes/git-native-issue) (GPL-v2).**
> `git-local-issue` is a from-scratch Rust project with its own on-disk format.
> Thank you to everyone who is frustrated with git issues not being a native feature.

## Install

```sh
cargo install git-local-issue   # installs the `gli` binary
```

Prebuilt binaries for macOS, Linux, and Windows are attached to each
[release](https://github.com/0xheartcode/git-local-issue/releases).

## Try it in a minute

```sh
cd your-repo
gli init                                 # set up refs/issues in this repo
gli create "Fix the login redirect" -l bug -p high
gli ls                                   # #1  open  Fix the login redirect  [bug]
gli comment 1 "reproduced on Safari"
gli close 1 --reason "shipped in 1.4.2"
gli show 1
```

Everything runs offline, with no server and no database. Back issues up, or move
them to another clone, with a plain Git bundle:

```sh
git bundle create issues.bundle --glob='refs/issues/*'
```

## Why this exists

Issues belong with the code, not in a separate SaaS. `gli` keeps them inside Git,
under `refs/issues/*`, so they travel with every clone, work with no network, and
need no account. State is an append-only log of operations folded into the
current view, which means issues are built to merge between clones without a
central server (the sync and merge steps are on the roadmap). Because everything
is just Git refs, the governance model is not hardcoded: centralized,
satellite-mirror, and fully decentralized workflows all emerge for free, the same
way Git itself handles code.

Local, single-user today. Platform bridges (GitHub, GitLab, Gitea) and
multi-machine sync are planned; see [ROADMAP.md](ROADMAP.md). The on-disk
`Format-Version` is `1` and every release so far is additive.

## Commands

```
gli init                       # set up refs/issues in the current repo
gli create "title" [-l label] [-a assignee] [-p priority] [-d desc]
gli ls [--state ...] [-l label ...] [--assignee ...] [--priority ...] [--creator ...] [-s search] [--sort newest|oldest|title] [--archived|--all] [--format short|full|json]   # alias: gli list
gli show <id> [--ops] [--json]  # id = number (e.g. 1), alice-1, prefixed myproject-1, or a uuidv7 prefix
gli comment <id> ["text"]       # no text opens $EDITOR ($VISUAL, then vi)
gli comment edit|rm|purge <id> <n> ["text"]   # edit, soft-delete, or hard-delete comment n
gli edit <id> [--title ...] [--desc ...] [--add-label ...] [--remove-label ...] [-a ...] [--add-assignee ...] [--remove-assignee ...] [-p ...]
gli field set|get|rm|list <id> [key] [value]   # optional free-form key/value metadata
gli file add|note|rm|list <id> <path> [-n note]   # optional repo-relative file pointers with notes
gli state <id> open|closed [--reason ...] [--fixed-by <sha>]   # aliases: gli close, gli reopen
gli archive <id> [--purge]     # soft-hide an issue (alias: gli rm); --purge deletes the ref
gli restore <id>               # un-archive a soft-hidden issue
gli config get|set|list        # per-repo defaults under gli.default.* (priority, assignee, labels, format, display-prefix)
gli status                     # renumber notices, integrity, and unpushed/local-only issues
gli fsck [--fix]               # validate integrity; --fix quarantines unusable refs
gli migrate                    # report the on-disk format version (a no-op until a format change lands)
gli completions <bash|zsh|fish|powershell|elvish>   # print a shell completion script
gli man                        # print the man page (roff)
gli --version / --help         # --help also prints the version
```

`ls` filters all combine with AND: repeatable `-l/--label`, `--assignee`,
`--priority`, `--creator` (alias `--author`), `-s/--search` (free text over
title, description, labels, and comments), `--state`, and `--sort`. Archived
issues are hidden by default: `--archived` shows only archived, `--all` shows
both. `--format json` (on `ls`) and `--json` (on `show`) emit a stable schema.

Assignees are multi-valued: `--add-assignee` and `--remove-assignee` (both
repeatable) adjust the set, while `-a/--assignee` sets the sole assignee (`-a ""`
clears). `gli field ...` and `gli file ...` are optional, unenforced metadata.
`create` and `ls` fall back to `gli config` defaults when a flag is absent.

An issue id resolves from a display number (`1`, shown as `#1`), an
actor-qualified `alice-1` (used when a number is shared across actors after a
sync), the shown handle (including a prefixed `myproject-1`), or a UUIDv7 prefix.
Set a project prefix with `gli config set display-prefix <name>` (or `auto`) to
show handles as `myproject-#1`; see
[docs/design/display-numbers.md](docs/design/display-numbers.md).

## Design

Each change is an immutable operation commit in the issue's chain, and the
current state is the fold of that log, with conflict-free merge semantics per
field (Last-Writer-Wins register, OR-Set, grow-only comment log). The truth id is
a UUIDv7 (`refs/issues/<uuid>`); display numbers are computed on read. Metadata
rides on Git trailers, and the parser ignores unknown trailers and never rejects
a higher `Format-Version`, for forward and backward compatibility.

The full rationale is in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), and the
on-disk format is specified in [FORMAT.md](FORMAT.md).

## Cache and backup

`gli` keeps a disposable, rebuildable query cache: it is never synced and is
never the source of truth. If anything looks off, delete it and it rebuilds by
folding the operation log again. Back issues up with the `git bundle` command
above.

## License

GPL-v2. See [LICENSE](LICENSE).

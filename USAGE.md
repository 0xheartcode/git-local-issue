# Using gli (git-local-issue)

`gli` tracks issues natively inside Git. There is no database and no server:
each issue is a chain of commits under `refs/issues/*`, and everything works
offline. This guide takes you from a fresh install to comfortable daily use.

## Install

From source, inside a clone of this repository:

```
cargo install --path .
```

You also need the `git` CLI available on your `PATH`, since `gli` stores its data
directly in Git.

## Quick start

Start in any Git repository (or run `git init` first), then set up issue tracking:

```
$ gli init
Initialized gli issue tracking.
```

Create a couple of issues. The first has labels, a priority, and a description;
the second is a plain one-liner:

```
$ gli create "Login button does nothing" -l bug -l frontend -p high \
    -d "Clicking Login on /signin has no effect. No network request fires."
Created #1 (0190f2a1) Login button does nothing

$ gli create "Add dark mode toggle"
Created #2 (0190f2b3) Add dark mode toggle
```

List what you have:

```
$ gli ls
#2  0190f2b3  open  -       Add dark mode toggle
#1  0190f2a1  open  high    Login button does nothing   [bug, frontend]
```

Show a single issue and its history:

```
$ gli show 1
#1  (0190f2a1-...)  Login button does nothing
state:     open
priority:  high
labels:    bug, frontend
assignee:  -

Clicking Login on /signin has no effect. No network request fires.

History:
  create        alice   Login button does nothing
```

Add a comment:

```
$ gli comment alice-1 "Reproduced on Firefox 128 and Chrome 127."
Added comment to alice-1.
```

Edit the issue: add a label, and assign it. You can add a label and set the
assignee in the same call:

```
$ gli edit alice-1 --add-label regression -a alice
Updated alice-1.
```

Later, clear the assignee by passing an empty value:

```
$ gli edit alice-1 -a ""
Updated alice-1.
```

Close the issue with a reason and the fixing commit:

```
$ gli state alice-1 closed --reason "Fixed event handler binding" --fixed-by 9f3c1a2
alice-1 is now closed.
```

Reopen it if the fix turns out to be incomplete:

```
$ gli state alice-1 open --reason "Regressed after refactor"
alice-1 is now open.
```

Check the overall status and run an integrity check:

```
$ gli status
Issues: 2 total, 1 open, 1 closed.

$ gli fsck
All issue refs are consistent.
```

## Understanding ids

Every issue has two ways to refer to it:

- The truth id is a UUIDv7. The Git ref is `refs/issues/<uuid>`. You will usually
  see it as a short prefix like `0190f2a1`.
- The display id is a number, shown as `#1`, `#2`, and so on. Type the bare
  number (`gli show 1`), since a shell treats a leading `#` as a comment. Once
  two actors share a number (after a sync), the handle qualifies with the actor,
  e.g. `alice-#1` versus `bob-#1`, and you disambiguate by typing `alice-1`.

A few things worth knowing:

- Ids are case-insensitive, and `alice-1` (or the shown `alice-#1`) resolves the
  same issue; a uuid prefix can be typed in any case.
- If a bare number or prefix is ambiguous (it matches more than one issue), `gli`
  will not guess. It lists the candidates so you can retype an unambiguous one.
- Display numbers are drift-allowed. The same underlying issue can show a
  different number on a different machine or after a sync. The UUIDv7 is always
  the stable, authoritative id, so durable references should use it.

You can pass either form to any command that takes an `<id>`:

```
$ gli show alice-1
$ gli show 0190f2a1
```

## Filtering and listing

`gli ls` (also available as `gli list`) supports filtering and formatting:

```
$ gli ls --state open              # only open issues
$ gli ls --state closed            # only closed issues
$ gli ls -l bug                    # only issues carrying the "bug" label
$ gli ls --format short            # compact one line per issue (default-style)
$ gli ls --format full             # more detail per issue
```

Filters combine, so you can narrow to, for example, open bugs:

```
$ gli ls --state open -l bug
```

A note on labels: labels cannot contain commas (this is validated), since commas
are used to separate labels in output.

## Backup and restore

Because issues are ordinary Git refs, you can back them up with a Git bundle.
Use the `--glob` form:

```
$ git bundle create issues.bundle --glob='refs/issues/*'
```

Note: the shell-glob form `refs/issues/*` does not work here. Use `--glob=` as
shown above so Git, not your shell, expands the pattern.

To restore into a fresh clone, fetch the issue refs back explicitly:

```
$ git init
$ git fetch <bundle-or-remote> 'refs/issues/*:refs/issues/*'
```

The cache is disposable. `gli` keeps a rebuildable cache under `.git/gli`, and in
this version it folds the operation log in memory on each run. If anything ever
looks inconsistent, delete the cache and it rebuilds on the next command.

## Where issues live

Each issue is stored as one commit chain under `refs/issues/<uuidv7>`. Every
change (create, comment, edits, state changes, label changes) is an immutable
operation commit appended to that chain. The current state you see is the fold of
those operations.

For the full on-disk specification (commit shape, operation encoding, and the
versioned format), see FORMAT.md.

## Current limitations

Version 0.1.0 is the local, single-user core. It is offline-first and complete for
one person on one machine, but it does not yet sync between machines or merge
concurrent edits from multiple actors. Multi-machine sync and CRDT merge are
planned (the data model is already CRDT-ready). See ROADMAP.md for what is coming.

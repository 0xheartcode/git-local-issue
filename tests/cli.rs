//! CLI-level tests: drive the actual `gli` binary end to end in a temp repo.
//!
//! These cover the command surface (argument parsing, dispatch, output) that
//! the library-level tests in integration.rs do not exercise: the close/reopen
//! aliases, `show --ops`, and the per-change edit reporting.

use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Path to the compiled `gli` binary for this test run.
fn gli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_gli")
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("run git")
        .success();
    assert!(ok, "git {args:?} failed");
}

/// Run `gli <args>` in `dir`; return (stdout, success).
fn gli(dir: &Path, args: &[&str]) -> (String, bool) {
    let out = Command::new(gli_bin())
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run gli");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.success(),
    )
}

fn new_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.name", "Alice"]);
    git(dir.path(), &["config", "user.email", "alice@example.com"]);
    let (_, ok) = gli(dir.path(), &["init"]);
    assert!(ok);
    dir
}

#[test]
fn close_and_reopen_aliases_drive_state() {
    let repo = new_repo();
    let dir = repo.path();
    gli(dir, &["create", "Bug"]);

    let (out, ok) = gli(
        dir,
        &[
            "close",
            "alice-1",
            "--reason",
            "fixed",
            "--fixed-by",
            "abc123",
        ],
    );
    assert!(ok);
    assert!(out.contains("closed"), "got: {out}");
    let (shown, _) = gli(dir, &["show", "alice-1"]);
    assert!(shown.contains("closed"), "got: {shown}");
    assert!(shown.contains("fixed"), "reason should appear: {shown}");

    let (out, ok) = gli(dir, &["reopen", "alice-1"]);
    assert!(ok);
    assert!(out.contains("open"), "got: {out}");
    let (shown, _) = gli(dir, &["show", "alice-1"]);
    assert!(shown.contains("State:    open"), "got: {shown}");
}

#[test]
fn show_ops_lists_the_operation_log() {
    let repo = new_repo();
    let dir = repo.path();
    gli(dir, &["create", "Bug", "-l", "x"]);
    gli(dir, &["comment", "alice-1", "a note"]);
    gli(dir, &["close", "alice-1"]);

    let (out, ok) = gli(dir, &["show", "alice-1", "--ops"]);
    assert!(ok);
    assert!(out.contains("operation log"), "got: {out}");
    // create + add-label folded into create? No: label was on create. So ops
    // are create, comment, set-state.
    assert!(out.contains("create"), "got: {out}");
    assert!(out.contains("comment"), "got: {out}");
    assert!(out.contains("set-state"), "got: {out}");
    assert!(out.contains("lamport"), "got: {out}");
}

#[test]
fn edit_reports_each_applied_change() {
    let repo = new_repo();
    let dir = repo.path();
    gli(dir, &["create", "Old", "-l", "bug"]);

    let (out, ok) = gli(
        dir,
        &[
            "edit",
            "alice-1",
            "--title",
            "New",
            "--add-label",
            "triaged",
            "--remove-label",
            "bug",
            "-a",
            "bob",
        ],
    );
    assert!(ok);
    assert!(out.contains("set title: New"), "got: {out}");
    assert!(out.contains("add label: triaged"), "got: {out}");
    assert!(out.contains("remove label: bug"), "got: {out}");
    assert!(out.contains("set assignee: bob"), "got: {out}");
    assert!(out.contains("Applied 4 change(s)"), "got: {out}");

    // Clearing the assignee reports the clear.
    let (out, ok) = gli(dir, &["edit", "alice-1", "-a", ""]);
    assert!(ok);
    assert!(out.contains("clear assignee"), "got: {out}");
}

#[test]
fn help_prints_version() {
    let repo = new_repo();
    let (out, ok) = gli(repo.path(), &["--help"]);
    assert!(ok);
    assert!(out.contains("gli 0.1.0"), "help should show version: {out}");
}

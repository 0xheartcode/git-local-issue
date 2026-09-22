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
fn edit_changes_and_clears_priority() {
    let repo = new_repo();
    let dir = repo.path();
    gli(dir, &["create", "Prio me", "-p", "low"]);

    // Change it.
    let (out, ok) = gli(dir, &["edit", "alice-1", "-p", "high"]);
    assert!(ok && out.contains("set priority: high"), "got: {out}");
    let (json, _) = gli(dir, &["show", "alice-1", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["priority"], "high");

    // Clear it with an empty value.
    let (out, ok) = gli(dir, &["edit", "alice-1", "--priority", ""]);
    assert!(ok && out.contains("clear priority"), "got: {out}");
    let (json, _) = gli(dir, &["show", "alice-1", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(
        v["priority"].is_null(),
        "priority should be cleared: {json}"
    );
}

#[test]
fn archive_hides_and_restore_shows() {
    let repo = new_repo();
    let dir = repo.path();
    gli(dir, &["create", "One"]);
    gli(dir, &["create", "Two"]);

    let (out, ok) = gli(dir, &["archive", "alice-1"]);
    assert!(ok);
    assert!(out.contains("Archived"), "got: {out}");

    // Default ls hides it.
    let (ls, _) = gli(dir, &["ls"]);
    assert!(!ls.contains("One"), "archived issue should be hidden: {ls}");
    assert!(ls.contains("Two"), "got: {ls}");
    // --archived shows only it; --all shows both.
    let (only, _) = gli(dir, &["ls", "--archived"]);
    assert!(only.contains("One") && !only.contains("Two"), "got: {only}");
    let (all, _) = gli(dir, &["ls", "--all"]);
    assert!(all.contains("One") && all.contains("Two"), "got: {all}");

    // Restore brings it back.
    let (out, ok) = gli(dir, &["restore", "alice-1"]);
    assert!(ok && out.contains("Restored"), "got: {out}");
    let (ls, _) = gli(dir, &["ls"]);
    assert!(ls.contains("One"), "restored issue should show: {ls}");
}

#[test]
fn rm_alias_archives_and_purge_deletes() {
    let repo = new_repo();
    let dir = repo.path();
    gli(dir, &["create", "Soft"]);
    gli(dir, &["create", "Hard"]);

    // `rm` is an alias for archive (soft, reversible).
    let (_, ok) = gli(dir, &["rm", "alice-1"]);
    assert!(ok);
    let (all, _) = gli(dir, &["ls", "--all"]);
    assert!(all.contains("Soft"), "rm should archive, not delete: {all}");

    // --purge hard-deletes the ref.
    let (out, ok) = gli(dir, &["archive", "alice-2", "--purge"]);
    assert!(ok && out.contains("Permanently deleted"), "got: {out}");
    let (all, _) = gli(dir, &["ls", "--all"]);
    assert!(!all.contains("Hard"), "purged issue should be gone: {all}");
}

#[test]
fn filters_and_search_narrow_the_list() {
    let repo = new_repo();
    let dir = repo.path();
    gli(
        dir,
        &[
            "create",
            "Login bug",
            "-l",
            "bug",
            "-l",
            "frontend",
            "-p",
            "high",
            "-a",
            "alice",
        ],
    );
    gli(dir, &["create", "Docs typo", "-l", "docs"]);

    // Label AND: both labels required.
    let (out, _) = gli(dir, &["ls", "-l", "bug", "-l", "frontend"]);
    assert!(
        out.contains("Login bug") && !out.contains("Docs typo"),
        "got: {out}"
    );
    // Assignee + priority.
    let (out, _) = gli(dir, &["ls", "--assignee", "alice", "--priority", "high"]);
    assert!(
        out.contains("Login bug") && !out.contains("Docs typo"),
        "got: {out}"
    );
    // Text search over title.
    let (out, _) = gli(dir, &["ls", "--search", "typo"]);
    assert!(
        out.contains("Docs typo") && !out.contains("Login bug"),
        "got: {out}"
    );
}

#[test]
fn json_output_is_valid_and_complete() {
    let repo = new_repo();
    let dir = repo.path();
    gli(dir, &["create", "JSON me", "-l", "x", "-p", "low"]);
    gli(dir, &["comment", "alice-1", "a note"]);

    let (out, ok) = gli(dir, &["ls", "--format", "json"]);
    assert!(ok);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON array");
    let arr = parsed.as_array().expect("array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["title"], "JSON me");
    assert_eq!(arr[0]["priority"], "low");
    assert_eq!(arr[0]["comment_count"], 1);

    let (out, ok) = gli(dir, &["show", "alice-1", "--json"]);
    assert!(ok);
    let obj: serde_json::Value = serde_json::from_str(&out).expect("valid JSON object");
    assert_eq!(obj["comments"][0]["text"], "a note");
}

#[test]
fn config_defaults_apply_on_create() {
    let repo = new_repo();
    let dir = repo.path();
    gli(dir, &["config", "set", "priority", "medium"]);
    gli(dir, &["config", "set", "labels", "triage,needs-info"]);

    // create with no -p / -l picks up the defaults.
    gli(dir, &["create", "Uses defaults"]);
    let (out, _) = gli(dir, &["show", "alice-1", "--json"]);
    let obj: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(obj["priority"], "medium");
    let labels = obj["labels"].as_array().unwrap();
    assert!(labels.iter().any(|l| l == "triage"));
    assert!(labels.iter().any(|l| l == "needs-info"));

    // An explicit flag overrides the default.
    gli(dir, &["create", "Explicit", "-p", "high"]);
    let (out, _) = gli(dir, &["show", "alice-2", "--json"]);
    let obj: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(obj["priority"], "high");
}

#[test]
fn completions_and_man_render() {
    let repo = new_repo();
    let dir = repo.path();
    let (bash, ok) = gli(dir, &["completions", "bash"]);
    assert!(
        ok && bash.contains("_gli"),
        "bash completions should mention _gli"
    );
    let (man, ok) = gli(dir, &["man"]);
    assert!(ok && man.contains("gli"), "man page should render");
}

#[test]
fn multiple_assignees_and_metadata_flow() {
    let repo = new_repo();
    let dir = repo.path();
    gli(dir, &["create", "Parser crash", "-a", "alice"]);

    // Multiple assignees via edit.
    gli(
        dir,
        &[
            "edit",
            "alice-1",
            "--add-assignee",
            "bob",
            "--add-assignee",
            "carol",
        ],
    );
    gli(dir, &["edit", "alice-1", "--remove-assignee", "alice"]);

    // Custom fields.
    gli(dir, &["field", "set", "alice-1", "milestone", "v0.2"]);
    gli(dir, &["field", "set", "alice-1", "severity", "high"]);
    let (got, ok) = gli(dir, &["field", "get", "alice-1", "milestone"]);
    assert!(ok && got.trim() == "v0.2", "got: {got}");
    gli(dir, &["field", "rm", "alice-1", "severity"]);

    // Related files with an editable note.
    gli(
        dir,
        &["file", "add", "alice-1", "src/parser.rs", "-n", "here"],
    );
    gli(
        dir,
        &["file", "note", "alice-1", "src/parser.rs", "fixed here"],
    );

    // JSON reflects all of it.
    let (out, ok) = gli(dir, &["show", "alice-1", "--json"]);
    assert!(ok);
    let obj: serde_json::Value = serde_json::from_str(&out).unwrap();
    let assignees: Vec<String> = obj["assignees"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(assignees, vec!["bob", "carol"]);
    // Fields: milestone present, severity cleared.
    let fields = obj["fields"].as_array().unwrap();
    assert!(fields.iter().any(|f| f[0] == "milestone" && f[1] == "v0.2"));
    assert!(!fields.iter().any(|f| f[0] == "severity"));
    // File with its edited note.
    assert_eq!(obj["files"][0]["path"], "src/parser.rs");
    assert_eq!(obj["files"][0]["note"], "fixed here");

    // ls --assignee matches any member of the set.
    let (ls, _) = gli(dir, &["ls", "--assignee", "carol"]);
    assert!(ls.contains("Parser crash"), "got: {ls}");
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
    assert!(out.contains("set sole assignee: bob"), "got: {out}");
    assert!(out.contains("Applied 4 change(s)"), "got: {out}");

    // Clearing the assignee reports the clear.
    let (out, ok) = gli(dir, &["edit", "alice-1", "-a", ""]);
    assert!(ok);
    assert!(out.contains("clear assignees"), "got: {out}");
}

#[test]
fn help_prints_version() {
    let repo = new_repo();
    let (out, ok) = gli(repo.path(), &["--help"]);
    assert!(ok);
    assert!(out.contains("gli 0.1.0"), "help should show version: {out}");
}

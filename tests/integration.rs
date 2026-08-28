//! End-to-end tests driving the real `git` CLI backend against throwaway repos.
//!
//! These exercise the whole local path: op commits under `refs/issues/*`,
//! reading the chain back, folding, display numbering, and id resolution.

use gli::cache::Cache;
use gli::git::CliBackend;
use gli::model::{OpKind, State};
use gli::store::Store;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// A throwaway git repo with a configured identity.
struct Repo {
    _dir: TempDir,
    backend: CliBackend,
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

fn new_repo(name: &str, email: &str) -> Repo {
    let dir = TempDir::new().expect("tempdir");
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.name", name]);
    git(dir.path(), &["config", "user.email", email]);
    let backend = CliBackend::discover(dir.path()).expect("discover repo");
    Repo { _dir: dir, backend }
}

#[test]
fn create_then_read_back_state() {
    let repo = new_repo("Alice", "alice@example.com");
    let store = Store::new(&repo.backend);
    let uuid = store
        .create(
            "First bug",
            "It is broken.",
            vec!["bug".into(), "urgent".into()],
            Some("bob".into()),
            Some("high".into()),
        )
        .unwrap();

    let cache = Cache::build(&repo.backend).unwrap();
    let issue = cache.issue(&uuid).expect("issue exists");
    assert_eq!(issue.title(), "First bug");
    assert_eq!(issue.description(), "It is broken.");
    assert_eq!(issue.assignee(), Some("bob"));
    assert_eq!(issue.priority(), Some("high"));
    assert_eq!(
        issue.labels(),
        vec!["bug".to_string(), "urgent".to_string()]
    );
    assert_eq!(issue.state().state, State::Open);
    assert_eq!(issue.creator, "alice");

    // Display number is the actor's first issue.
    assert_eq!(cache.display_of(&uuid).nonce, "alice-1");
}

#[test]
fn comments_survive_reload_with_stable_identity() {
    let repo = new_repo("Alice", "alice@example.com");
    let store = Store::new(&repo.backend);
    let uuid = store.create("Bug", "", vec![], None, None).unwrap();
    store
        .append(
            &uuid,
            OpKind::Comment {
                text: "first".into(),
            },
        )
        .unwrap();
    store
        .append(
            &uuid,
            OpKind::Comment {
                text: "second".into(),
            },
        )
        .unwrap();

    // Read twice; the comment op-ids must be identical across reads (stable).
    let a = Cache::build(&repo.backend).unwrap();
    let b = Cache::build(&repo.backend).unwrap();
    let ca = &a.issue(&uuid).unwrap().comments;
    let cb = &b.issue(&uuid).unwrap().comments;
    assert_eq!(ca.len(), 2);
    let ids_a: Vec<&str> = ca.iter().map(|c| c.id.as_str()).collect();
    let ids_b: Vec<&str> = cb.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids_a, ids_b);
    assert_eq!(
        ca.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
        vec!["first", "second"]
    );
}

#[test]
fn edit_title_description_and_labels() {
    let repo = new_repo("Alice", "alice@example.com");
    let store = Store::new(&repo.backend);
    let uuid = store
        .create(
            "Old",
            "old body",
            vec!["keep".into(), "drop".into()],
            None,
            None,
        )
        .unwrap();

    store
        .append(
            &uuid,
            OpKind::SetTitle {
                title: "New".into(),
            },
        )
        .unwrap();
    store
        .append(
            &uuid,
            OpKind::SetDescription {
                description: "new body".into(),
            },
        )
        .unwrap();
    store
        .append(
            &uuid,
            OpKind::AddLabel {
                label: "added".into(),
            },
        )
        .unwrap();
    store
        .append(
            &uuid,
            OpKind::RemoveLabel {
                label: "drop".into(),
            },
        )
        .unwrap();

    let cache = Cache::build(&repo.backend).unwrap();
    let issue = cache.issue(&uuid).unwrap();
    assert_eq!(issue.title(), "New");
    assert_eq!(issue.description(), "new body");
    assert_eq!(
        issue.labels(),
        vec!["added".to_string(), "keep".to_string()]
    );
}

#[test]
fn comment_with_control_bytes_does_not_break_the_tracker() {
    // Regression: a comment containing the RS byte (0x1e), which git permits in
    // a commit message, once bricked every read because the reader used that
    // byte as a git-log record delimiter. The length-framed cat-file reader
    // must round-trip it intact and leave unrelated issues readable. (NUL is
    // separately rejected by git itself, so it never reaches storage.)
    let repo = new_repo("Alice", "alice@example.com");
    let store = Store::new(&repo.backend);
    let a = store.create("First", "", vec![], None, None).unwrap();
    let b = store.create("Second", "", vec![], None, None).unwrap();

    let nasty = "see record\u{1e}separator and unit\u{1f}sep and bell\u{7}";
    store
        .append(&a, OpKind::Comment { text: nasty.into() })
        .unwrap();

    // Every issue must still read, and the nasty comment must be intact.
    let cache = Cache::build(&repo.backend).unwrap();
    assert_eq!(cache.issues.len(), 2);
    assert_eq!(cache.issue(&b).unwrap().title(), "Second");
    let comments = &cache.issue(&a).unwrap().comments;
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].text, nasty);
}

#[test]
fn body_text_round_trips_exactly() {
    // Regression: body reconstruction once collapsed multiple blank lines and
    // trimmed leading whitespace. Both must survive a real git round-trip.
    let repo = new_repo("Alice", "alice@example.com");
    let store = Store::new(&repo.backend);
    let body = "  indented first line\n\n\nthree newlines above\n\ttrailing tab kept\t";
    let uuid = store.create("Fidelity", body, vec![], None, None).unwrap();
    let cache = Cache::build(&repo.backend).unwrap();
    assert_eq!(cache.issue(&uuid).unwrap().description(), body);
}

#[test]
fn oversized_fields_are_rejected() {
    let repo = new_repo("Alice", "alice@example.com");
    let store = Store::new(&repo.backend);
    let huge = "x".repeat(gli::store::MAX_FIELD_BYTES + 1);

    // On create (description).
    assert!(store.create("t", &huge, vec![], None, None).is_err());
    // On a comment append.
    let uuid = store.create("t", "", vec![], None, None).unwrap();
    assert!(store.append(&uuid, OpKind::Comment { text: huge }).is_err());
    // A field exactly at the limit is accepted.
    let at_limit = "y".repeat(gli::store::MAX_FIELD_BYTES);
    assert!(store.create("t", &at_limit, vec![], None, None).is_ok());
}

#[test]
fn labels_with_commas_are_rejected() {
    let repo = new_repo("Alice", "alice@example.com");
    let store = Store::new(&repo.backend);
    // On create.
    assert!(
        store
            .create("t", "", vec!["a,b".into()], None, None)
            .is_err()
    );
    // On add-label.
    let uuid = store.create("t", "", vec![], None, None).unwrap();
    assert!(
        store
            .append(
                &uuid,
                OpKind::AddLabel {
                    label: "x,y".into()
                }
            )
            .is_err()
    );
}

#[test]
fn state_transitions_are_lww() {
    let repo = new_repo("Alice", "alice@example.com");
    let store = Store::new(&repo.backend);
    let uuid = store.create("Bug", "", vec![], None, None).unwrap();

    store
        .append(
            &uuid,
            OpKind::SetState {
                state: State::Closed,
                reason: Some("fixed".into()),
                fixed_by: Some("cafebabe".into()),
            },
        )
        .unwrap();

    let cache = Cache::build(&repo.backend).unwrap();
    let st = cache.issue(&uuid).unwrap().state().clone();
    assert_eq!(st.state, State::Closed);
    assert_eq!(st.reason.as_deref(), Some("fixed"));
    assert_eq!(st.fixed_by.as_deref(), Some("cafebabe"));
}

#[test]
fn description_with_tricky_content_round_trips_through_git() {
    let repo = new_repo("Alice", "alice@example.com");
    let store = Store::new(&repo.backend);
    let tricky =
        "Repro steps:\n\n1. do a thing\n\nNote: this line looks like a trailer\nOp: not-an-op";
    let uuid = store.create("Edge", tricky, vec![], None, None).unwrap();

    let cache = Cache::build(&repo.backend).unwrap();
    assert_eq!(cache.issue(&uuid).unwrap().description(), tricky);
}

#[test]
fn lamport_clock_advances_along_the_chain() {
    let repo = new_repo("Alice", "alice@example.com");
    let store = Store::new(&repo.backend);
    let uuid = store.create("Bug", "", vec![], None, None).unwrap();
    store
        .append(&uuid, OpKind::Comment { text: "a".into() })
        .unwrap();
    store
        .append(&uuid, OpKind::Comment { text: "b".into() })
        .unwrap();

    let cache = Cache::build(&repo.backend).unwrap();
    let ops = &cache.ops[&uuid];
    let lamports: Vec<u64> = ops.iter().map(|o| o.lamport).collect();
    assert_eq!(lamports, vec![1, 2, 3]);
    assert_eq!(cache.issue(&uuid).unwrap().max_lamport, 3);
}

#[test]
fn cross_actor_display_numbers_do_not_clash() {
    // Two actors in the same repo both get a "-1"; they never collide because
    // display numbers are actor-scoped.
    let repo = new_repo("Alice", "alice@example.com");
    let alice = Store::new(&repo.backend);
    let a = alice.create("Alice bug", "", vec![], None, None).unwrap();

    // Switch identity, then create as bob in the same repo.
    git(repo._dir.path(), &["config", "user.name", "Bob"]);
    git(
        repo._dir.path(),
        &["config", "user.email", "bob@example.com"],
    );
    let bob = Store::new(&repo.backend);
    let b = bob.create("Bob bug", "", vec![], None, None).unwrap();

    let cache = Cache::build(&repo.backend).unwrap();
    assert_eq!(cache.display_of(&a).nonce, "alice-1");
    assert_eq!(cache.display_of(&b).nonce, "bob-1");
}

#[test]
fn forward_compat_unknown_trailers_and_higher_version_are_read() {
    // A commit written by a hypothetical newer gli: unknown trailers and a
    // higher Format-Version must still parse and fold, never be rejected.
    let repo = new_repo("Alice", "alice@example.com");
    let dir = repo._dir.path();

    let msg = "Create: Future issue\n\nfrom the future\n\n\
               Title: Future issue\n\
               Op: create\n\
               Op-Id: 00000000000000000000000000000001\n\
               Lamport: 1\n\
               Actor: alice\n\
               Format-Version: 999\n\
               X-Reactions: 5\n\
               Unknown-Trailer: ignore me\n";

    // Build the commit by hand and point a ref at it.
    let tree = run_out(dir, &["hash-object", "-t", "tree", "--stdin"], Some(""));
    let commit = run_out(dir, &["commit-tree", tree.trim()], Some(msg));
    git(
        dir,
        &[
            "update-ref",
            "refs/issues/00000000000000000000000000000abc",
            commit.trim(),
        ],
    );

    let cache = Cache::build(&repo.backend).unwrap();
    let issue = cache
        .issue("00000000000000000000000000000abc")
        .expect("future issue is readable");
    assert_eq!(issue.title(), "Future issue");
    assert_eq!(issue.description(), "from the future");
}

#[test]
fn documented_bundle_backup_and_restore_round_trips() {
    // Lock the exact backup command gli prints. A shell glob `refs/issues/*`
    // does NOT work (git resolves zero refs and refuses); the --glob form does.
    let repo = new_repo("Alice", "alice@example.com");
    let store = Store::new(&repo.backend);
    let uuid = store
        .create(
            "Precious",
            "do not lose me",
            vec!["keep".into()],
            None,
            None,
        )
        .unwrap();
    store
        .append(
            &uuid,
            OpKind::Comment {
                text: "backed up".into(),
            },
        )
        .unwrap();

    let bundle = repo._dir.path().join("issues.bundle");
    let bundle_str = bundle.to_str().unwrap();
    git(
        repo._dir.path(),
        &["bundle", "create", bundle_str, "--glob=refs/issues/*"],
    );
    assert!(bundle.exists(), "bundle was created");

    // Restore into a brand-new repo and read the issue back.
    let restored = TempDir::new().unwrap();
    git(restored.path(), &["init", "-q"]);
    git(
        restored.path(),
        &["fetch", bundle_str, "refs/issues/*:refs/issues/*"],
    );
    let backend = CliBackend::discover(restored.path()).unwrap();
    let cache = Cache::build(&backend).unwrap();
    let issue = cache.issue(&uuid).expect("issue restored from bundle");
    assert_eq!(issue.title(), "Precious");
    assert_eq!(issue.comments.len(), 1);
}

#[test]
fn ambiguous_prefix_lists_distinguishable_candidates() {
    // Two issues sharing an 8-char short prefix must still be told apart in the
    // error: the candidate list carries the actor-nonce plus a longer prefix.
    let repo = new_repo("Alice", "alice@example.com");
    let dir = repo._dir.path();
    let tree = run_out(dir, &["hash-object", "-t", "tree", "--stdin"], Some(""));
    for (i, uuid) in [
        "018fabcd0000000000000000000000a1",
        "018fabcd0000000000000000000000b2",
    ]
    .iter()
    .enumerate()
    {
        let msg = format!(
            "Create: I{i}\n\nTitle: I{i}\nOp: create\nOp-Id: {uuid}\nLamport: 1\nActor: alice\nFormat-Version: 1\n"
        );
        let commit = run_out(dir, &["commit-tree", tree.trim()], Some(&msg));
        git(
            dir,
            &["update-ref", &format!("refs/issues/{uuid}"), commit.trim()],
        );
    }

    let cache = Cache::build(&repo.backend).unwrap();
    let err = cache.resolve("018fabcd").unwrap_err().to_string();
    assert!(err.contains("ambiguous"), "got: {err}");
    // Both actor-nonces appear so the user can pick.
    assert!(err.contains("alice-1"), "got: {err}");
    assert!(err.contains("alice-2"), "got: {err}");
}

#[test]
fn fsck_passes_on_healthy_issues() {
    let repo = new_repo("Alice", "alice@example.com");
    let store = Store::new(&repo.backend);
    let uuid = store
        .create("Bug", "body", vec!["l".into()], None, None)
        .unwrap();
    store
        .append(&uuid, OpKind::Comment { text: "c".into() })
        .unwrap();

    let report = gli::fsck::check(&repo.backend).unwrap();
    assert_eq!(report.issue_count, 1);
    assert!(
        report.problems.is_empty(),
        "problems: {:?}",
        report.problems
    );
}

#[test]
fn fsck_flags_a_chain_with_no_create() {
    // A ref pointing at a lone non-Create op is corrupt: no root Create.
    let repo = new_repo("Alice", "alice@example.com");
    let dir = repo._dir.path();
    let msg = "Comment\n\nhi\n\nOp: comment\nOp-Id: 00000000000000000000000000000001\nLamport: 1\nActor: alice\nFormat-Version: 1\n";
    let tree = run_out(dir, &["hash-object", "-t", "tree", "--stdin"], Some(""));
    let commit = run_out(dir, &["commit-tree", tree.trim()], Some(msg));
    git(
        dir,
        &[
            "update-ref",
            "refs/issues/00000000000000000000000000000fff",
            commit.trim(),
        ],
    );

    let report = gli::fsck::check(&repo.backend).unwrap();
    assert!(
        report.problems.iter().any(|p| p.contains("no Create")),
        "expected a 'no Create' problem, got {:?}",
        report.problems
    );
}

#[test]
fn fsck_flags_duplicate_op_ids() {
    // Two ops sharing an Op-Id violates stable-identity; fsck must catch it.
    let repo = new_repo("Alice", "alice@example.com");
    let dir = repo._dir.path();
    let tree = run_out(dir, &["hash-object", "-t", "tree", "--stdin"], Some(""));
    let create_msg = "Create: X\n\nTitle: X\nOp: create\nOp-Id: 00000000000000000000000000000abc\nLamport: 1\nActor: alice\nFormat-Version: 1\n";
    let root = run_out(dir, &["commit-tree", tree.trim()], Some(create_msg));
    // Second op deliberately reuses the same Op-Id.
    let dup_msg = "Comment\n\nhi\n\nOp: comment\nOp-Id: 00000000000000000000000000000abc\nLamport: 2\nActor: alice\nFormat-Version: 1\n";
    let child = run_out(
        dir,
        &["commit-tree", tree.trim(), "-p", root.trim()],
        Some(dup_msg),
    );
    git(
        dir,
        &[
            "update-ref",
            "refs/issues/00000000000000000000000000000abc",
            child.trim(),
        ],
    );

    let report = gli::fsck::check(&repo.backend).unwrap();
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.contains("duplicate op-id")),
        "expected a duplicate op-id problem, got {:?}",
        report.problems
    );
}

/// Run a git command, feeding optional stdin, and return stdout.
fn run_out(dir: &Path, args: &[&str], stdin: Option<&str>) -> String {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new("git")
        .current_dir(dir)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(s) = stdin {
        child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "git {args:?} failed");
    String::from_utf8(out.stdout).unwrap()
}

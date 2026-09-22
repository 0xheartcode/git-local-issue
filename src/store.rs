//! Write operations: turning intents into op commits on the issue chains.
//!
//! This layer composes the [`GitBackend`] seam with the op model and id rules.
//! Every write reads the current chain to compute the next Lamport clock, mints
//! a stable op-id, serializes the op to a commit message, commits it onto the
//! chain tip, and moves the ref with a compare-and-swap.

use crate::cache::read_ops;
use crate::error::{GliError, Result};
use crate::git::{Author, GitBackend};
use crate::id;
use crate::model::{Op, OpKind, next_lamport};

/// Soft cap on any single free-form field (title, description, comment). The
/// format allows small inline content but attachments are not wired yet, so we
/// refuse oversized payloads rather than bloat the op chain. ~1 MB.
pub const MAX_FIELD_BYTES: usize = 1024 * 1024;

/// How many times `append` re-reads and retries when the ref moves under it
/// (optimistic concurrency: another process committed to the same issue first).
const MAX_APPEND_ATTEMPTS: u32 = 5;

/// Reject an oversized free-form field.
fn check_size(field: &str, value: &str) -> Result<()> {
    if value.len() > MAX_FIELD_BYTES {
        return Err(GliError::FieldTooLarge {
            field: field.to_string(),
            size: value.len(),
            limit: MAX_FIELD_BYTES,
        });
    }
    Ok(())
}

/// Validate the free-form and label content of an op before it is written.
fn validate_op(kind: &OpKind) -> Result<()> {
    match kind {
        OpKind::Create {
            title,
            description,
            labels,
            ..
        } => {
            check_size("title", title)?;
            check_size("description", description)?;
            for label in labels {
                crate::model::validate_label(label)?;
            }
        }
        OpKind::Comment { text } => check_size("comment", text)?,
        OpKind::SetTitle { title } => check_size("title", title)?,
        OpKind::SetDescription { description } => check_size("description", description)?,
        OpKind::AddLabel { label } => crate::model::validate_label(label)?,
        OpKind::SetField { value, .. } => check_size("field value", value)?,
        OpKind::AddFile { note, .. } | OpKind::SetFileNote { note, .. } => {
            check_size("file note", note)?
        }
        _ => {}
    }
    Ok(())
}

/// A handle over a [`GitBackend`] that performs issue writes.
pub struct Store<'a> {
    backend: &'a dyn GitBackend,
}

impl<'a> Store<'a> {
    pub fn new(backend: &'a dyn GitBackend) -> Store<'a> {
        Store { backend }
    }

    /// The git identity to stamp on commits, resolved from git config with
    /// sensible fallbacks so writes work on a bare-configured repo.
    pub fn author(&self) -> Author {
        let email = self
            .backend
            .config("user.email")
            .unwrap_or_else(|| "gli@localhost".to_string());
        let name = self.backend.config("user.name").unwrap_or_else(|| {
            email
                .split('@')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("gli")
                .to_string()
        });
        Author { name, email }
    }

    /// The actor slug that namespaces this identity's display numbers.
    pub fn actor_slug(&self) -> String {
        if let Some(name) = self.backend.config("user.name") {
            return id::actor_slug(&name);
        }
        if let Some(email) = self.backend.config("user.email") {
            let local = email.split('@').next().unwrap_or(&email);
            return id::actor_slug(local);
        }
        id::actor_slug("anon")
    }

    /// Create a new issue and return its uuid.
    pub fn create(
        &self,
        title: &str,
        description: &str,
        labels: Vec<String>,
        assignee: Option<String>,
        priority: Option<String>,
    ) -> Result<String> {
        let kind = OpKind::Create {
            title: title.to_string(),
            description: description.to_string(),
            labels,
            assignee,
            priority,
        };
        validate_op(&kind)?;
        let uuid = id::new_uuid();
        let op = Op::new(id::new_uuid(), 1, self.actor_slug(), kind);
        let commit = self
            .backend
            .commit_op(None, &op.to_commit_message(), &self.author())?;
        self.backend.update_ref(&refname(&uuid), &commit, None)?;
        Ok(uuid)
    }

    /// Append an operation to an existing issue's chain.
    ///
    /// This is optimistic: it reads the current tip, commits onto it, and moves
    /// the ref with a compare-and-swap. If a concurrent process moved the ref
    /// first, the swap fails and we re-read and retry (recomputing the Lamport
    /// clock), up to [`MAX_APPEND_ATTEMPTS`], then report a clean `Conflict`
    /// instead of a raw git error.
    pub fn append(&self, uuid: &str, kind: OpKind) -> Result<String> {
        validate_op(&kind)?;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let existing = self.load_ops(uuid)?;
            let tip = existing.iter().rev().find_map(|op| op.commit.clone());
            let lamport = next_lamport(&existing);
            let op = Op::new(id::new_uuid(), lamport, self.actor_slug(), kind.clone());
            let commit =
                self.backend
                    .commit_op(tip.as_deref(), &op.to_commit_message(), &self.author())?;
            match self
                .backend
                .update_ref(&refname(uuid), &commit, tip.as_deref())
            {
                Ok(()) => return Ok(commit),
                Err(e) => {
                    if attempt >= MAX_APPEND_ATTEMPTS {
                        // Surface the conflict, but keep the underlying git
                        // error visible for genuinely non-CAS failures.
                        return Err(match e {
                            GliError::Git(_) => GliError::Conflict {
                                uuid: uuid.to_string(),
                                attempts: attempt,
                            },
                            other => other,
                        });
                    }
                    // Ref moved under us: loop, re-read, and try again.
                }
            }
        }
    }

    /// Read the op log of a single issue by uuid.
    fn load_ops(&self, uuid: &str) -> Result<Vec<Op>> {
        for entry in self.backend.list_issue_refs()? {
            if entry.uuid == uuid {
                return read_ops(self.backend, &entry.tip);
            }
        }
        Err(crate::error::GliError::UnknownIssue(uuid.to_string()))
    }
}

/// The ref name for an issue uuid.
pub fn refname(uuid: &str) -> String {
    format!("refs/issues/{uuid}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn over_limit() -> String {
        "x".repeat(MAX_FIELD_BYTES + 1)
    }

    #[test]
    fn validate_op_rejects_oversized_content_for_every_kind() {
        assert!(
            validate_op(&OpKind::Create {
                title: over_limit(),
                description: String::new(),
                labels: Vec::new(),
                assignee: None,
                priority: None,
            })
            .is_err()
        );
        assert!(validate_op(&OpKind::Comment { text: over_limit() }).is_err());
        assert!(
            validate_op(&OpKind::SetTitle {
                title: over_limit()
            })
            .is_err()
        );
        assert!(
            validate_op(&OpKind::SetDescription {
                description: over_limit()
            })
            .is_err()
        );
        assert!(
            validate_op(&OpKind::SetField {
                key: "k".into(),
                value: over_limit(),
            })
            .is_err()
        );
        assert!(
            validate_op(&OpKind::AddFile {
                path: "p".into(),
                note: over_limit(),
            })
            .is_err()
        );
        assert!(
            validate_op(&OpKind::SetFileNote {
                path: "p".into(),
                note: over_limit(),
            })
            .is_err()
        );
    }

    #[test]
    fn check_size_boundary_is_inclusive_at_the_limit() {
        // Exactly the limit is allowed; one byte over is refused. This also pins
        // MAX_FIELD_BYTES at 1024*1024 (a `*`->`+` mutation drops it to ~2 KiB,
        // which this multi-kilobyte "ok" case would then wrongly reject).
        assert!(check_size("t", &"x".repeat(MAX_FIELD_BYTES)).is_ok());
        assert!(check_size("t", &"x".repeat(MAX_FIELD_BYTES + 1)).is_err());
        assert!(
            validate_op(&OpKind::SetTitle {
                title: "x".repeat(4096)
            })
            .is_ok()
        );
    }

    // --- Mock-backend tests for author derivation and append's retry loop. ---

    use crate::git::{RawCommit, RefEntry};
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// A minimal in-memory backend. `fail_updates` counts how many `update_ref`
    /// calls fail (with a git-shaped error, so `append` treats them as CAS
    /// losses) before one succeeds.
    struct MockBackend {
        cfg: HashMap<String, String>,
        fail_updates: Cell<u32>,
    }

    impl MockBackend {
        fn new() -> Self {
            MockBackend {
                cfg: HashMap::new(),
                fail_updates: Cell::new(0),
            }
        }
    }

    impl GitBackend for MockBackend {
        fn repo_root(&self) -> Result<PathBuf> {
            Ok(PathBuf::from("/repo"))
        }
        fn config(&self, key: &str) -> Option<String> {
            self.cfg.get(key).cloned()
        }
        fn list_issue_refs(&self) -> Result<Vec<RefEntry>> {
            Ok(vec![RefEntry {
                uuid: "u".into(),
                tip: "tip0".into(),
            }])
        }
        fn read_chain(&self, _tip: &str) -> Result<Vec<RawCommit>> {
            Ok(Vec::new())
        }
        fn commit_op(
            &self,
            _parent: Option<&str>,
            _message: &str,
            _author: &Author,
        ) -> Result<String> {
            Ok("newcommit".into())
        }
        fn update_ref(&self, _refname: &str, _new: &str, _old: Option<&str>) -> Result<()> {
            let remaining = self.fail_updates.get();
            if remaining > 0 {
                self.fail_updates.set(remaining - 1);
                Err(GliError::Git("simulated CAS loss".into()))
            } else {
                Ok(())
            }
        }
        fn delete_ref(&self, _refname: &str) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn author_name_falls_back_to_email_local_part() {
        // No user.name set: the name is derived from the email's non-empty local
        // part. Pins the `!s.is_empty()` filter (a deleted `!` would drop the
        // local part and yield the "gli" fallback instead).
        let mut backend = MockBackend::new();
        backend
            .cfg
            .insert("user.email".into(), "alice@example.com".into());
        let store = Store::new(&backend);
        let author = store.author();
        assert_eq!(author.name, "alice");
        assert_eq!(author.email, "alice@example.com");
    }

    #[test]
    fn append_retries_past_a_transient_cas_loss() {
        // Two update_ref calls fail, the third succeeds: append must keep trying
        // and return Ok. A broken retry bound (`>=` -> `<`) would give up first.
        let backend = MockBackend {
            cfg: HashMap::new(),
            fail_updates: Cell::new(2),
        };
        let store = Store::new(&backend);
        let out = store.append("u", OpKind::Comment { text: "hi".into() });
        assert!(out.is_ok());
    }

    #[test]
    fn append_gives_up_with_a_conflict_after_max_attempts() {
        // update_ref always fails: append exhausts its retries and reports a
        // Conflict tagged with the full attempt count. Pins both the attempt
        // counter (`+=`) and the retry bound (`>=`).
        let backend = MockBackend {
            cfg: HashMap::new(),
            fail_updates: Cell::new(1000),
        };
        let store = Store::new(&backend);
        match store.append("u", OpKind::Comment { text: "hi".into() }) {
            Err(GliError::Conflict { attempts, .. }) => {
                assert_eq!(attempts, MAX_APPEND_ATTEMPTS);
            }
            other => panic!("expected Conflict after max attempts, got {other:?}"),
        }
    }
}

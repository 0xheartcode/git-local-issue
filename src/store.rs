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

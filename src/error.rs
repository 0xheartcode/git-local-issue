//! Error types for gli.
//!
//! We keep a small typed error set for the domain-meaningful failures the CLI
//! wants to react to (not a git repo, ambiguous id, unknown issue). Everything
//! else flows through `anyhow` at the command layer.

use thiserror::Error;

/// Typed errors for the domain-meaningful failures the CLI reacts to.
#[derive(Debug, Error)]
pub enum GliError {
    /// Not run inside a git repository.
    #[error("not inside a git repository (run this from a git working tree, or `git init` first)")]
    NotAGitRepo,

    /// No issue matches the given id or query (the looked-up string).
    #[error("no issue matches '{0}'")]
    UnknownIssue(String),

    /// An id prefix or query matched more than one issue.
    #[error("id '{query}' is ambiguous; candidates: {}", candidates.join(", "))]
    AmbiguousId {
        /// The ambiguous id as it was typed.
        query: String,
        /// Display handles of the issues that matched.
        candidates: Vec<String>,
    },

    /// Could not determine an actor identity from git config.
    #[error("could not determine an actor identity: set `git config user.email` or `user.name`")]
    NoActorIdentity,

    /// A commit holds an operation that could not be parsed.
    #[error("malformed operation in commit {commit}: {reason}")]
    MalformedOp {
        /// The commit that carries the malformed operation.
        commit: String,
        /// Why the operation could not be parsed.
        reason: String,
    },

    /// A label value failed validation.
    #[error("invalid label {label:?}: {reason}")]
    InvalidLabel {
        /// The rejected label value.
        label: String,
        /// Why the label was rejected.
        reason: String,
    },

    /// A field value exceeds the allowed byte size.
    #[error("{field} is too large ({size} bytes, limit {limit}); attachments are not wired yet")]
    FieldTooLarge {
        /// Name of the oversized field.
        field: String,
        /// Actual size of the value, in bytes.
        size: usize,
        /// Maximum allowed size, in bytes.
        limit: usize,
    },

    /// A concurrent write changed the issue, so nothing was applied.
    #[error(
        "issue {uuid} changed underneath this write after {attempts} attempts (another process may be editing it); nothing was applied, retry"
    )]
    Conflict {
        /// Full UUID of the issue that changed underneath the write.
        uuid: String,
        /// Number of attempts made before giving up.
        attempts: u32,
    },

    /// A git command failed (the string carries the failure detail).
    #[error("git command failed: {0}")]
    Git(String),
}

/// Convenience result type whose error is [`GliError`].
pub type Result<T> = std::result::Result<T, GliError>;

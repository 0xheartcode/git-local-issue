//! Error types for gli.
//!
//! We keep a small typed error set for the domain-meaningful failures the CLI
//! wants to react to (not a git repo, ambiguous id, unknown issue). Everything
//! else flows through `anyhow` at the command layer.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GliError {
    #[error("not inside a git repository (run this from a git working tree, or `git init` first)")]
    NotAGitRepo,

    #[error("no issue matches '{0}'")]
    UnknownIssue(String),

    #[error("id '{query}' is ambiguous; candidates: {}", candidates.join(", "))]
    AmbiguousId {
        query: String,
        candidates: Vec<String>,
    },

    #[error("could not determine an actor identity: set `git config user.email` or `user.name`")]
    NoActorIdentity,

    #[error("malformed operation in commit {commit}: {reason}")]
    MalformedOp { commit: String, reason: String },

    #[error("invalid label {label:?}: {reason}")]
    InvalidLabel { label: String, reason: String },

    #[error("git command failed: {0}")]
    Git(String),
}

pub type Result<T> = std::result::Result<T, GliError>;

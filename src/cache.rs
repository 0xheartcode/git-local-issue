//! The query cache.
//!
//! Following the git-bug pattern, the cache is local, disposable, and
//! rebuildable: it is never the source of truth and never synced. v1 folds the
//! whole op log in memory on each run ([`Cache::build`]). A persistent on-disk
//! cache in `.git/gli/` is a later drop-in behind this same shape (same fold,
//! same query surface). The guarantee to document: delete the cache and it
//! rebuilds; back issues up with `git bundle create <file> refs/issues/*`.

use crate::error::{GliError, Result};
use crate::git::GitBackend;
use crate::id::{self, DisplayId, Resolution};
use crate::model::{Issue, Op};
use std::collections::HashMap;

/// An in-memory snapshot of every issue: folded state plus display ids.
pub struct Cache {
    /// Folded issues, sorted by uuid (time order) for stable listing.
    pub issues: Vec<Issue>,
    /// Raw op log per issue uuid (kept for status/fsck and appends).
    pub ops: HashMap<String, Vec<Op>>,
    /// Chain tip commit per issue uuid (for compare-and-swap writes).
    pub tips: HashMap<String, String>,
    /// Drift-allowed display ids per issue uuid.
    pub display: HashMap<String, DisplayId>,
}

impl Cache {
    /// Build the cache by reading and folding every `refs/issues/*` chain.
    pub fn build(backend: &dyn GitBackend) -> Result<Cache> {
        let mut issues = Vec::new();
        let mut ops_by_uuid = HashMap::new();
        let mut tips = HashMap::new();

        for entry in backend.list_issue_refs()? {
            let ops = read_ops(backend, &entry.tip)?;
            let issue = Issue::fold(&entry.uuid, &ops)?;
            tips.insert(entry.uuid.clone(), entry.tip.clone());
            ops_by_uuid.insert(entry.uuid.clone(), ops);
            issues.push(issue);
        }

        issues.sort_by(|a, b| a.uuid.cmp(&b.uuid));
        let display = id::assign_display_ids(&issues);

        Ok(Cache {
            issues,
            ops: ops_by_uuid,
            tips,
            display,
        })
    }

    /// Look up a folded issue by uuid.
    pub fn issue(&self, uuid: &str) -> Option<&Issue> {
        self.issues.iter().find(|i| i.uuid == uuid)
    }

    /// The display id for a uuid (falls back to a short prefix if unassigned).
    pub fn display_of(&self, uuid: &str) -> DisplayId {
        self.display.get(uuid).cloned().unwrap_or(DisplayId {
            uuid: uuid.to_string(),
            nonce: id::short_prefix(uuid),
            short: id::short_prefix(uuid),
        })
    }

    /// Resolve an id query to a single uuid, turning ambiguity/misses into
    /// typed errors the CLI can present.
    pub fn resolve(&self, query: &str) -> Result<String> {
        match id::resolve(query, &self.display) {
            Resolution::Unique(uuid) => Ok(uuid),
            Resolution::None => Err(GliError::UnknownIssue(query.to_string())),
            Resolution::Ambiguous(candidates) => {
                let candidates = candidates
                    .iter()
                    .map(|u| id::short_prefix(u))
                    .collect::<Vec<_>>();
                Err(GliError::AmbiguousId {
                    query: query.to_string(),
                    candidates,
                })
            }
        }
    }
}

/// Read and parse an issue's whole op log from its chain tip, attaching commit
/// provenance (author, timestamp) to each op.
pub fn read_ops(backend: &dyn GitBackend, tip: &str) -> Result<Vec<Op>> {
    let mut ops = Vec::new();
    for commit in backend.read_chain(tip)? {
        let mut op = Op::from_commit_message(&commit.message, &commit.sha)?;
        op.commit = Some(commit.sha.clone());
        op.author = Some(format!("{} <{}>", commit.author_name, commit.author_email));
        op.timestamp = Some(commit.author_time);
        ops.push(op);
    }
    Ok(ops)
}

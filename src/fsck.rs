//! Integrity checks over the issue chains.
//!
//! fsck reads every `refs/issues/*` chain raw and validates the structural
//! invariants the model relies on: a single root `Create`, a linear chain,
//! parseable ops, unique op-ids, and a foldable log. Problems are reported and
//! cause a non-zero exit; notes are informational.

use crate::error::Result;
use crate::git::GitBackend;
use crate::model::{Issue, Op, OpKind};
use std::collections::HashSet;

pub struct Report {
    pub issue_count: usize,
    pub notes: Vec<String>,
    pub problems: Vec<String>,
}

pub fn check(backend: &dyn GitBackend) -> Result<Report> {
    let mut report = Report {
        issue_count: 0,
        notes: Vec::new(),
        problems: Vec::new(),
    };

    for entry in backend.list_issue_refs()? {
        report.issue_count += 1;
        let short = crate::id::short_prefix(&entry.uuid);
        let commits = backend.read_chain(&entry.tip)?;

        if commits.is_empty() {
            report
                .problems
                .push(format!("{short}: ref points at an empty chain"));
            continue;
        }

        // Parse every commit into an op; a parse failure is a problem.
        let mut ops: Vec<Op> = Vec::new();
        let mut parse_ok = true;
        for commit in &commits {
            match Op::from_commit_message(&commit.message, &commit.sha) {
                Ok(op) => ops.push(op),
                Err(e) => {
                    report.problems.push(format!("{short}: {e}"));
                    parse_ok = false;
                }
            }
        }
        if !parse_ok {
            continue;
        }

        // Root must have no parent; the rest must be linear (<= 1 parent).
        if !commits[0].parents.is_empty() {
            report.problems.push(format!(
                "{short}: root commit has a parent (not a chain root)"
            ));
        }
        for commit in &commits[1..] {
            if commit.parents.len() > 1 {
                report.problems.push(format!(
                    "{short}: commit {} is a merge ({} parents); linear chain expected in v1",
                    &commit.sha[..7.min(commit.sha.len())],
                    commit.parents.len()
                ));
            }
        }

        // Exactly one Create, and it must be the root op.
        let create_count = ops
            .iter()
            .filter(|o| matches!(o.kind, OpKind::Create { .. }))
            .count();
        match create_count {
            1 => {
                if !matches!(ops[0].kind, OpKind::Create { .. }) {
                    report
                        .problems
                        .push(format!("{short}: Create op is not the root of the chain"));
                }
            }
            0 => report
                .problems
                .push(format!("{short}: no Create op in chain")),
            n => report
                .problems
                .push(format!("{short}: {n} Create ops (expected exactly 1)")),
        }

        // Op-ids must be unique within the issue (stable-identity invariant).
        let mut seen = HashSet::new();
        for op in &ops {
            if !seen.insert(op.id.clone()) {
                report
                    .problems
                    .push(format!("{short}: duplicate op-id {}", op.id));
            }
        }

        // Lamport clocks should not decrease along a linear chain.
        for pair in ops.windows(2) {
            if pair[1].lamport < pair[0].lamport {
                report.notes.push(format!(
                    "{short}: lamport decreases ({} -> {}) along the chain",
                    pair[0].lamport, pair[1].lamport
                ));
            }
        }

        // The log must fold cleanly.
        if let Err(e) = Issue::fold(&entry.uuid, &ops) {
            report.problems.push(format!("{short}: {e}"));
        }
    }

    Ok(report)
}

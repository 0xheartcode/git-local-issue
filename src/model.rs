//! The operation model and CRDT fold.
//!
//! Every change to an issue is an immutable [`Op`], serialized as one Git
//! commit in the issue's chain (root = `Create`). Current state is never
//! stored: it is COMPUTED by folding the op log with CRDT rules ([`Issue`]).
//!
//! CRDT design (merge algorithm lands with sync in v2, but the model is
//! correct-by-construction now):
//!   * comments          -> grow-only log, ordered by (lamport, op-id)
//!   * title/description  -> LWW-Register, ordered by (lamport, op-id)
//!   * state/assignee/priority -> LWW-Register
//!   * labels             -> OR-Set (observed-remove), tag = op-id
//!
//! Every op carries a STABLE, explicit op-id minted at creation (never derived
//! from position or content). This is the git-bug scar (#1582): comments
//! duplicate on sync when their identity is not stable. We bake it in now.

use crate::error::{GliError, Result};
use crate::trailer::{self, TrailerBlock};
use std::collections::BTreeMap;

/// On-disk format version, stamped on every op from commit #1. Parsers ignore
/// unknown fields and never reject a higher version (forward/backward compat).
pub const FORMAT_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Open,
    Closed,
}

impl State {
    pub fn as_str(&self) -> &'static str {
        match self {
            State::Open => "open",
            State::Closed => "closed",
        }
    }

    pub fn parse(s: &str) -> Option<State> {
        match s.trim().to_ascii_lowercase().as_str() {
            "open" => Some(State::Open),
            "closed" => Some(State::Closed),
            _ => None,
        }
    }
}

/// The payload of an operation. The op-id, lamport clock, actor and format
/// version live on the enclosing [`Op`], not here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpKind {
    Create {
        title: String,
        description: String,
        labels: Vec<String>,
        assignee: Option<String>,
        priority: Option<String>,
    },
    Comment {
        text: String,
    },
    SetTitle {
        title: String,
    },
    SetDescription {
        description: String,
    },
    SetState {
        state: State,
        reason: Option<String>,
        fixed_by: Option<String>,
    },
    SetAssignee {
        /// `None` means "unassigned" (an explicit clear).
        assignee: Option<String>,
    },
    SetPriority {
        priority: Option<String>,
    },
    AddLabel {
        label: String,
    },
    RemoveLabel {
        label: String,
    },
    /// Archive (hide) or restore an issue. Soft and reversible: nothing is
    /// destroyed, the op log stays intact, and it folds as an LWW-Register.
    SetArchived {
        archived: bool,
    },
}

impl OpKind {
    /// The `Op:` trailer slug identifying this operation type.
    pub fn slug(&self) -> &'static str {
        match self {
            OpKind::Create { .. } => "create",
            OpKind::Comment { .. } => "comment",
            OpKind::SetTitle { .. } => "set-title",
            OpKind::SetDescription { .. } => "set-description",
            OpKind::SetState { .. } => "set-state",
            OpKind::SetAssignee { .. } => "set-assignee",
            OpKind::SetPriority { .. } => "set-priority",
            OpKind::AddLabel { .. } => "add-label",
            OpKind::RemoveLabel { .. } => "remove-label",
            OpKind::SetArchived { .. } => "set-archived",
        }
    }
}

/// One immutable operation: payload plus stable identity and CRDT metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Op {
    /// Stable op-id (uuidv7 hex, no dashes). Minted once, never changes.
    pub id: String,
    /// Logical (Lamport) clock, not wall-clock.
    pub lamport: u64,
    /// Actor slug that authored this op (namespaces display numbers).
    pub actor: String,
    pub format_version: u32,
    pub kind: OpKind,
    /// Provenance filled in when read back from git (ignored on write).
    pub commit: Option<String>,
    pub author: Option<String>,
    pub timestamp: Option<i64>,
}

impl Op {
    /// Build a fresh op with the given identity and payload.
    pub fn new(id: String, lamport: u64, actor: String, kind: OpKind) -> Op {
        Op {
            id,
            lamport,
            actor,
            format_version: FORMAT_VERSION,
            kind,
            commit: None,
            author: None,
            timestamp: None,
        }
    }

    /// The commit subject line summarising this op for humans.
    fn subject(&self) -> String {
        match &self.kind {
            OpKind::Create { title, .. } => format!("Create: {}", first_line(title)),
            OpKind::Comment { .. } => "Comment".to_string(),
            OpKind::SetTitle { title } => format!("Set title: {}", first_line(title)),
            OpKind::SetDescription { .. } => "Set description".to_string(),
            OpKind::SetState { state, .. } => format!("Set state: {}", state.as_str()),
            OpKind::SetAssignee { assignee } => match assignee {
                Some(a) => format!("Set assignee: {a}"),
                None => "Clear assignee".to_string(),
            },
            OpKind::SetPriority { priority } => match priority {
                Some(p) => format!("Set priority: {p}"),
                None => "Clear priority".to_string(),
            },
            OpKind::AddLabel { label } => format!("Add label: {label}"),
            OpKind::RemoveLabel { label } => format!("Remove label: {label}"),
            OpKind::SetArchived { archived } => {
                if *archived { "Archive" } else { "Restore" }.to_string()
            }
        }
    }

    /// The free-form body (multi-line payload) for ops that carry one.
    fn body(&self) -> Option<&str> {
        match &self.kind {
            OpKind::Create { description, .. } if !description.is_empty() => Some(description),
            OpKind::Comment { text } => Some(text),
            OpKind::SetDescription { description } => Some(description),
            _ => None,
        }
    }

    /// Serialize this op into a full Git commit message: subject, optional
    /// body, then a trailer block. Kept deterministic so round-trips are exact.
    pub fn to_commit_message(&self) -> String {
        let mut t = TrailerBlock::new();
        // Op-specific trailers first, then the standard metadata block.
        match &self.kind {
            OpKind::Create {
                title,
                labels,
                assignee,
                priority,
                ..
            } => {
                t.push("Title", title);
                if !labels.is_empty() {
                    t.push("Labels", &labels.join(", "));
                }
                if let Some(a) = assignee {
                    t.push("Assignee", a);
                }
                if let Some(p) = priority {
                    t.push("Priority", p);
                }
            }
            OpKind::SetTitle { title } => t.push("Title", title),
            OpKind::SetState {
                state,
                reason,
                fixed_by,
            } => {
                t.push("State", state.as_str());
                if let Some(r) = reason {
                    t.push("Reason", r);
                }
                if let Some(f) = fixed_by {
                    t.push("Fixed-By", f);
                }
            }
            OpKind::SetAssignee { assignee } => {
                t.push("Assignee", assignee.as_deref().unwrap_or(""));
            }
            OpKind::SetPriority { priority } => {
                t.push("Priority", priority.as_deref().unwrap_or(""));
            }
            OpKind::AddLabel { label } | OpKind::RemoveLabel { label } => t.push("Label", label),
            OpKind::SetArchived { archived } => {
                t.push("Archived", if *archived { "true" } else { "false" })
            }
            OpKind::Comment { .. } | OpKind::SetDescription { .. } => {}
        }
        t.push("Op", self.kind.slug());
        t.push("Op-Id", &self.id);
        t.push("Lamport", &self.lamport.to_string());
        t.push("Actor", &self.actor);
        t.push("Format-Version", &self.format_version.to_string());

        let mut msg = self.subject();
        if let Some(body) = self.body() {
            msg.push_str("\n\n");
            msg.push_str(body);
        }
        msg.push_str("\n\n");
        msg.push_str(&t.render());
        msg
    }

    /// Parse an op back from a commit message. Unknown trailers are ignored,
    /// never rejected (forward compat). `commit` is used only for error text.
    pub fn from_commit_message(message: &str, commit: &str) -> Result<Op> {
        let (subject, body, trailers) = trailer::split_message(message);
        let _ = subject;

        let get = |k: &str| trailers.get(k).map(|s| s.as_str());
        let malformed = |reason: &str| GliError::MalformedOp {
            commit: commit.to_string(),
            reason: reason.to_string(),
        };

        let op_slug = get("Op").ok_or_else(|| malformed("missing Op trailer"))?;
        let id = get("Op-Id")
            .ok_or_else(|| malformed("missing Op-Id trailer"))?
            .to_string();
        let lamport: u64 = get("Lamport")
            .ok_or_else(|| malformed("missing Lamport trailer"))?
            .trim()
            .parse()
            .map_err(|_| malformed("Lamport is not a non-negative integer"))?;
        let actor = get("Actor")
            .ok_or_else(|| malformed("missing Actor trailer"))?
            .to_string();
        // Unknown/higher versions are accepted: we never reject on version.
        let format_version: u32 = get("Format-Version")
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(FORMAT_VERSION);

        let opt = |k: &str| get(k).map(|s| s.to_string());
        let opt_nonempty = |k: &str| get(k).filter(|s| !s.is_empty()).map(|s| s.to_string());

        let kind = match op_slug {
            "create" => OpKind::Create {
                title: opt("Title").unwrap_or_default(),
                description: body.clone(),
                labels: get("Labels").map(split_labels).unwrap_or_default(),
                assignee: opt_nonempty("Assignee"),
                priority: opt_nonempty("Priority"),
            },
            "comment" => OpKind::Comment { text: body.clone() },
            "set-title" => OpKind::SetTitle {
                title: opt("Title").unwrap_or_default(),
            },
            "set-description" => OpKind::SetDescription {
                description: body.clone(),
            },
            "set-state" => OpKind::SetState {
                state: get("State")
                    .and_then(State::parse)
                    .ok_or_else(|| malformed("set-state has no valid State trailer"))?,
                reason: opt_nonempty("Reason"),
                fixed_by: opt_nonempty("Fixed-By"),
            },
            "set-assignee" => OpKind::SetAssignee {
                assignee: opt_nonempty("Assignee"),
            },
            "set-priority" => OpKind::SetPriority {
                priority: opt_nonempty("Priority"),
            },
            "add-label" => OpKind::AddLabel {
                label: get("Label")
                    .ok_or_else(|| malformed("add-label has no Label trailer"))?
                    .to_string(),
            },
            "remove-label" => OpKind::RemoveLabel {
                label: get("Label")
                    .ok_or_else(|| malformed("remove-label has no Label trailer"))?
                    .to_string(),
            },
            "set-archived" => OpKind::SetArchived {
                // Default to archived=true when the flag is missing or unparsable:
                // the op only exists to change the bit, and true is its usual intent.
                archived: get("Archived").map(|v| v.trim() == "true").unwrap_or(true),
            },
            other => return Err(malformed(&format!("unknown Op type '{other}'"))),
        };

        Ok(Op {
            id,
            lamport,
            actor,
            format_version,
            kind,
            commit: Some(commit.to_string()),
            author: None,
            timestamp: None,
        })
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("").trim()
}

fn split_labels(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------
// CRDT registers
// ---------------------------------------------------------------------------

/// Last-Writer-Wins register ordered by (lamport, op-id). The op-id tie-break
/// makes the winner deterministic across clones when two writes share a clock.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lww<T> {
    value: T,
    lamport: u64,
    tiebreak: String,
    set: bool,
}

impl<T: Clone> Lww<T> {
    /// Apply a write; keep it only if it strictly beats the current stamp.
    pub fn apply(&mut self, value: T, lamport: u64, op_id: &str) {
        let incoming = (lamport, op_id);
        let current = (self.lamport, self.tiebreak.as_str());
        if !self.set || incoming > current {
            self.value = value;
            self.lamport = lamport;
            self.tiebreak = op_id.to_string();
            self.set = true;
        }
    }

    pub fn get(&self) -> &T {
        &self.value
    }

    pub fn is_set(&self) -> bool {
        self.set
    }
}

/// Observed-remove set. Each element maps to the set of live add-tags (op-ids).
/// An element is present iff it has at least one live tag.
///
/// v1 folds a single linear chain, so a remove clears every tag observed so
/// far for the element. The concurrent-merge tombstone handling lands with
/// sync (v2); the model already carries per-add tags so that is a drop-in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrSet {
    tags: BTreeMap<String, Vec<String>>,
}

impl OrSet {
    pub fn add(&mut self, element: &str, tag: &str) {
        let entry = self.tags.entry(element.to_string()).or_default();
        if !entry.iter().any(|t| t == tag) {
            entry.push(tag.to_string());
        }
    }

    pub fn remove(&mut self, element: &str) {
        self.tags.remove(element);
    }

    pub fn contains(&self, element: &str) -> bool {
        self.tags.get(element).is_some_and(|t| !t.is_empty())
    }

    /// Present elements, sorted for stable display.
    pub fn values(&self) -> Vec<String> {
        self.tags
            .iter()
            .filter(|(_, tags)| !tags.is_empty())
            .map(|(el, _)| el.clone())
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub id: String,
    pub actor: String,
    pub text: String,
    pub lamport: u64,
    pub timestamp: Option<i64>,
    pub author: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateVal {
    pub state: State,
    pub reason: Option<String>,
    pub fixed_by: Option<String>,
}

impl Default for StateVal {
    fn default() -> Self {
        StateVal {
            state: State::Open,
            reason: None,
            fixed_by: None,
        }
    }
}

// ---------------------------------------------------------------------------
// The folded issue
// ---------------------------------------------------------------------------

/// The current state of an issue, computed by folding its op log. Never stored
/// as truth: rebuild it any time by re-reading the chain.
#[derive(Clone, Debug)]
pub struct Issue {
    /// The issue uuid (hex, no dashes). Truth identity; ref = refs/issues/<uuid>.
    pub uuid: String,
    pub title: Lww<String>,
    pub description: Lww<String>,
    pub state: Lww<StateVal>,
    pub assignee: Lww<Option<String>>,
    pub priority: Lww<Option<String>>,
    pub labels: OrSet,
    pub comments: Vec<Comment>,
    /// Whether the issue is archived (hidden from default listings).
    pub archived: Lww<bool>,
    /// Actor that created the issue (namespaces its display number).
    pub creator: String,
    pub created_at: Option<i64>,
    pub op_count: usize,
    pub max_lamport: u64,
}

impl Issue {
    /// Fold an op log into current issue state.
    ///
    /// Ops are applied in causal order: (lamport, op-id). On a single linear
    /// chain this equals commit order; sorting keeps us correct once concurrent
    /// branches merge in v2.
    pub fn fold(uuid: &str, ops: &[Op]) -> Result<Issue> {
        let mut ordered: Vec<&Op> = ops.iter().collect();
        ordered.sort_by(|a, b| (a.lamport, &a.id).cmp(&(b.lamport, &b.id)));

        let create = ordered
            .iter()
            .find(|op| matches!(op.kind, OpKind::Create { .. }))
            .ok_or_else(|| GliError::MalformedOp {
                commit: uuid.to_string(),
                reason: "issue has no Create operation".to_string(),
            })?;

        let mut issue = Issue {
            uuid: uuid.to_string(),
            title: Lww::default(),
            description: Lww::default(),
            state: Lww::default(),
            assignee: Lww::default(),
            priority: Lww::default(),
            labels: OrSet::default(),
            comments: Vec::new(),
            archived: Lww::default(),
            creator: create.actor.clone(),
            created_at: create.timestamp,
            op_count: ordered.len(),
            max_lamport: 0,
        };

        for op in &ordered {
            issue.max_lamport = issue.max_lamport.max(op.lamport);
            match &op.kind {
                OpKind::Create {
                    title,
                    description,
                    labels,
                    assignee,
                    priority,
                } => {
                    issue.title.apply(title.clone(), op.lamport, &op.id);
                    issue
                        .description
                        .apply(description.clone(), op.lamport, &op.id);
                    issue.state.apply(StateVal::default(), op.lamport, &op.id);
                    for label in labels {
                        issue.labels.add(label, &op.id);
                    }
                    if let Some(a) = assignee {
                        issue.assignee.apply(Some(a.clone()), op.lamport, &op.id);
                    }
                    if let Some(p) = priority {
                        issue.priority.apply(Some(p.clone()), op.lamport, &op.id);
                    }
                }
                OpKind::Comment { text } => issue.comments.push(Comment {
                    id: op.id.clone(),
                    actor: op.actor.clone(),
                    text: text.clone(),
                    lamport: op.lamport,
                    timestamp: op.timestamp,
                    author: op.author.clone(),
                }),
                OpKind::SetTitle { title } => issue.title.apply(title.clone(), op.lamport, &op.id),
                OpKind::SetDescription { description } => {
                    issue
                        .description
                        .apply(description.clone(), op.lamport, &op.id)
                }
                OpKind::SetState {
                    state,
                    reason,
                    fixed_by,
                } => issue.state.apply(
                    StateVal {
                        state: state.clone(),
                        reason: reason.clone(),
                        fixed_by: fixed_by.clone(),
                    },
                    op.lamport,
                    &op.id,
                ),
                OpKind::SetAssignee { assignee } => {
                    issue.assignee.apply(assignee.clone(), op.lamport, &op.id)
                }
                OpKind::SetPriority { priority } => {
                    issue.priority.apply(priority.clone(), op.lamport, &op.id)
                }
                OpKind::AddLabel { label } => issue.labels.add(label, &op.id),
                OpKind::RemoveLabel { label } => issue.labels.remove(label),
                OpKind::SetArchived { archived } => {
                    issue.archived.apply(*archived, op.lamport, &op.id)
                }
            }
        }

        // Comments are a grow-only log ordered by (lamport, op-id) for a stable,
        // duplicate-free display regardless of read order.
        issue
            .comments
            .sort_by(|a, b| (a.lamport, &a.id).cmp(&(b.lamport, &b.id)));
        issue.comments.dedup_by(|a, b| a.id == b.id);

        Ok(issue)
    }

    pub fn title(&self) -> &str {
        self.title.get()
    }

    pub fn description(&self) -> &str {
        self.description.get()
    }

    pub fn state(&self) -> &StateVal {
        self.state.get()
    }

    pub fn assignee(&self) -> Option<&str> {
        self.assignee.get().as_deref()
    }

    pub fn priority(&self) -> Option<&str> {
        self.priority.get().as_deref()
    }

    pub fn labels(&self) -> Vec<String> {
        self.labels.values()
    }

    pub fn archived(&self) -> bool {
        *self.archived.get()
    }
}

/// The next Lamport clock value for a new op given the ops already seen.
pub fn next_lamport(ops: &[Op]) -> u64 {
    ops.iter().map(|op| op.lamport).max().unwrap_or(0) + 1
}

/// Validate a label value. Labels are single-line and comma-free because the
/// `Labels:` trailer is comma-separated and trailers are single-line; a comma
/// or newline would be silently mangled on round-trip, so we reject it up front.
pub fn validate_label(label: &str) -> Result<()> {
    let reject = |reason: &str| {
        Err(GliError::InvalidLabel {
            label: label.to_string(),
            reason: reason.to_string(),
        })
    };
    if label.trim().is_empty() {
        return reject("must not be empty");
    }
    if label.contains(',') {
        return reject("must not contain a comma");
    }
    if label.contains('\n') || label.contains('\r') {
        return reject("must not contain a newline");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(id: &str, lamport: u64, actor: &str, kind: OpKind) -> Op {
        Op::new(id.to_string(), lamport, actor.to_string(), kind)
    }

    fn create(id: &str, lamport: u64, title: &str) -> Op {
        op(
            id,
            lamport,
            "alice",
            OpKind::Create {
                title: title.to_string(),
                description: String::new(),
                labels: Vec::new(),
                assignee: None,
                priority: None,
            },
        )
    }

    #[test]
    fn fold_requires_a_create() {
        let ops = vec![op("a", 1, "alice", OpKind::Comment { text: "hi".into() })];
        assert!(Issue::fold("uuid", &ops).is_err());
    }

    #[test]
    fn lww_title_higher_lamport_wins_regardless_of_order() {
        // Apply the later-lamport op first; the fold must still pick it.
        let ops = vec![
            create("c", 1, "original"),
            op(
                "t2",
                5,
                "alice",
                OpKind::SetTitle {
                    title: "newest".into(),
                },
            ),
            op(
                "t1",
                3,
                "alice",
                OpKind::SetTitle {
                    title: "middle".into(),
                },
            ),
        ];
        let issue = Issue::fold("uuid", &ops).unwrap();
        assert_eq!(issue.title(), "newest");
    }

    #[test]
    fn lww_same_lamport_breaks_by_op_id_deterministically() {
        // Two writes share a Lamport clock; the larger op-id must win, and the
        // outcome must not depend on the order the ops are folded in.
        let a = op("aaa", 4, "alice", OpKind::SetTitle { title: "a".into() });
        let b = op("bbb", 4, "alice", OpKind::SetTitle { title: "b".into() });
        let forward = Issue::fold("u", &[create("c", 1, "x"), a.clone(), b.clone()]).unwrap();
        let backward = Issue::fold("u", &[create("c", 1, "x"), b, a]).unwrap();
        assert_eq!(forward.title(), "b");
        assert_eq!(backward.title(), "b");
    }

    #[test]
    fn state_is_lww() {
        let ops = vec![
            create("c", 1, "t"),
            op(
                "s1",
                2,
                "alice",
                OpKind::SetState {
                    state: State::Closed,
                    reason: Some("done".into()),
                    fixed_by: None,
                },
            ),
            op(
                "s2",
                3,
                "alice",
                OpKind::SetState {
                    state: State::Open,
                    reason: None,
                    fixed_by: None,
                },
            ),
        ];
        let issue = Issue::fold("u", &ops).unwrap();
        assert_eq!(issue.state().state, State::Open);
    }

    #[test]
    fn orset_add_and_remove() {
        let ops = vec![
            create("c", 1, "t"),
            op(
                "l1",
                2,
                "alice",
                OpKind::AddLabel {
                    label: "bug".into(),
                },
            ),
            op(
                "l2",
                3,
                "alice",
                OpKind::AddLabel {
                    label: "urgent".into(),
                },
            ),
            op(
                "l3",
                4,
                "alice",
                OpKind::RemoveLabel {
                    label: "urgent".into(),
                },
            ),
        ];
        let issue = Issue::fold("u", &ops).unwrap();
        assert_eq!(issue.labels(), vec!["bug".to_string()]);
        assert!(issue.labels.contains("bug"));
        assert!(!issue.labels.contains("urgent"));
        assert!(!issue.labels.contains("never-added"));
    }

    #[test]
    fn orset_add_is_idempotent_per_tag() {
        // Re-applying the same add op (same tag) must not create a duplicate
        // tag, so a single later remove still clears the element.
        let add = op(
            "l1",
            2,
            "alice",
            OpKind::AddLabel {
                label: "bug".into(),
            },
        );
        let ops = vec![
            create("c", 1, "t"),
            add.clone(),
            add,
            op(
                "l2",
                3,
                "alice",
                OpKind::RemoveLabel {
                    label: "bug".into(),
                },
            ),
        ];
        let issue = Issue::fold("u", &ops).unwrap();
        assert!(!issue.labels.contains("bug"));
    }

    #[test]
    fn label_validation_rejects_comma_and_newline() {
        assert!(validate_label("ok").is_ok());
        assert!(validate_label("a,b").is_err());
        assert!(validate_label("a\nb").is_err());
        assert!(validate_label("  ").is_err());
    }

    #[test]
    fn orset_readd_after_remove_is_present() {
        let ops = vec![
            create("c", 1, "t"),
            op(
                "l1",
                2,
                "alice",
                OpKind::AddLabel {
                    label: "bug".into(),
                },
            ),
            op(
                "l2",
                3,
                "alice",
                OpKind::RemoveLabel {
                    label: "bug".into(),
                },
            ),
            op(
                "l3",
                4,
                "alice",
                OpKind::AddLabel {
                    label: "bug".into(),
                },
            ),
        ];
        let issue = Issue::fold("u", &ops).unwrap();
        assert!(issue.labels.contains("bug"));
    }

    #[test]
    fn comments_are_grow_only_and_ordered_by_clock() {
        let ops = vec![
            create("c", 1, "t"),
            op(
                "m2",
                3,
                "alice",
                OpKind::Comment {
                    text: "second".into(),
                },
            ),
            op(
                "m1",
                2,
                "bob",
                OpKind::Comment {
                    text: "first".into(),
                },
            ),
        ];
        let issue = Issue::fold("u", &ops).unwrap();
        let texts: Vec<&str> = issue.comments.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["first", "second"]);
    }

    #[test]
    fn comments_dedup_by_stable_op_id() {
        // The same comment op appearing twice (as it might after a naive sync)
        // must collapse to one: identity is the op-id, not position/content.
        let dup = op(
            "m1",
            2,
            "alice",
            OpKind::Comment {
                text: "hello".into(),
            },
        );
        let ops = vec![create("c", 1, "t"), dup.clone(), dup];
        let issue = Issue::fold("u", &ops).unwrap();
        assert_eq!(issue.comments.len(), 1);
    }

    #[test]
    fn create_carries_initial_fields() {
        let ops = vec![op(
            "c",
            1,
            "alice",
            OpKind::Create {
                title: "T".into(),
                description: "D".into(),
                labels: vec!["a".into(), "b".into()],
                assignee: Some("bob".into()),
                priority: Some("high".into()),
            },
        )];
        let issue = Issue::fold("u", &ops).unwrap();
        assert_eq!(issue.title(), "T");
        assert_eq!(issue.description(), "D");
        assert_eq!(issue.assignee(), Some("bob"));
        assert_eq!(issue.priority(), Some("high"));
        assert_eq!(issue.labels(), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(issue.state().state, State::Open);
    }

    #[test]
    fn assignee_can_be_cleared() {
        let ops = vec![
            op(
                "c",
                1,
                "alice",
                OpKind::Create {
                    title: "T".into(),
                    description: String::new(),
                    labels: Vec::new(),
                    assignee: Some("bob".into()),
                    priority: None,
                },
            ),
            op("a1", 2, "alice", OpKind::SetAssignee { assignee: None }),
        ];
        let issue = Issue::fold("u", &ops).unwrap();
        assert_eq!(issue.assignee(), None);
    }

    #[test]
    fn next_lamport_advances() {
        let ops = vec![
            create("c", 1, "t"),
            op("x", 7, "alice", OpKind::Comment { text: "c".into() }),
        ];
        assert_eq!(next_lamport(&ops), 8);
        assert_eq!(next_lamport(&[]), 1);
    }

    // --- round-trip and forward-compat -------------------------------------

    fn roundtrip(op: &Op) -> Op {
        let msg = op.to_commit_message();
        let mut parsed = Op::from_commit_message(&msg, "deadbeef").unwrap();
        // Provenance is not part of the serialized op; normalise for equality.
        parsed.commit = None;
        parsed
    }

    #[test]
    fn roundtrip_every_op_kind() {
        let kinds = vec![
            OpKind::Create {
                title: "Title: with colon".into(),
                description: "Line one\n\nA paragraph that has\nNote: a trailer-looking line"
                    .into(),
                labels: vec!["bug".into(), "p1".into()],
                assignee: Some("bob".into()),
                priority: Some("high".into()),
            },
            OpKind::Comment {
                text: "multi\nline\ncomment".into(),
            },
            OpKind::SetTitle {
                title: "new title".into(),
            },
            OpKind::SetDescription {
                description: "new\ndesc".into(),
            },
            OpKind::SetState {
                state: State::Closed,
                reason: Some("fixed".into()),
                fixed_by: Some("abc123".into()),
            },
            OpKind::SetAssignee {
                assignee: Some("carol".into()),
            },
            OpKind::SetAssignee { assignee: None },
            OpKind::SetPriority {
                priority: Some("low".into()),
            },
            OpKind::AddLabel {
                label: "triage".into(),
            },
            OpKind::RemoveLabel {
                label: "wontfix".into(),
            },
            OpKind::SetArchived { archived: true },
            OpKind::SetArchived { archived: false },
        ];
        for (i, kind) in kinds.into_iter().enumerate() {
            let original = op(&format!("op{i}"), (i + 1) as u64, "alice", kind);
            assert_eq!(roundtrip(&original), original, "op #{i} did not round-trip");
        }
    }

    #[test]
    fn archived_is_lww() {
        // Default not archived; archive then restore folds to not-archived; a
        // higher-lamport archive wins regardless of fold order.
        let base = vec![create("c", 1, "t")];
        assert!(!Issue::fold("u", &base).unwrap().archived());

        let archived = vec![
            create("c", 1, "t"),
            op("a1", 2, "alice", OpKind::SetArchived { archived: true }),
        ];
        assert!(Issue::fold("u", &archived).unwrap().archived());

        let restored = vec![
            create("c", 1, "t"),
            op("a2", 3, "alice", OpKind::SetArchived { archived: false }),
            op("a1", 2, "alice", OpKind::SetArchived { archived: true }),
        ];
        assert!(!Issue::fold("u", &restored).unwrap().archived());
    }

    #[test]
    fn parser_ignores_unknown_trailers_and_higher_version() {
        let msg = "Comment\n\nhello world\n\n\
                   Op: comment\n\
                   Op-Id: abc\n\
                   Lamport: 9\n\
                   Actor: alice\n\
                   Format-Version: 99\n\
                   X-Custom: something new\n\
                   Reactions: 3\n";
        let parsed = Op::from_commit_message(msg, "sha").unwrap();
        assert_eq!(parsed.lamport, 9);
        assert_eq!(parsed.format_version, 99);
        assert!(matches!(parsed.kind, OpKind::Comment { text } if text == "hello world"));
    }
}

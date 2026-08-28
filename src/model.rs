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
    // --- optional metadata layer (all unenforced) ---
    /// Add an assignee (OR-Set; issues support multiple assignees).
    AddAssignee {
        assignee: String,
    },
    /// Remove an assignee (OR-Set).
    RemoveAssignee {
        assignee: String,
    },
    /// Set or clear a custom field (LWW-Register per key; empty value clears).
    /// One generic mechanism for milestone, type, severity, due-date, etc.
    SetField {
        key: String,
        value: String,
    },
    /// Attach a related file path with an optional note (OR-Set of paths plus
    /// an LWW note per path).
    AddFile {
        path: String,
        note: String,
    },
    /// Detach a related file path (OR-Set remove).
    RemoveFile {
        path: String,
    },
    /// Edit the note on an already-attached file path (LWW note per path).
    SetFileNote {
        path: String,
        note: String,
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
            OpKind::AddAssignee { .. } => "add-assignee",
            OpKind::RemoveAssignee { .. } => "remove-assignee",
            OpKind::SetField { .. } => "set-field",
            OpKind::AddFile { .. } => "add-file",
            OpKind::RemoveFile { .. } => "remove-file",
            OpKind::SetFileNote { .. } => "set-file-note",
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
            OpKind::AddAssignee { assignee } => format!("Add assignee: {assignee}"),
            OpKind::RemoveAssignee { assignee } => format!("Remove assignee: {assignee}"),
            OpKind::SetField { key, value } => {
                if value.is_empty() {
                    format!("Clear field: {key}")
                } else {
                    format!("Set field: {key}")
                }
            }
            OpKind::AddFile { path, .. } => format!("Add file: {path}"),
            OpKind::RemoveFile { path } => format!("Remove file: {path}"),
            OpKind::SetFileNote { path, .. } => format!("Set file note: {path}"),
        }
    }

    /// The free-form body (multi-line payload) for ops that carry one.
    fn body(&self) -> Option<&str> {
        match &self.kind {
            OpKind::Create { description, .. } if !description.is_empty() => Some(description),
            OpKind::Comment { text } => Some(text),
            OpKind::SetDescription { description } => Some(description),
            OpKind::AddFile { note, .. } if !note.is_empty() => Some(note),
            OpKind::SetFileNote { note, .. } if !note.is_empty() => Some(note),
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
            OpKind::AddAssignee { assignee } | OpKind::RemoveAssignee { assignee } => {
                t.push("Assignee", assignee)
            }
            OpKind::SetField { key, value } => {
                t.push("Field-Key", key);
                t.push("Field-Value", value);
            }
            OpKind::AddFile { path, .. }
            | OpKind::RemoveFile { path }
            | OpKind::SetFileNote { path, .. } => t.push("Path", path),
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
            "add-assignee" => OpKind::AddAssignee {
                assignee: get("Assignee")
                    .ok_or_else(|| malformed("add-assignee has no Assignee trailer"))?
                    .to_string(),
            },
            "remove-assignee" => OpKind::RemoveAssignee {
                assignee: get("Assignee")
                    .ok_or_else(|| malformed("remove-assignee has no Assignee trailer"))?
                    .to_string(),
            },
            "set-field" => OpKind::SetField {
                key: get("Field-Key")
                    .ok_or_else(|| malformed("set-field has no Field-Key trailer"))?
                    .to_string(),
                value: get("Field-Value").unwrap_or("").to_string(),
            },
            "add-file" => OpKind::AddFile {
                path: get("Path")
                    .ok_or_else(|| malformed("add-file has no Path trailer"))?
                    .to_string(),
                note: body.clone(),
            },
            "remove-file" => OpKind::RemoveFile {
                path: get("Path")
                    .ok_or_else(|| malformed("remove-file has no Path trailer"))?
                    .to_string(),
            },
            "set-file-note" => OpKind::SetFileNote {
                path: get("Path")
                    .ok_or_else(|| malformed("set-file-note has no Path trailer"))?
                    .to_string(),
                note: body.clone(),
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

    /// Remove every element (a linear-chain clear, used for legacy
    /// single-assignee `set-assignee None`). Concurrent-merge nuance is deferred
    /// with sync, matching label removes.
    pub fn clear(&mut self) {
        self.tags.clear();
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

/// A map of keys to LWW-Register string values. Used for custom fields (key =
/// field name) and file notes (key = path). An empty value marks the key
/// cleared, so it is filtered from [`entries`](Self::entries).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LwwMap {
    inner: BTreeMap<String, Lww<String>>,
}

impl LwwMap {
    pub fn apply(&mut self, key: &str, value: String, lamport: u64, op_id: &str) {
        self.inner
            .entry(key.to_string())
            .or_default()
            .apply(value, lamport, op_id);
    }

    /// The current value for a key, or `None` if unset or cleared (empty).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.inner
            .get(key)
            .filter(|l| l.is_set() && !l.get().is_empty())
            .map(|l| l.get().as_str())
    }

    /// All present (non-empty) key/value pairs, sorted by key.
    pub fn entries(&self) -> Vec<(String, String)> {
        self.inner
            .iter()
            .filter(|(_, l)| l.is_set() && !l.get().is_empty())
            .map(|(k, l)| (k.clone(), l.get().clone()))
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
    /// Assignees, an OR-Set (issues can have several). Legacy single-assignee
    /// ops fold into this set.
    pub assignees: OrSet,
    pub priority: Lww<Option<String>>,
    pub labels: OrSet,
    pub comments: Vec<Comment>,
    /// Whether the issue is archived (hidden from default listings).
    pub archived: Lww<bool>,
    /// Custom fields (key -> LWW value): milestone, type, severity, etc.
    pub fields: LwwMap,
    /// Related file paths (OR-Set presence).
    pub files: OrSet,
    /// Optional note per related file path (LWW).
    pub file_notes: LwwMap,
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
            assignees: OrSet::default(),
            priority: Lww::default(),
            labels: OrSet::default(),
            comments: Vec::new(),
            archived: Lww::default(),
            fields: LwwMap::default(),
            files: OrSet::default(),
            file_notes: LwwMap::default(),
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
                        issue.assignees.add(a, &op.id);
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
                OpKind::SetAssignee { assignee } => match assignee {
                    // Legacy single-assignee: Some seeds the set, None clears it.
                    Some(a) => issue.assignees.add(a, &op.id),
                    None => issue.assignees.clear(),
                },
                OpKind::SetPriority { priority } => {
                    issue.priority.apply(priority.clone(), op.lamport, &op.id)
                }
                OpKind::AddLabel { label } => issue.labels.add(label, &op.id),
                OpKind::RemoveLabel { label } => issue.labels.remove(label),
                OpKind::SetArchived { archived } => {
                    issue.archived.apply(*archived, op.lamport, &op.id)
                }
                OpKind::AddAssignee { assignee } => issue.assignees.add(assignee, &op.id),
                OpKind::RemoveAssignee { assignee } => issue.assignees.remove(assignee),
                OpKind::SetField { key, value } => {
                    issue.fields.apply(key, value.clone(), op.lamport, &op.id)
                }
                OpKind::AddFile { path, note } => {
                    issue.files.add(path, &op.id);
                    if !note.is_empty() {
                        issue
                            .file_notes
                            .apply(path, note.clone(), op.lamport, &op.id);
                    }
                }
                OpKind::RemoveFile { path } => issue.files.remove(path),
                OpKind::SetFileNote { path, note } => {
                    issue
                        .file_notes
                        .apply(path, note.clone(), op.lamport, &op.id)
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

    /// The first assignee, for compact/legacy single-assignee display.
    pub fn assignee(&self) -> Option<String> {
        self.assignees.values().into_iter().next()
    }

    /// All assignees (sorted).
    pub fn assignees(&self) -> Vec<String> {
        self.assignees.values()
    }

    /// Present custom fields as sorted (key, value) pairs.
    pub fn fields(&self) -> Vec<(String, String)> {
        self.fields.entries()
    }

    /// Related files as (path, optional note), sorted by path.
    pub fn files(&self) -> Vec<(String, Option<String>)> {
        self.files
            .values()
            .into_iter()
            .map(|p| {
                let note = self.file_notes.get(&p).map(str::to_string);
                (p, note)
            })
            .collect()
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
        assert_eq!(issue.assignee().as_deref(), Some("bob"));
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
            OpKind::AddAssignee {
                assignee: "bob".into(),
            },
            OpKind::RemoveAssignee {
                assignee: "carol".into(),
            },
            OpKind::SetField {
                key: "milestone".into(),
                value: "v0.2".into(),
            },
            OpKind::SetField {
                key: "severity".into(),
                value: "".into(),
            },
            OpKind::AddFile {
                path: "src/parser.rs".into(),
                note: "the buggy function is here\nsee line 40".into(),
            },
            OpKind::RemoveFile {
                path: "docs/old.md".into(),
            },
            OpKind::SetFileNote {
                path: "src/parser.rs".into(),
                note: "updated note".into(),
            },
        ];
        for (i, kind) in kinds.into_iter().enumerate() {
            let original = op(&format!("op{i}"), (i + 1) as u64, "alice", kind);
            assert_eq!(roundtrip(&original), original, "op #{i} did not round-trip");
        }
    }

    #[test]
    fn assignees_are_a_set() {
        let ops = vec![
            create("c", 1, "t"),
            op(
                "a1",
                2,
                "alice",
                OpKind::AddAssignee {
                    assignee: "bob".into(),
                },
            ),
            op(
                "a2",
                3,
                "alice",
                OpKind::AddAssignee {
                    assignee: "carol".into(),
                },
            ),
            op(
                "a3",
                4,
                "alice",
                OpKind::RemoveAssignee {
                    assignee: "bob".into(),
                },
            ),
        ];
        let issue = Issue::fold("u", &ops).unwrap();
        assert_eq!(issue.assignees(), vec!["carol".to_string()]);
    }

    #[test]
    fn custom_fields_are_lww_and_clearable() {
        let ops = vec![
            create("c", 1, "t"),
            op(
                "f1",
                2,
                "alice",
                OpKind::SetField {
                    key: "milestone".into(),
                    value: "v0.1".into(),
                },
            ),
            op(
                "f2",
                3,
                "alice",
                OpKind::SetField {
                    key: "milestone".into(),
                    value: "v0.2".into(),
                },
            ),
            op(
                "f3",
                4,
                "alice",
                OpKind::SetField {
                    key: "severity".into(),
                    value: "high".into(),
                },
            ),
            op(
                "f4",
                5,
                "alice",
                OpKind::SetField {
                    key: "severity".into(),
                    value: "".into(),
                },
            ),
        ];
        let issue = Issue::fold("u", &ops).unwrap();
        assert_eq!(
            issue.fields(),
            vec![("milestone".to_string(), "v0.2".to_string())]
        );
    }

    #[test]
    fn files_carry_editable_notes() {
        let ops = vec![
            create("c", 1, "t"),
            op(
                "x1",
                2,
                "alice",
                OpKind::AddFile {
                    path: "a.rs".into(),
                    note: "first".into(),
                },
            ),
            op(
                "x2",
                3,
                "alice",
                OpKind::SetFileNote {
                    path: "a.rs".into(),
                    note: "edited".into(),
                },
            ),
        ];
        let issue = Issue::fold("u", &ops).unwrap();
        assert_eq!(
            issue.files(),
            vec![("a.rs".to_string(), Some("edited".to_string()))]
        );
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

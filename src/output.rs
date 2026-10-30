//! Stable machine-readable output (JSON).
//!
//! These DTOs are the public `--format json` schema. They are deliberately
//! separate from the internal [`crate::model::Issue`] so the wire format can
//! stay stable while the model evolves. Fields are plain and self-describing.

use crate::cache::Cache;
use crate::model::Issue;
use serde::Serialize;

/// JSON DTO for a single issue in `--format json` output.
#[derive(Serialize)]
pub struct IssueJson {
    /// Full UUIDv7 (truth id).
    pub uuid: String,
    /// Display handle: `#N` (or `myproject-#N` with a display prefix set), or
    /// `alice-#N` when a number is shared across actors (may drift; not a
    /// permanent handle).
    pub handle: String,
    /// Per-actor display number (the `N` in the handle; may drift).
    pub number: usize,
    /// Actor-scoped nonce, e.g. `alice-1` (the typeable actor-qualified form).
    pub nonce: String,
    /// Short uuid prefix (a permanent handle).
    pub short: String,
    /// The issue title.
    pub title: String,
    /// The issue description (body text).
    pub description: String,
    /// "open" or "closed".
    pub state: String,
    /// Optional reason recorded with the current state (for example a close reason).
    pub reason: Option<String>,
    /// Optional reference to what fixed the issue (recorded with the state).
    pub fixed_by: Option<String>,
    /// First assignee, kept for compact/legacy consumers.
    pub assignee: Option<String>,
    /// All assignees.
    pub assignees: Vec<String>,
    /// Priority label, if set.
    pub priority: Option<String>,
    /// Labels on the issue.
    pub labels: Vec<String>,
    /// Custom fields as (key, value) pairs.
    pub fields: Vec<(String, String)>,
    /// Related files with optional notes.
    pub files: Vec<FileJson>,
    /// Whether the issue is archived (hidden from default listings).
    pub archived: bool,
    /// Actor slug that created the issue.
    pub creator: String,
    /// Number of comments on the issue.
    pub comment_count: usize,
    /// The issue comments.
    pub comments: Vec<CommentJson>,
}

/// JSON DTO for a related file entry on an issue.
#[derive(Serialize)]
pub struct FileJson {
    /// The file path.
    pub path: String,
    /// Optional note attached to the file.
    pub note: Option<String>,
}

/// JSON DTO for a single comment on an issue.
#[derive(Serialize)]
pub struct CommentJson {
    /// The comment id (its op-id).
    pub id: String,
    /// Actor slug that authored the comment.
    pub actor: String,
    /// Optional human author name.
    pub author: Option<String>,
    /// The comment text.
    pub text: String,
    /// True when the comment has been edited.
    pub edited: bool,
    /// True when the comment has been hidden (soft-deleted).
    pub hidden: bool,
}

/// Build the JSON DTO for one issue.
pub fn issue_json(cache: &Cache, issue: &Issue) -> IssueJson {
    let d = cache.display_of(&issue.uuid);
    let st = issue.state();
    IssueJson {
        uuid: issue.uuid.clone(),
        handle: d.handle,
        number: d.number,
        nonce: d.nonce,
        short: d.short,
        title: issue.title().to_string(),
        description: issue.description().to_string(),
        state: st.state.as_str().to_string(),
        reason: st.reason.clone(),
        fixed_by: st.fixed_by.clone(),
        assignee: issue.assignee(),
        assignees: issue.assignees(),
        fields: issue.fields(),
        files: issue
            .files()
            .into_iter()
            .map(|(path, note)| FileJson { path, note })
            .collect(),
        priority: issue.priority().map(str::to_string),
        labels: issue.labels(),
        archived: issue.archived(),
        creator: issue.creator.clone(),
        comment_count: issue.comments.len(),
        comments: issue
            .comments
            .iter()
            .map(|c| CommentJson {
                id: c.id.clone(),
                actor: c.actor.clone(),
                author: c.author.clone(),
                text: c.text.clone(),
                edited: c.edited,
                hidden: c.hidden,
            })
            .collect(),
    }
}

/// Render a list of issues as a pretty JSON array.
pub fn render_issues(cache: &Cache, issues: &[&Issue]) -> String {
    let dtos: Vec<IssueJson> = issues.iter().map(|i| issue_json(cache, i)).collect();
    serde_json::to_string_pretty(&dtos).unwrap_or_else(|_| "[]".to_string())
}

/// Render a single issue as pretty JSON.
pub fn render_issue(cache: &Cache, issue: &Issue) -> String {
    serde_json::to_string_pretty(&issue_json(cache, issue)).unwrap_or_else(|_| "{}".to_string())
}

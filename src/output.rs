//! Stable machine-readable output (JSON).
//!
//! These DTOs are the public `--format json` schema. They are deliberately
//! separate from the internal [`crate::model::Issue`] so the wire format can
//! stay stable while the model evolves. Fields are plain and self-describing.

use crate::cache::Cache;
use crate::model::Issue;
use serde::Serialize;

#[derive(Serialize)]
pub struct IssueJson {
    /// Full UUIDv7 (truth id).
    pub uuid: String,
    /// Actor-scoped display nonce (may drift; not a permanent handle).
    pub nonce: String,
    /// Short uuid prefix (a permanent handle).
    pub short: String,
    pub title: String,
    pub description: String,
    /// "open" or "closed".
    pub state: String,
    pub reason: Option<String>,
    pub fixed_by: Option<String>,
    pub assignee: Option<String>,
    pub priority: Option<String>,
    pub labels: Vec<String>,
    pub archived: bool,
    pub creator: String,
    pub comment_count: usize,
    pub comments: Vec<CommentJson>,
}

#[derive(Serialize)]
pub struct CommentJson {
    pub id: String,
    pub actor: String,
    pub author: Option<String>,
    pub text: String,
}

/// Build the JSON DTO for one issue.
pub fn issue_json(cache: &Cache, issue: &Issue) -> IssueJson {
    let d = cache.display_of(&issue.uuid);
    let st = issue.state();
    IssueJson {
        uuid: issue.uuid.clone(),
        nonce: d.nonce,
        short: d.short,
        title: issue.title().to_string(),
        description: issue.description().to_string(),
        state: st.state.as_str().to_string(),
        reason: st.reason.clone(),
        fixed_by: st.fixed_by.clone(),
        assignee: issue.assignee().map(str::to_string),
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

//! Query model for `gli ls`: the filter and sort criteria, and the pure
//! selection over a set of issues.
//!
//! The CLI builds an [`LsFilter`] from flags and config defaults, then calls
//! [`select`]. Keeping the matching and ordering here, separate from the
//! command handler, means the logic is unit-testable without driving the binary
//! and the handler stays a thin adapter.

use crate::model::{Issue, State};

/// How `ls` selects and orders issues.
#[derive(Debug, Default, Clone)]
pub struct LsFilter {
    /// Keep only issues in this state; `None` keeps both.
    pub state: Option<State>,
    /// Every label here must be present (AND).
    pub labels: Vec<String>,
    /// Keep issues with this assignee among their assignees.
    pub assignee: Option<String>,
    /// Keep issues with exactly this priority.
    pub priority: Option<String>,
    /// Keep issues created by this actor.
    pub creator: Option<String>,
    /// Free-text search term, already lowercased by the caller.
    pub search: Option<String>,
    /// Show only archived issues.
    pub archived: bool,
    /// Show archived and non-archived issues together.
    pub all: bool,
    /// Result ordering.
    pub sort: Sort,
}

/// Result ordering for `ls`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    /// Newest first (UUIDv7 is time-ordered, so reverse-uuid order).
    #[default]
    Newest,
    /// Oldest first.
    Oldest,
    /// By title, case-insensitive.
    Title,
}

impl Sort {
    /// Parse a `--sort` value; an unknown value falls back to newest.
    pub fn parse(s: &str) -> Sort {
        match s {
            "oldest" => Sort::Oldest,
            "title" => Sort::Title,
            _ => Sort::Newest,
        }
    }
}

/// Apply `filter` to `issues`, returning the matching issues in the requested
/// order. The issues are borrowed; the caller owns them.
pub fn select<'a>(issues: &'a [Issue], filter: &LsFilter) -> Vec<&'a Issue> {
    let mut rows: Vec<&Issue> = issues
        .iter()
        .filter(|i| visible(i, filter))
        .filter(|i| match &filter.state {
            Some(want) => &i.state().state == want,
            None => true,
        })
        // Every requested label must be present (AND).
        .filter(|i| filter.labels.iter().all(|l| i.labels.contains(l)))
        .filter(|i| match &filter.assignee {
            Some(a) => i.assignees().iter().any(|x| x == a),
            None => true,
        })
        .filter(|i| match &filter.priority {
            Some(p) => i.priority() == Some(p.as_str()),
            None => true,
        })
        .filter(|i| match &filter.creator {
            Some(c) => &i.creator == c,
            None => true,
        })
        .filter(|i| match &filter.search {
            Some(term) => matches_search(i, term),
            None => true,
        })
        .collect();

    match filter.sort {
        Sort::Oldest => rows.sort_by(|a, b| a.uuid.cmp(&b.uuid)),
        Sort::Title => rows.sort_by_key(|a| a.title().to_lowercase()),
        Sort::Newest => rows.sort_by(|a, b| b.uuid.cmp(&a.uuid)),
    }
    rows
}

/// Archived visibility: hidden by default, only-archived with `archived`, both
/// with `all` (which wins over `archived`).
fn visible(issue: &Issue, filter: &LsFilter) -> bool {
    if filter.all {
        true
    } else if filter.archived {
        issue.archived()
    } else {
        !issue.archived()
    }
}

/// Case-insensitive substring match over an issue's title, description, labels,
/// and comment text. `term` must already be lowercased.
pub fn matches_search(issue: &Issue, term: &str) -> bool {
    issue.title().to_lowercase().contains(term)
        || issue.description().to_lowercase().contains(term)
        || issue
            .labels()
            .iter()
            .any(|l| l.to_lowercase().contains(term))
        || issue
            .comments
            .iter()
            .any(|c| c.text.to_lowercase().contains(term))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Op, OpKind};

    /// Fold an issue from a Create plus any extra ops, with ascending op-ids
    /// and Lamport clocks and a given creator.
    fn issue(uuid: &str, creator: &str, kinds: Vec<OpKind>) -> Issue {
        let ops: Vec<Op> = kinds
            .into_iter()
            .enumerate()
            .map(|(i, k)| {
                Op::new(
                    format!("{uuid}-op{i}"),
                    (i + 1) as u64,
                    creator.to_string(),
                    k,
                )
            })
            .collect();
        Issue::fold(uuid, &ops).unwrap()
    }

    fn create(
        title: &str,
        desc: &str,
        labels: &[&str],
        assignee: Option<&str>,
        priority: Option<&str>,
    ) -> OpKind {
        OpKind::Create {
            title: title.into(),
            description: desc.into(),
            labels: labels.iter().map(|s| s.to_string()).collect(),
            assignee: assignee.map(str::to_string),
            priority: priority.map(str::to_string),
        }
    }

    fn sample() -> Vec<Issue> {
        vec![
            // #1 alice, open, [bug, urgent], assignee alice, high
            issue(
                "018f0000000000000000000000000001",
                "alice",
                vec![create(
                    "Fix login bug",
                    "the session drops",
                    &["bug", "urgent"],
                    Some("alice"),
                    Some("high"),
                )],
            ),
            // #2 alice, closed, [docs]
            issue(
                "018f0000000000000000000000000002",
                "alice",
                vec![
                    create("Add docs", "write the README", &["docs"], None, Some("low")),
                    OpKind::SetState {
                        state: State::Closed,
                        reason: None,
                        fixed_by: None,
                    },
                ],
            ),
            // #3 bob, open, [bug], archived, with a comment
            issue(
                "018f0000000000000000000000000003",
                "bob",
                vec![
                    create("Refactor cache", "", &["bug"], Some("bob"), None),
                    OpKind::Comment {
                        text: "the fold is slow".into(),
                    },
                    OpKind::SetArchived { archived: true },
                ],
            ),
        ]
    }

    fn uuids(rows: &[&Issue]) -> Vec<String> {
        rows.iter().map(|i| i.uuid.clone()).collect()
    }

    #[test]
    fn default_hides_archived() {
        let issues = sample();
        let rows = select(&issues, &LsFilter::default());
        // #3 is archived, so it is hidden by default.
        assert_eq!(rows.len(), 2);
        assert!(!uuids(&rows).contains(&"018f0000000000000000000000000003".to_string()));
    }

    #[test]
    fn archived_and_all_flags() {
        let issues = sample();
        let only = select(
            &issues,
            &LsFilter {
                archived: true,
                ..Default::default()
            },
        );
        assert_eq!(uuids(&only), vec!["018f0000000000000000000000000003"]);
        let all = select(
            &issues,
            &LsFilter {
                all: true,
                ..Default::default()
            },
        );
        assert_eq!(all.len(), 3);
        // `all` wins over `archived`.
        let both = select(
            &issues,
            &LsFilter {
                all: true,
                archived: true,
                ..Default::default()
            },
        );
        assert_eq!(both.len(), 3);
    }

    #[test]
    fn state_label_and_creator_filters() {
        let issues = sample();
        let closed = select(
            &issues,
            &LsFilter {
                state: Some(State::Closed),
                all: true,
                ..Default::default()
            },
        );
        assert_eq!(uuids(&closed), vec!["018f0000000000000000000000000002"]);
        // label AND: bug must be present; across all visibility.
        let bugs = select(
            &issues,
            &LsFilter {
                labels: vec!["bug".into()],
                all: true,
                ..Default::default()
            },
        );
        assert_eq!(bugs.len(), 2); // #1 and #3
        // two labels, both required.
        let urgent_bugs = select(
            &issues,
            &LsFilter {
                labels: vec!["bug".into(), "urgent".into()],
                all: true,
                ..Default::default()
            },
        );
        assert_eq!(
            uuids(&urgent_bugs),
            vec!["018f0000000000000000000000000001"]
        );
        let by_bob = select(
            &issues,
            &LsFilter {
                creator: Some("bob".into()),
                all: true,
                ..Default::default()
            },
        );
        assert_eq!(uuids(&by_bob), vec!["018f0000000000000000000000000003"]);
    }

    #[test]
    fn assignee_and_priority_filters() {
        let issues = sample();
        let alices = select(
            &issues,
            &LsFilter {
                assignee: Some("alice".into()),
                all: true,
                ..Default::default()
            },
        );
        assert_eq!(uuids(&alices), vec!["018f0000000000000000000000000001"]);
        let high = select(
            &issues,
            &LsFilter {
                priority: Some("high".into()),
                all: true,
                ..Default::default()
            },
        );
        assert_eq!(uuids(&high), vec!["018f0000000000000000000000000001"]);
    }

    #[test]
    fn search_covers_title_description_labels_and_comments() {
        let issues = sample();
        let want = |term: &str| {
            select(
                &issues,
                &LsFilter {
                    search: Some(term.into()),
                    all: true,
                    ..Default::default()
                },
            )
        };
        // title
        assert_eq!(
            uuids(&want("login")),
            vec!["018f0000000000000000000000000001"]
        );
        // description
        assert_eq!(
            uuids(&want("readme")),
            vec!["018f0000000000000000000000000002"]
        );
        // label
        assert_eq!(
            uuids(&want("docs")),
            vec!["018f0000000000000000000000000002"]
        );
        // comment text, and only #3 has it
        assert_eq!(
            uuids(&want("fold is slow")),
            vec!["018f0000000000000000000000000003"]
        );
        // no match
        assert!(want("nonexistent-term").is_empty());
    }

    #[test]
    fn sort_orders() {
        let issues = sample();
        let newest = select(
            &issues,
            &LsFilter {
                all: true,
                sort: Sort::Newest,
                ..Default::default()
            },
        );
        assert_eq!(
            uuids(&newest),
            vec![
                "018f0000000000000000000000000003",
                "018f0000000000000000000000000002",
                "018f0000000000000000000000000001",
            ]
        );
        let oldest = select(
            &issues,
            &LsFilter {
                all: true,
                sort: Sort::Oldest,
                ..Default::default()
            },
        );
        assert_eq!(oldest[0].uuid, "018f0000000000000000000000000001");
        let by_title = select(
            &issues,
            &LsFilter {
                all: true,
                sort: Sort::Title,
                ..Default::default()
            },
        );
        // "Add docs" < "Fix login bug" < "Refactor cache"
        assert_eq!(by_title[0].title(), "Add docs");
    }

    #[test]
    fn sort_parse_falls_back_to_newest() {
        assert_eq!(Sort::parse("oldest"), Sort::Oldest);
        assert_eq!(Sort::parse("title"), Sort::Title);
        assert_eq!(Sort::parse("newest"), Sort::Newest);
        assert_eq!(Sort::parse("whatever"), Sort::Newest);
    }
}

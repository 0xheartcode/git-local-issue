//! Identity: truth ids, op ids, actor slugs, and drift-allowed display numbers.
//!
//! Truth identity is a UUIDv7 (time-sortable, never collides): the issue ref is
//! `refs/issues/<uuid>` and a short prefix is a permanent handle. Display
//! identity is an actor-scoped nonce (`alice-1`, `bob-1`) computed on read, so
//! two actors never clash and a same-actor clash (two offline clones) resolves
//! deterministically by UUIDv7 order. Display numbers may drift after a sync;
//! they are convenience, not permanent handles.

use crate::model::Issue;
use std::collections::HashMap;
use uuid::Uuid;

/// A new UUIDv7 as lowercase hex, no dashes (used for issue ids and op ids).
pub fn new_uuid() -> String {
    Uuid::now_v7().simple().to_string()
}

/// Sanitize a raw identity (git user.name or the local-part of user.email) into
/// a display-safe actor slug: lowercase, ascii alphanumerics and hyphens.
pub fn actor_slug(raw: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for c in raw.trim().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "anon".to_string()
    } else {
        slug
    }
}

/// A resolved display handle for one issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayId {
    pub uuid: String,
    /// Actor-scoped nonce, e.g. `alice-3`.
    pub nonce: String,
    /// Short uuid prefix, always a valid permanent handle.
    pub short: String,
}

/// Compute display ids for every issue.
///
/// For each creating actor, its issues are sorted by UUIDv7 (time order) and
/// numbered 1..N. This is exactly the deterministic collision resolution: if an
/// actor minted the same number from two offline clones, the earlier UUIDv7
/// keeps the lower number and the later drifts to the next slot. Numbers may
/// change after a sync (drift-allowed) and that is fine.
pub fn assign_display_ids(issues: &[Issue]) -> HashMap<String, DisplayId> {
    let mut by_actor: HashMap<String, Vec<&Issue>> = HashMap::new();
    for issue in issues {
        by_actor
            .entry(issue.creator.clone())
            .or_default()
            .push(issue);
    }

    let mut out = HashMap::new();
    for (actor, mut group) in by_actor {
        // UUIDv7 hex is lexicographically time-ordered, so a string sort is a
        // timestamp sort.
        group.sort_by(|a, b| a.uuid.cmp(&b.uuid));
        for (i, issue) in group.iter().enumerate() {
            out.insert(
                issue.uuid.clone(),
                DisplayId {
                    uuid: issue.uuid.clone(),
                    nonce: format!("{actor}-{}", i + 1),
                    short: short_prefix(&issue.uuid),
                },
            );
        }
    }
    out
}

/// The shortest conventional handle prefix (8 hex chars, or the whole id).
pub fn short_prefix(uuid: &str) -> String {
    uuid.chars().take(8).collect()
}

/// The outcome of resolving a user-supplied id query.
pub enum Resolution {
    /// Exactly one issue matched; here is its uuid.
    Unique(String),
    /// No issue matched.
    None,
    /// Several issues matched a uuid prefix.
    Ambiguous(Vec<String>),
}

/// Resolve an id query (actor-nonce like `alice-3`, or a uuid prefix) against
/// the known issues and their display ids.
pub fn resolve(query: &str, display: &HashMap<String, DisplayId>) -> Resolution {
    let ql = query.trim().to_ascii_lowercase();

    // 1. Exact actor-nonce match (nonces are always lowercase, so compare the
    //    lowercased query so `ALICE-1` resolves like `alice-1`).
    for d in display.values() {
        if d.nonce == ql {
            return Resolution::Unique(d.uuid.clone());
        }
    }

    // 2. Exact full-uuid match.
    if display.contains_key(&ql) {
        return Resolution::Unique(ql);
    }

    // 3. UUIDv7 hex-prefix match (only if the query looks like hex).
    if !ql.is_empty() && ql.chars().all(|c| c.is_ascii_hexdigit()) {
        let mut matches: Vec<String> = display
            .keys()
            .filter(|uuid| uuid.starts_with(&ql))
            .cloned()
            .collect();
        matches.sort();
        return match matches.len() {
            0 => Resolution::None,
            1 => Resolution::Unique(matches.into_iter().next().unwrap()),
            _ => Resolution::Ambiguous(matches),
        };
    }

    Resolution::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Issue, Op, OpKind};

    fn issue_with(uuid: &str, creator: &str) -> Issue {
        // Fold a minimal Create op so the issue's creator is set correctly.
        let op = Op::new(
            "op".into(),
            1,
            creator.to_string(),
            OpKind::Create {
                title: "t".into(),
                description: String::new(),
                labels: Vec::new(),
                assignee: None,
                priority: None,
            },
        );
        Issue::fold(uuid, &[op]).unwrap()
    }

    #[test]
    fn actor_slug_sanitizes() {
        assert_eq!(actor_slug("Alice"), "alice");
        assert_eq!(actor_slug("Ada Lovelace"), "ada-lovelace");
        assert_eq!(actor_slug("  weird__name!! "), "weird-name");
        assert_eq!(actor_slug("@@@"), "anon");
        assert_eq!(actor_slug(""), "anon");
    }

    #[test]
    fn display_numbers_are_per_actor_and_time_ordered() {
        // Two actors; each numbered independently starting at 1, by uuid order.
        let issues = vec![
            issue_with("018f0000000000000000000000000002", "alice"),
            issue_with("018f0000000000000000000000000001", "alice"),
            issue_with("018f0000000000000000000000000009", "bob"),
        ];
        let d = assign_display_ids(&issues);
        // alice's earlier uuid (...0001) keeps -1; the later (...0002) is -2.
        assert_eq!(d["018f0000000000000000000000000001"].nonce, "alice-1");
        assert_eq!(d["018f0000000000000000000000000002"].nonce, "alice-2");
        assert_eq!(d["018f0000000000000000000000000009"].nonce, "bob-1");
    }

    #[test]
    fn short_prefix_is_first_eight_hex() {
        assert_eq!(short_prefix("0123456789abcdef0123456789abcdef"), "01234567");
        // Shorter-than-eight ids return the whole string.
        assert_eq!(short_prefix("abc"), "abc");
    }

    #[test]
    fn resolve_empty_query_matches_nothing() {
        // The empty string is neither a nonce nor a hex prefix. Without the
        // non-empty guard it would `starts_with("")`-match every uuid, so this
        // pins that guard (a `&&` -> `||` mutation resurfaces as an ambiguity).
        let issues = vec![
            issue_with("018f0000000000000000000000000001", "alice"),
            issue_with("018f0000000000000000000000000002", "bob"),
        ];
        let d = assign_display_ids(&issues);
        assert!(matches!(resolve("", &d), Resolution::None));
        assert!(matches!(resolve("   ", &d), Resolution::None));
    }

    #[test]
    fn same_actor_collision_resolves_by_uuid_order() {
        // The scenario the plan calls out: the same actor minted "alice-1" from
        // two offline clones. The earlier UUIDv7 keeps the number; the later
        // deterministically drifts to the next slot.
        let earlier = "018f0000000000000000000000000010";
        let later = "018f0000000000000000000000000011";
        let issues = vec![issue_with(later, "alice"), issue_with(earlier, "alice")];
        let d = assign_display_ids(&issues);
        assert_eq!(d[earlier].nonce, "alice-1");
        assert_eq!(d[later].nonce, "alice-2");
    }

    #[test]
    fn resolve_matches_nonce_prefix_and_reports_ambiguity() {
        let issues = vec![
            issue_with("018f0000000000000000000000000001", "alice"),
            issue_with("018fabcdef00000000000000000000aa", "bob"),
            issue_with("018fabcdef00000000000000000000bb", "bob"),
        ];
        let d = assign_display_ids(&issues);

        // Actor-nonce.
        assert!(
            matches!(resolve("alice-1", &d), Resolution::Unique(u) if u == "018f0000000000000000000000000001")
        );
        // Nonce resolution is case-insensitive.
        assert!(matches!(resolve("ALICE-1", &d), Resolution::Unique(_)));
        // Unambiguous uuid prefix.
        assert!(matches!(resolve("018f00000", &d), Resolution::Unique(_)));
        // Ambiguous prefix -> both bob issues.
        match resolve("018fabcdef", &d) {
            Resolution::Ambiguous(c) => assert_eq!(c.len(), 2),
            other => panic!(
                "expected ambiguous, got {:?}",
                matches!(other, Resolution::None)
            ),
        }
        // No match.
        assert!(matches!(resolve("ffffffff", &d), Resolution::None));
        assert!(matches!(resolve("nobody-3", &d), Resolution::None));
    }
}

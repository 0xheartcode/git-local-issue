//! Identity: truth ids, op ids, actor slugs, and drift-allowed display numbers.
//!
//! Truth identity is a UUIDv7 (time-sortable, never collides): the issue ref is
//! `refs/issues/<uuid>` and a short prefix is a permanent handle. Display
//! identity is a number computed on read, shown as a bare `#N` while that
//! number is unambiguous and as the actor-qualified `alice-#N` only once two
//! actors share a number (after a sync). A same-actor clash (two offline
//! clones) resolves deterministically by UUIDv7 order. Display numbers may
//! drift after a sync; they are convenience, not permanent handles (the uuid is).

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
    /// Actor-scoped nonce, e.g. `alice-3`. The typeable actor-qualified form.
    pub nonce: String,
    /// Creating actor slug, e.g. `alice`.
    pub actor: String,
    /// Per-actor sequence number (the `3` in `alice-3`).
    pub number: usize,
    /// The display handle: `#3` when that number is unambiguous across all
    /// issues, else the actor-qualified `alice-#3` (only needed once several
    /// actors share a number, i.e. after a sync).
    pub handle: String,
    /// Short uuid prefix, always a valid permanent handle.
    pub short: String,
}

/// Compute display ids for every issue.
///
/// For each creating actor, its issues are sorted by UUIDv7 (time order) and
/// numbered 1..N. This is the deterministic collision resolution: if an actor
/// minted the same number from two offline clones, the earlier UUIDv7 keeps the
/// lower number and the later drifts to the next slot. The shown handle is a
/// bare `#N` while that number is globally unique, and only becomes the
/// actor-qualified `alice-#N` once two actors share a number. Numbers may drift
/// after a sync; they are convenience, not permanent handles (the uuid is).
pub fn assign_display_ids(issues: &[Issue]) -> HashMap<String, DisplayId> {
    let mut by_actor: HashMap<String, Vec<&Issue>> = HashMap::new();
    for issue in issues {
        by_actor
            .entry(issue.creator.clone())
            .or_default()
            .push(issue);
    }

    // First pass: assign each issue its (actor, per-actor number).
    let mut assigned: Vec<(String, String, usize)> = Vec::new(); // (uuid, actor, number)
    for (actor, mut group) in by_actor {
        // UUIDv7 hex is lexicographically time-ordered, so a string sort is a
        // timestamp sort.
        group.sort_by(|a, b| a.uuid.cmp(&b.uuid));
        for (i, issue) in group.iter().enumerate() {
            assigned.push((issue.uuid.clone(), actor.clone(), i + 1));
        }
    }

    // Count how many issues carry each number, so a bare `#N` is shown only when
    // it is unambiguous.
    let mut number_counts: HashMap<usize, usize> = HashMap::new();
    for (_, _, number) in &assigned {
        *number_counts.entry(*number).or_default() += 1;
    }

    let mut out = HashMap::new();
    for (uuid, actor, number) in assigned {
        let handle = if number_counts.get(&number).copied().unwrap_or(0) > 1 {
            format!("{actor}-#{number}")
        } else {
            format!("#{number}")
        };
        out.insert(
            uuid.clone(),
            DisplayId {
                nonce: format!("{actor}-{number}"),
                short: short_prefix(&uuid),
                actor,
                number,
                handle,
                uuid,
            },
        );
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

    // 1. Position number: `#N` or bare `N` (the `#` is optional because a shell
    //    treats a leading `#` as a comment). Matches by display number; several
    //    issues can share a number across actors, so this can be ambiguous.
    let qn = ql.strip_prefix('#').unwrap_or(&ql);
    if !qn.is_empty() && qn.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(n) = qn.parse::<usize>() {
            let mut matches: Vec<String> = display
                .values()
                .filter(|d| d.number == n)
                .map(|d| d.uuid.clone())
                .collect();
            matches.sort();
            match matches.len() {
                0 => {} // fall through: could be an all-digit uuid prefix
                1 => return Resolution::Unique(matches.into_iter().next().unwrap()),
                _ => return Resolution::Ambiguous(matches),
            }
        }
    }

    // 2. Exact actor-nonce match (`alice-1`; nonces are lowercase, so the
    //    lowercased query makes `ALICE-1` resolve like `alice-1`). Also accept
    //    the shown `alice-#1` form by dropping the `#`.
    let nonce_query = ql.replace("-#", "-");
    for d in display.values() {
        if d.nonce == nonce_query {
            return Resolution::Unique(d.uuid.clone());
        }
    }

    // 3. Exact full-uuid match.
    if display.contains_key(&ql) {
        return Resolution::Unique(ql);
    }

    // 4. UUIDv7 hex-prefix match (only if the query looks like hex).
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
    fn single_actor_shows_bare_numbers() {
        // One actor: handles are bare #N, resolvable by the plain number.
        let issues = vec![
            issue_with("018f0000000000000000000000000010", "alice"),
            issue_with("018f0000000000000000000000000011", "alice"),
        ];
        let d = assign_display_ids(&issues);
        assert_eq!(d["018f0000000000000000000000000010"].handle, "#1");
        assert_eq!(d["018f0000000000000000000000000011"].handle, "#2");
        // Bare number resolves; so does the #-prefixed form.
        assert!(
            matches!(resolve("2", &d), Resolution::Unique(u) if u == "018f0000000000000000000000000011")
        );
        assert!(matches!(resolve("#2", &d), Resolution::Unique(_)));
        // The actor-qualified alias still resolves.
        assert!(matches!(resolve("alice-1", &d), Resolution::Unique(_)));
    }

    #[test]
    fn shared_number_qualifies_with_actor_and_bare_number_is_ambiguous() {
        // Two actors both have a #1: the handle qualifies with the actor, and a
        // bare `1` is ambiguous (lists both).
        let issues = vec![
            issue_with("018f0000000000000000000000000001", "alice"),
            issue_with("018f0000000000000000000000000002", "bob"),
        ];
        let d = assign_display_ids(&issues);
        assert_eq!(d["018f0000000000000000000000000001"].handle, "alice-#1");
        assert_eq!(d["018f0000000000000000000000000002"].handle, "bob-#1");
        match resolve("1", &d) {
            Resolution::Ambiguous(c) => assert_eq!(c.len(), 2),
            _ => panic!("bare number should be ambiguous across actors"),
        }
        // Actor-qualified disambiguates.
        assert!(
            matches!(resolve("bob-1", &d), Resolution::Unique(u) if u == "018f0000000000000000000000000002")
        );
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

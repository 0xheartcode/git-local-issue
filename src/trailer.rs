//! Git commit-message trailers: the wire format for op metadata.
//!
//! A commit message is `subject`, an optional free-form `body`, then a trailing
//! block of `Key: value` lines (git's own trailer convention). We keep the
//! body verbatim (comment text, description) and put structured metadata in the
//! trailer block. Unknown trailer keys are preserved on read and ignored by the
//! model, so a newer writer never breaks an older reader.

use std::collections::HashMap;

/// A key is a trailer key if it is a non-empty run of letters, digits and
/// hyphens starting with a letter, followed by `:`.
fn parse_trailer_line(line: &str) -> Option<(String, String)> {
    let (key, value) = line.split_once(':')?;
    if key.is_empty() {
        return None;
    }
    let mut chars = key.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    Some((key.to_string(), value.trim().to_string()))
}

/// Split a commit message into (subject, body, trailers).
///
/// The trailer block is the last paragraph when every one of its lines parses
/// as a trailer and it is preceded by a blank line (or is the whole message
/// after the subject). If the whole message is a single paragraph it is treated
/// as the subject (plus body), never as trailers, matching git's behaviour.
///
/// The body is preserved verbatim between the subject and the trailer block:
/// only the blank-line separators around it are stripped, so internal blank
/// lines and leading whitespace on content lines survive a round-trip. Line
/// endings are not normalised.
pub fn split_message(message: &str) -> (String, String, HashMap<String, String>) {
    let lines: Vec<&str> = message.split('\n').collect();
    let subject = lines.first().copied().unwrap_or("").trim().to_string();

    // Consider only the region after the subject line.
    let mut end = lines.len();
    while end > 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }

    // Walk back over a maximal trailing run of trailer lines: the candidate
    // trailer block is [tb_start, end).
    let mut tb_start = end;
    while tb_start > 1 && parse_trailer_line(lines[tb_start - 1]).is_some() {
        tb_start -= 1;
    }

    // Valid only if non-empty and it is a whole paragraph: the line just above
    // it must be blank (so a trailer-shaped line glued to a body paragraph is
    // not misread as metadata).
    let has_trailers = tb_start < end && (tb_start == 1 || lines[tb_start - 1].trim().is_empty());

    let mut trailers = HashMap::new();
    let body_end = if has_trailers {
        for line in &lines[tb_start..end] {
            if let Some((k, v)) = parse_trailer_line(line) {
                // Last write wins for a repeated key; single-valued trailers
                // (`Labels`, `Title`, ...) never repeat on write.
                trailers.insert(k, v);
            }
        }
        tb_start
    } else {
        end
    };

    // Body is lines[1..body_end] with only the surrounding blank separators
    // trimmed; internal formatting is preserved exactly.
    let body_lines = &lines[1.min(body_end)..body_end];
    let mut bs = 0;
    while bs < body_lines.len() && body_lines[bs].trim().is_empty() {
        bs += 1;
    }
    let mut be = body_lines.len();
    while be > bs && body_lines[be - 1].trim().is_empty() {
        be -= 1;
    }
    let body = body_lines[bs..be].join("\n");

    (subject, body, trailers)
}

/// An ordered trailer block being built for a commit message.
pub struct TrailerBlock {
    lines: Vec<(String, String)>,
}

impl TrailerBlock {
    pub fn new() -> Self {
        TrailerBlock { lines: Vec::new() }
    }

    pub fn push(&mut self, key: &str, value: &str) {
        // Trailer values are single-line; collapse any newlines defensively.
        let value = value.replace('\n', " ");
        self.lines.push((key.to_string(), value));
    }

    pub fn render(&self) -> String {
        self.lines
            .iter()
            .map(|(k, v)| {
                if v.is_empty() {
                    format!("{k}:")
                } else {
                    format!("{k}: {v}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Default for TrailerBlock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_subject_body_and_trailers() {
        let msg = "Subject line\n\nBody paragraph one.\n\nBody two.\n\nOp: comment\nActor: alice\n";
        let (subject, body, trailers) = split_message(msg);
        assert_eq!(subject, "Subject line");
        assert_eq!(body, "Body paragraph one.\n\nBody two.");
        assert_eq!(trailers.get("Op").unwrap(), "comment");
        assert_eq!(trailers.get("Actor").unwrap(), "alice");
    }

    #[test]
    fn empty_trailer_value_is_preserved() {
        let msg = "S\n\nAssignee:\nOp: set-assignee\n";
        let (_, _, trailers) = split_message(msg);
        assert_eq!(trailers.get("Assignee").unwrap(), "");
        assert_eq!(trailers.get("Op").unwrap(), "set-assignee");
    }

    #[test]
    fn body_that_looks_like_trailers_is_not_consumed_as_trailers() {
        // Only the LAST paragraph is the trailer block; a trailer-shaped line
        // inside the body must stay in the body.
        let msg = "S\n\nNote: this looks like a trailer\n\nOp: comment\nActor: a\n";
        let (_, body, trailers) = split_message(msg);
        assert_eq!(body, "Note: this looks like a trailer");
        assert_eq!(trailers.get("Op").unwrap(), "comment");
        assert!(!trailers.contains_key("Note"));
    }

    #[test]
    fn single_paragraph_message_has_no_trailers() {
        let msg = "Just a subject";
        let (subject, body, trailers) = split_message(msg);
        assert_eq!(subject, "Just a subject");
        assert_eq!(body, "");
        assert!(trailers.is_empty());
    }

    #[test]
    fn render_round_trips_through_split() {
        let mut t = TrailerBlock::new();
        t.push("Op", "comment");
        t.push("Assignee", "");
        t.push("Lamport", "3");
        let rendered = t.render();
        let msg = format!("Subject\n\n{rendered}\n");
        let (_, _, trailers) = split_message(&msg);
        assert_eq!(trailers.get("Op").unwrap(), "comment");
        assert_eq!(trailers.get("Assignee").unwrap(), "");
        assert_eq!(trailers.get("Lamport").unwrap(), "3");
    }
}

//! Claim-log anchors (#1551, design `docs/projects/let-em-loose/1551-claim-log-anchor-design.md`).
//!
//! The board keeps each linked BBP task's log head as an append-only card comment, written
//! under [`ACTOR`], a name no credential can act as (`:` is outside the actor alphabet). On
//! each tick it runs `bbp verify --expect <last anchor>`: a log rewritten by its writer no
//! longer contains the anchored line and fails. This module is pure: it reads comments and a
//! verify result and says what to write.

use std::collections::BTreeSet;

use crate::domain::{Card, Comment};

/// The internal actor anchors are written under.
pub const ACTOR: &str = "bbp-anchor:mill-board";

/// Every anchor comment starts with this.
const MARK: &str = "bbp-anchor v1 task=";

/// The longest failure reason kept in a comment.
const MAX_REASON: usize = 400;

/// What `bbp verify` printed on success.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    /// Complete lines in the log.
    pub lines: u64,
    /// Legacy lines before the chain starts.
    pub unchained: u64,
    /// SHA-256 hex of the last line.
    pub head: String,
}

/// One trusted anchor comment for a task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// The head verified and was recorded.
    Anchored {
        /// The recorded head.
        head: String,
    },
    /// `bbp verify` refused, with its reason.
    Failed(String),
}

/// Whether `task` can be written into and read back from a comment unambiguously.
#[must_use]
pub fn anchorable(task: &str) -> bool {
    !task.is_empty() && !task.contains(char::is_whitespace)
}

/// The tasks to anchor on `card`: its linked task, plus every task it already holds a
/// trusted anchor for, so unlinking or re-linking a card never stops anchoring silently.
#[must_use]
pub fn tasks(card: &Card) -> BTreeSet<String> {
    let linked = card.execution.as_ref().map(|x| x.bbp_task.clone());
    card.comments
        .iter()
        .filter(|c| c.by == ACTOR)
        .filter_map(|c| c.text.strip_prefix(MARK))
        .filter_map(|rest| rest.split(' ').next())
        .map(str::to_owned)
        .chain(linked)
        .filter(|t| anchorable(t))
        .collect()
}

/// The trusted anchor history of `task`, oldest first. Comments by anyone but [`ACTOR`],
/// or for another task, are ignored.
#[must_use]
pub fn history(comments: &[Comment], task: &str) -> Vec<Entry> {
    comments
        .iter()
        .filter(|c| c.by == ACTOR)
        .filter_map(|c| parse(&c.text, task))
        .collect()
}

fn parse(text: &str, task: &str) -> Option<Entry> {
    let rest = text.strip_prefix(MARK)?.strip_prefix(task)?;
    if let Some(reason) = rest.strip_prefix(" FAILED: ") {
        return Some(Entry::Failed(reason.to_owned()));
    }
    let head = rest.strip_prefix(" lines=")?.split_once(" head=")?.1;
    is_hex64(head).then(|| Entry::Anchored {
        head: head.to_owned(),
    })
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The head of the last good anchor: what the next verify must still find.
#[must_use]
pub fn last_head(history: &[Entry]) -> Option<&str> {
    history.iter().rev().find_map(|e| match e {
        Entry::Anchored { head } => Some(head.as_str()),
        Entry::Failed(_) => None,
    })
}

/// The comment to write for `task` after a verify returned `result`, or `None` to write
/// nothing. A failure is written once until its reason changes; a head is written when it
/// is chained and differs from the last entry, or when it verifies again after a failure.
#[must_use]
pub fn next(task: &str, history: &[Entry], result: Result<&Verified, &str>) -> Option<String> {
    let latest = history.last();
    match result {
        Err(reason) => {
            let reason = one_line(reason);
            let repeat = matches!(latest, Some(Entry::Failed(r)) if *r == reason);
            (!repeat).then(|| format!("{MARK}{task} FAILED: {reason}"))
        }
        Ok(v) => {
            let unchanged = matches!(latest, Some(Entry::Anchored { head }) if *head == v.head);
            let chained = v.unchained < v.lines;
            (chained && !unchanged && is_hex64(&v.head))
                .then(|| format!("{MARK}{task} lines={} head={}", v.lines, v.head))
        }
    }
}

/// `reason` on one line, cut to [`MAX_REASON`] bytes at a character boundary.
fn one_line(reason: &str) -> String {
    let flat = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    let flat = if flat.is_empty() {
        "no reason given".to_owned()
    } else {
        flat
    };
    let mut end = flat.len().min(MAX_REASON);
    while !flat.is_char_boundary(end) {
        end -= 1;
    }
    flat[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn comment(by: &str, text: &str) -> Comment {
        Comment {
            by: by.to_owned(),
            text: text.to_owned(),
            ..Comment::default()
        }
    }

    fn verified(lines: u64, unchained: u64, head: &str) -> Verified {
        Verified {
            lines,
            unchained,
            head: head.to_owned(),
        }
    }

    #[test]
    fn a_first_chained_head_is_recorded() {
        let v = verified(3, 0, A);
        let text = next("T1", &[], Ok(&v)).expect("a first head is written");
        assert_eq!(text, format!("bbp-anchor v1 task=T1 lines=3 head={A}"));
        let read = history(&[comment(ACTOR, &text)], "T1");
        assert_eq!(read, vec![Entry::Anchored { head: A.into() }]);
    }

    #[test]
    fn an_unchanged_head_writes_nothing() {
        let h = [Entry::Anchored { head: A.into() }];
        assert_eq!(next("T1", &h, Ok(&verified(3, 0, A))), None);
        assert!(next("T1", &h, Ok(&verified(4, 0, B))).is_some());
    }

    #[test]
    fn an_all_legacy_or_empty_log_is_not_anchored() {
        assert_eq!(next("T1", &[], Ok(&verified(5, 5, A))), None);
        assert_eq!(next("T1", &[], Ok(&verified(0, 0, A))), None);
        assert!(next("T1", &[], Ok(&verified(6, 5, A))).is_some());
    }

    #[test]
    fn a_failure_is_written_once_per_reason() {
        let text = next("T1", &[], Err("--expect is not\na line")).expect("written");
        assert_eq!(text, "bbp-anchor v1 task=T1 FAILED: --expect is not a line");
        let h = history(&[comment(ACTOR, &text)], "T1");
        assert_eq!(next("T1", &h, Err("--expect is not a line")), None);
        assert!(next("T1", &h, Err("chain broken")).is_some());
    }

    #[test]
    fn a_head_that_verifies_after_a_failure_is_written_again() {
        let h = [
            Entry::Anchored { head: A.into() },
            Entry::Failed("partial line".into()),
        ];
        assert_eq!(last_head(&h), Some(A));
        assert!(next("T1", &h, Ok(&verified(3, 0, A))).is_some());
    }

    #[test]
    fn only_the_anchor_actor_and_the_named_task_count() {
        let forged = format!("bbp-anchor v1 task=T1 lines=1 head={B}");
        let other = format!("bbp-anchor v1 task=T10 lines=1 head={B}");
        let ok = format!("bbp-anchor v1 task=T1 lines=1 head={A}");
        let comments = [
            comment("claude-cli/x", &forged),
            comment(ACTOR, &other),
            comment(ACTOR, &ok),
            comment(ACTOR, "bbp-anchor v1 task=T1 lines=1 head=nothex"),
        ];
        assert_eq!(
            history(&comments, "T1"),
            vec![Entry::Anchored { head: A.into() }]
        );
    }

    #[test]
    fn a_long_reason_is_cut_at_a_char_boundary() {
        let reason = "é".repeat(MAX_REASON);
        let text = next("T1", &[], Err(&reason)).expect("written");
        let kept = text.split("FAILED: ").nth(1).expect("has a reason");
        assert!(kept.len() <= MAX_REASON);
    }

    #[test]
    fn tasks_with_whitespace_are_not_anchorable() {
        assert!(anchorable("T1"));
        assert!(!anchorable("T 1"));
        assert!(!anchorable(""));
    }
}

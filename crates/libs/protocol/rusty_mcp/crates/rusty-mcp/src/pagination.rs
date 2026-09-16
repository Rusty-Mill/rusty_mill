//! One cursor implementation, shared by every paginated list.
//!
//! There are four list methods in this crate that page — resources, resource
//! templates, tools and prompts — and they must not each invent a cursor. Two
//! shapes in one server is its own bug: a client cannot tell which sequence a
//! cursor came from, and neither can the server.
//!
//! # The cursor carries a key, not an index
//!
//! That is the whole trick, and both properties fall out of it:
//!
//! - A fabricated cursor names some position in the **key space** rather than
//!   an offset into a slice, so there is no such thing as an out-of-range read
//!   to guard against.
//! - An entry added or removed between requests cannot shift the entries after
//!   it onto a page they were already served on. The cursor stays valid even
//!   when the entry it names is the one that was deleted.
//!
//! Pages are ordered by key rather than by registration order, which is what
//! makes that total order exist in the first place.

use rmcp::model::ErrorData;

/// Entries returned per page when nothing else is configured.
///
/// Matches the cap the spec puts on completion results, for no deeper reason
/// than that a hundred of anything is a reasonable thing to put in one
/// response.
pub const DEFAULT_PAGE_SIZE: usize = 100;

/// Cursor format version, so a cursor held across a deploy is refused rather
/// than reinterpreted under a new encoding.
const CURSOR_VERSION: u8 = b'1';

/// Which sequence a cursor belongs to.
///
/// Tagging the sequence is what stops a cursor issued by `tools/list` from
/// quietly seeking into the resources. Every list in a server shares the key
/// space of *its own* sequence and no other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CursorKind {
    /// `resources/list`.
    Resource,
    /// `resources/templates/list`.
    Template,
    /// `tools/list`.
    Tool,
    /// `prompts/list`.
    Prompt,
}

impl CursorKind {
    /// A one-byte tag, so a cursor issued for one sequence is rejected rather
    /// than quietly indexing into another.
    fn tag(self) -> u8 {
        match self {
            Self::Resource => b'r',
            Self::Template => b't',
            Self::Tool => b'o',
            Self::Prompt => b'p',
        }
    }
}

/// Encode a cursor naming `key` in the `kind` sequence.
pub fn encode_cursor(kind: CursorKind, key: &str) -> String {
    let mut payload = vec![CURSOR_VERSION, kind.tag()];
    payload.extend_from_slice(key.as_bytes());
    rusty_base64::encode_url_safe_no_pad(&payload)
}

/// Decode a cursor, rejecting anything this server did not issue for `kind`.
pub fn decode_cursor(kind: CursorKind, cursor: &str) -> Result<String, ErrorData> {
    // Deliberately one message for every failure. Which byte was wrong is of no
    // use to a caller and only helps someone probing the encoding.
    let invalid =
        || ErrorData::invalid_params("the pagination cursor is not one this server issued", None);

    let bytes = rusty_base64::decode_url_safe(cursor).map_err(|_| invalid())?;

    let [version, tag, key @ ..] = bytes.as_slice() else {
        return Err(invalid());
    };
    if *version != CURSOR_VERSION || *tag != kind.tag() {
        return Err(invalid());
    }

    String::from_utf8(key.to_vec()).map_err(|_| invalid())
}

/// One page of `items`, ordered by `key`, resuming after `cursor`.
///
/// Returns the page and the cursor for the next one, which is `None` exactly
/// when nothing is left. Emitting a cursor on the last page costs the client a
/// whole extra round trip to learn nothing.
pub fn page<'a, T>(
    items: &'a [T],
    key: impl Fn(&'a T) -> &'a str,
    kind: CursorKind,
    cursor: Option<&str>,
    page_size: usize,
) -> Result<(Vec<&'a T>, Option<String>), ErrorData> {
    let after = cursor.map(|c| decode_cursor(kind, c)).transpose()?;

    let mut ordered: Vec<(&str, &T)> = items.iter().map(|item| (key(item), item)).collect();
    ordered.sort_by(|a, b| a.0.cmp(b.0));

    // Strictly after: the cursor names the last entry already served.
    let start = match &after {
        Some(after) => ordered.partition_point(|(k, _)| *k <= after.as_str()),
        None => 0,
    };

    let remaining = &ordered[start..];
    let taken = remaining.len().min(page_size);
    let next = (remaining.len() > taken).then(|| encode_cursor(kind, remaining[taken - 1].0));

    Ok((
        remaining[..taken].iter().map(|(_, item)| *item).collect(),
        next,
    ))
}

/// Page a list of owned values, cloning what the page contains.
///
/// The `rmcp` routers hand back `Vec<Tool>` and `Vec<Prompt>` by value, so the
/// borrowing form above has nothing to borrow from.
pub fn page_owned<T: Clone>(
    items: &[T],
    key: impl for<'a> Fn(&'a T) -> &'a str,
    kind: CursorKind,
    cursor: Option<&str>,
    page_size: usize,
) -> Result<(Vec<T>, Option<String>), ErrorData> {
    let (page, next) = page(items, key, kind, cursor, page_size)?;
    Ok((page.into_iter().cloned().collect(), next))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(
        items: &[&str],
        kind: CursorKind,
        cursor: Option<&str>,
        size: usize,
    ) -> (Vec<String>, Option<String>) {
        let owned: Vec<String> = items.iter().map(|s| (*s).to_string()).collect();
        let (page, next) = page(&owned, |s| s.as_str(), kind, cursor, size).expect("pages");
        (page.into_iter().cloned().collect(), next)
    }

    #[test]
    fn a_cursor_from_one_sequence_is_rejected_by_every_other() {
        // The property that keeps four independent lists from colliding.
        let kinds = [
            CursorKind::Resource,
            CursorKind::Template,
            CursorKind::Tool,
            CursorKind::Prompt,
        ];

        for minted in kinds {
            let cursor = encode_cursor(minted, "some-key");
            for spent in kinds {
                let result = decode_cursor(spent, &cursor);
                if minted == spent {
                    assert_eq!(result.expect("its own kind"), "some-key");
                } else {
                    assert!(
                        result.is_err(),
                        "a {minted:?} cursor was accepted by {spent:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_kind_has_a_distinct_tag() {
        // A duplicated tag would silently reintroduce the collision above.
        let tags: std::collections::BTreeSet<u8> = [
            CursorKind::Resource,
            CursorKind::Template,
            CursorKind::Tool,
            CursorKind::Prompt,
        ]
        .into_iter()
        .map(CursorKind::tag)
        .collect();

        assert_eq!(tags.len(), 4, "two kinds share a tag");
    }

    #[test]
    fn paging_covers_everything_exactly_once() {
        let items = ["e", "a", "d", "b", "c"];
        let mut seen = Vec::new();
        let mut cursor = None;

        loop {
            let (page, next) = keys(&items, CursorKind::Tool, cursor.as_deref(), 2);
            seen.extend(page);
            match next {
                Some(n) => cursor = Some(n),
                None => break,
            }
        }

        assert_eq!(
            seen,
            ["a", "b", "c", "d", "e"],
            "sorted, complete, no repeats"
        );
    }

    #[test]
    fn the_last_page_carries_no_cursor() {
        let items = ["a", "b", "c", "d"];
        let (_, next) = keys(&items, CursorKind::Prompt, None, 2);
        let (page, next) = keys(&items, CursorKind::Prompt, next.as_deref(), 2);
        assert_eq!(page, ["c", "d"]);
        assert!(next.is_none(), "an exact multiple must not emit a cursor");
    }

    #[test]
    fn a_malformed_cursor_is_invalid_params() {
        for cursor in ["not base64!!", "", "AAAA"] {
            let err = decode_cursor(CursorKind::Tool, cursor).expect_err("should reject");
            assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        }
    }

    #[test]
    fn a_cursor_past_the_end_is_an_empty_final_page() {
        // Indistinguishable from one whose successors were all deleted, which
        // is a normal end to a walk rather than an error.
        let items = ["a", "b"];
        let cursor = encode_cursor(CursorKind::Tool, "zzz");
        let (page, next) = keys(&items, CursorKind::Tool, Some(&cursor), 10);
        assert!(page.is_empty());
        assert!(next.is_none());
    }

    #[test]
    fn removing_the_entry_a_cursor_names_does_not_disturb_the_next_page() {
        let cursor = encode_cursor(CursorKind::Tool, "b");
        // "b" is gone by the time the cursor is spent.
        let (page, _) = keys(&["a", "c", "d"], CursorKind::Tool, Some(&cursor), 10);
        assert_eq!(page, ["c", "d"], "the walk resumes past the missing anchor");
    }
}

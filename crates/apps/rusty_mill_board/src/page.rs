//! Keyset pages over a listing, and the opaque cursor that continues one.
//!
//! A listing is ordered by `(created_ms, id)`, the board's list order. Neither changes when
//! a card is edited, so an edit never moves a card between pages. The cursor is stateless:
//! it names the last key a page returned and the query it belongs to, so it survives a
//! restart and carries no authority (scope is applied again to every page).
//!
//! Consistency is a live view, not a snapshot: a card unchanged during a scan is returned
//! exactly once; a card added during it sorts after every earlier card, so a later page or the
//! next scan returns it; a card removed or moved out of the query disappears.

use serde::Serialize;
use uuid::Uuid;

use crate::domain::BoardError;

/// The most cards a page may hold.
pub const MAX_LIMIT: usize = 500;
/// The serialized size a page stops at. One card is always returned, however large.
pub const PAGE_BYTES: usize = 512 * 1024;

/// Where a listing's order puts a card.
pub type Key = (i64, Uuid);

/// The opaque continuation of a paged listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// The last key the previous page returned.
    pub after: Key,
    /// [`query_hash`] of the query the cursor belongs to.
    pub query: u64,
}

impl Cursor {
    /// `v1.<created_ms>.<id>.<query hash>`. Opaque to clients.
    pub fn encode(&self) -> String {
        let (created, id) = self.after;
        format!("v1.{created}.{}.{:016x}", id.simple(), self.query)
    }

    /// Parse a cursor and check it belongs to `query`.
    ///
    /// # Errors
    /// [`BoardError::Invalid`] if it is malformed or belongs to another query.
    pub fn decode(text: &str, query: u64) -> Result<Self, BoardError> {
        let bad = || BoardError::Invalid("invalid cursor");
        let mut parts = text.split('.');
        let (Some("v1"), Some(created), Some(id), Some(hash), None) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return Err(bad());
        };
        let created: i64 = created.parse().map_err(|_| bad())?;
        let id = Uuid::try_parse(id).map_err(|_| bad())?;
        let hash = u64::from_str_radix(hash, 16).map_err(|_| bad())?;
        if hash != query {
            return Err(BoardError::Invalid(
                "cursor belongs to a different query; start again without it",
            ));
        }
        Ok(Self {
            after: (created, id),
            query,
        })
    }
}

/// FNV-1a over the query's canonical text: stable across runs and builds, so a cursor
/// outlives a restart.
pub fn query_hash(canonical: &str) -> u64 {
    canonical.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// A page limit from the caller: 1 to [`MAX_LIMIT`].
///
/// # Errors
/// [`BoardError::Invalid`] outside that range.
pub fn checked_limit(limit: u32) -> Result<usize, BoardError> {
    let limit = usize::try_from(limit).map_err(|_| BoardError::Invalid("limit is too large"))?;
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(BoardError::Invalid("limit must be 1 to 500"));
    }
    Ok(limit)
}

/// One page and, if more remain, the cursor to the next.
#[derive(Debug)]
pub struct Page<T> {
    /// The items, in key order.
    pub items: Vec<T>,
    /// Set when items remain after this page.
    pub next: Option<Cursor>,
}

/// The page of `items`, in `key` order, after `after`: at most `limit` items and, past the
/// first, no more than [`PAGE_BYTES`] serialized.
///
/// # Errors
/// [`BoardError::Storage`] if an item cannot be serialized.
pub fn page<T: Serialize>(
    mut items: Vec<T>,
    key: impl Fn(&T) -> Key,
    after: Option<Key>,
    limit: usize,
    query: u64,
) -> Result<Page<T>, BoardError> {
    items.sort_by_key(|item| key(item));
    let mut rest = items
        .into_iter()
        .filter(|item| after.is_none_or(|a| key(item) > a))
        .peekable();
    let mut taken = Vec::new();
    let mut bytes = 0;
    while let Some(item) = rest.peek() {
        let size = serde_json::to_vec(item)
            .map_err(|e| BoardError::Storage(e.to_string()))?
            .len();
        if taken.len() == limit || (!taken.is_empty() && bytes + size > PAGE_BYTES) {
            break;
        }
        bytes += size;
        taken.extend(rest.next());
    }
    let next = match (rest.peek(), taken.last()) {
        (Some(_), Some(last)) => Some(Cursor {
            after: key(last),
            query,
        }),
        _ => None,
    };
    Ok(Page { items: taken, next })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed(n: u128) -> Key {
        (i64::try_from(n).expect("small"), Uuid::from_u128(n))
    }

    #[test]
    fn cursor_round_trips_and_refuses_another_query_or_garbage() {
        let cursor = Cursor {
            after: keyed(7),
            query: query_hash("a"),
        };
        assert_eq!(
            Cursor::decode(&cursor.encode(), query_hash("a")).expect("ok"),
            cursor
        );
        assert!(Cursor::decode(&cursor.encode(), query_hash("b")).is_err());
        for bad in [
            "",
            "v1",
            "v2.1.x.y",
            "v1.x.00.00",
            "v1.1.zz.00",
            "v1.1.2.3.4",
        ] {
            assert!(Cursor::decode(bad, query_hash("a")).is_err(), "{bad}");
        }
    }

    #[test]
    fn pages_cover_every_item_once_in_order() {
        let items: Vec<Key> = (1..=23).map(keyed).collect();
        let mut seen = Vec::new();
        let mut after = None;
        loop {
            let p = page(items.clone(), |k| *k, after, 5, 1).expect("page");
            assert!(p.items.len() <= 5);
            seen.extend(p.items);
            match p.next {
                Some(c) => after = Some(c.after),
                None => break,
            }
        }
        assert_eq!(seen, items);
    }

    #[test]
    fn a_page_stops_at_its_byte_budget_but_always_holds_one_item() {
        let big = "x".repeat(PAGE_BYTES);
        let items: Vec<(Key, String)> = (1..=3).map(|n| (keyed(n), big.clone())).collect();
        let p = page(items, |i| i.0, None, 500, 1).expect("page");
        assert_eq!(p.items.len(), 1);
        assert_eq!(p.next.map(|c| c.after), Some(keyed(1)));
    }

    #[test]
    fn limits_outside_one_to_five_hundred_are_refused() {
        assert!(checked_limit(0).is_err());
        assert!(checked_limit(501).is_err());
        assert_eq!(checked_limit(500).expect("ok"), 500);
    }
}

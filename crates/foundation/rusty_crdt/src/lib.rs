//! `rusty_crdt`: a sequence CRDT (RGA), sync, with no I/O and no dependencies.
//!
//! An [`Rga`] is a tree. Every element has a parent: the element to its left
//! when it was authored, or the head. Siblings sort by **descending** id, and
//! the visible sequence is a pre-order walk, so the newest sibling sits right
//! after its parent. A delete leaves a tombstone, so later ops that name the
//! element still resolve. Replicas that apply the same set of ops reach the
//! same sequence, in any causally valid order.
//!
//! # Caller contract
//!
//! - Ids are unique across replicas and totally ordered by `Ord`. Reusing an
//!   id with a different parent or value is refused with [`Error::IdReused`].
//! - For a local insert to land where it was meant, its id must be greater
//!   than every existing child of its parent. [`Rga::insert_at`] checks this
//!   ([`Error::NotFresh`]). Lamport-style ids (`(clock, site)`, the clock past
//!   every id seen) satisfy it. Convergence does not depend on it.
//! - Delivery need not be causal. An op whose parent or target is unknown is
//!   reported as [`Applied::Pending`] and leaves the state unchanged; the
//!   caller buffers it or refuses it.
//!
//! Op logs, version vectors, transport and persistence are the caller's.

use std::collections::BTreeMap;
use std::fmt;

/// One change to a sequence, as replicas exchange it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op<I, T> {
    /// Insert `value` as element `id` after `parent` (`None` is the head).
    Insert {
        /// The new element's id.
        id: I,
        /// The element it was authored to the right of.
        parent: Option<I>,
        /// The element itself.
        value: T,
    },
    /// Tombstone element `target`.
    Delete {
        /// The element to hide.
        target: I,
    },
}

/// What [`Rga::apply`] did with an op.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Applied<I> {
    /// The visible sequence or the tombstone set changed.
    Changed,
    /// The op was already applied; nothing changed.
    Duplicate,
    /// The op names an element this replica has not seen. Nothing changed;
    /// apply it again once `missing` has arrived.
    Pending {
        /// The unknown parent or target.
        missing: I,
    },
}

/// Why an op could not be built or applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error<I> {
    /// A position past the visible length.
    OutOfRange {
        /// The position asked for.
        pos: usize,
        /// The visible length.
        len: usize,
    },
    /// A local insert id not greater than every existing child of its parent:
    /// it would render in the wrong place.
    NotFresh {
        /// The rejected id.
        id: I,
    },
    /// The id is already held with a different parent or value.
    IdReused {
        /// The reused id.
        id: I,
    },
}

impl<I: fmt::Debug> fmt::Display for Error<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::OutOfRange { pos, len } => {
                write!(f, "position {pos} is past the visible length {len}")
            }
            Error::NotFresh { id } => write!(
                f,
                "id {id:?} is not greater than every existing child of its parent"
            ),
            Error::IdReused { id } => {
                write!(
                    f,
                    "id {id:?} is already held with a different parent or value"
                )
            }
        }
    }
}

impl<I: fmt::Debug> std::error::Error for Error<I> {}

#[derive(Clone, Debug)]
struct Node<I, T> {
    id: I,
    parent: Option<usize>,
    value: T,
    live: bool,
    /// Indices into the arena, by descending id.
    children: Vec<usize>,
}

/// A replicated sequence of `T`, with elements identified by `I`.
///
/// `len`, `is_empty` and `contains` are cheap; positional lookups walk the
/// tree and are O(n). Every walk is iterative, so a long chain of inserts
/// cannot overflow the stack. Tombstones keep their value, so a replayed
/// insert is told apart from a reused id even after a delete.
#[derive(Clone, Debug)]
pub struct Rga<I, T> {
    nodes: Vec<Node<I, T>>,
    index: BTreeMap<I, usize>,
    /// Indices of head elements, by descending id.
    roots: Vec<usize>,
    visible: usize,
}

impl<I, T> Default for Rga<I, T> {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            index: BTreeMap::new(),
            roots: Vec::new(),
            visible: 0,
        }
    }
}

impl<I: Clone + Ord, T: Clone + Eq> Rga<I, T> {
    /// An empty sequence.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of visible elements. O(1).
    pub fn len(&self) -> usize {
        self.visible
    }

    /// True when no element is visible.
    pub fn is_empty(&self) -> bool {
        self.visible == 0
    }

    /// True when `id` is held, live or tombstoned.
    pub fn contains(&self, id: &I) -> bool {
        self.index.contains_key(id)
    }

    /// The visible elements with their ids, in sequence order.
    pub fn iter(&self) -> Iter<'_, I, T> {
        Iter {
            rga: self,
            stack: self.roots.iter().rev().copied().collect(),
        }
    }

    /// The id of the visible element at `pos`. O(n).
    pub fn id_at(&self, pos: usize) -> Option<&I> {
        self.iter().nth(pos).map(|(id, _)| id)
    }

    /// The op that inserts `value` as element `id` at visible position `pos`
    /// (`pos == len()` appends). The state is unchanged until the op is
    /// applied.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfRange`] when `pos > len()`, [`Error::IdReused`] when `id`
    /// is already held, and [`Error::NotFresh`] when `id` is not greater than
    /// every existing child of the element it would follow.
    pub fn insert_at(&self, pos: usize, id: I, value: T) -> Result<Op<I, T>, Error<I>> {
        if pos > self.visible {
            return Err(Error::OutOfRange {
                pos,
                len: self.visible,
            });
        }
        if self.contains(&id) {
            return Err(Error::IdReused { id });
        }
        let parent = match pos.checked_sub(1) {
            None => None,
            Some(left) => self.id_at(left).cloned(),
        };
        let parent_ix = parent.as_ref().and_then(|p| self.index.get(p).copied());
        if self
            .children(parent_ix)
            .first()
            .is_some_and(|&n| self.nodes[n].id > id)
        {
            return Err(Error::NotFresh { id });
        }
        Ok(Op::Insert { id, parent, value })
    }

    /// The op that deletes the visible element at `pos`.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfRange`] when `pos >= len()`.
    pub fn delete_at(&self, pos: usize) -> Result<Op<I, T>, Error<I>> {
        match self.id_at(pos) {
            Some(target) => Ok(Op::Delete {
                target: target.clone(),
            }),
            None => Err(Error::OutOfRange {
                pos,
                len: self.visible,
            }),
        }
    }

    /// Apply an op, local or remote. Idempotent, and never panics.
    ///
    /// # Errors
    ///
    /// [`Error::IdReused`] when an insert reuses a held id with a different
    /// parent or value. The state is unchanged.
    pub fn apply(&mut self, op: &Op<I, T>) -> Result<Applied<I>, Error<I>> {
        match op {
            Op::Insert { id, parent, value } => self.apply_insert(id, parent.as_ref(), value),
            Op::Delete { target } => Ok(self.apply_delete(target)),
        }
    }

    fn apply_insert(
        &mut self,
        id: &I,
        parent: Option<&I>,
        value: &T,
    ) -> Result<Applied<I>, Error<I>> {
        if let Some(&held) = self.index.get(id) {
            return self.replay(held, id, parent, value);
        }
        let parent_ix = match parent {
            None => None,
            Some(p) => match self.index.get(p) {
                Some(&ix) => Some(ix),
                None => return Ok(Applied::Pending { missing: p.clone() }),
            },
        };
        let at = self
            .children(parent_ix)
            .partition_point(|&s| self.nodes[s].id > *id);
        let ix = self.nodes.len();
        match parent_ix {
            None => self.roots.insert(at, ix),
            Some(p) => self.nodes[p].children.insert(at, ix),
        }
        self.nodes.push(Node {
            id: id.clone(),
            parent: parent_ix,
            value: value.clone(),
            live: true,
            children: Vec::new(),
        });
        self.index.insert(id.clone(), ix);
        self.visible += 1;
        Ok(Applied::Changed)
    }

    /// An insert of a held id: a duplicate when parent and value match.
    fn replay(
        &self,
        held: usize,
        id: &I,
        parent: Option<&I>,
        value: &T,
    ) -> Result<Applied<I>, Error<I>> {
        let node = &self.nodes[held];
        let held_parent = node.parent.map(|p| &self.nodes[p].id);
        if held_parent == parent && node.value == *value {
            Ok(Applied::Duplicate)
        } else {
            Err(Error::IdReused { id: id.clone() })
        }
    }

    fn apply_delete(&mut self, target: &I) -> Applied<I> {
        let Some(&ix) = self.index.get(target) else {
            return Applied::Pending {
                missing: target.clone(),
            };
        };
        let node = &mut self.nodes[ix];
        if !node.live {
            return Applied::Duplicate;
        }
        node.live = false;
        self.visible -= 1;
        Applied::Changed
    }

    /// The children of `parent`, or the head elements for `None`.
    fn children(&self, parent: Option<usize>) -> &[usize] {
        match parent {
            None => &self.roots,
            Some(ix) => &self.nodes[ix].children,
        }
    }
}

/// Iterator over the visible `(id, value)` pairs of an [`Rga`], in order.
pub struct Iter<'a, I, T> {
    rga: &'a Rga<I, T>,
    stack: Vec<usize>,
}

impl<'a, I, T> Iterator for Iter<'a, I, T> {
    type Item = (&'a I, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(ix) = self.stack.pop() {
            let node = &self.rga.nodes[ix];
            self.stack.extend(node.children.iter().rev());
            if node.live {
                return Some((&node.id, &node.value));
            }
        }
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    type Seq = Rga<u32, char>;

    fn ins(id: u32, parent: Option<u32>, value: char) -> Op<u32, char> {
        Op::Insert { id, parent, value }
    }

    fn text(r: &Seq) -> String {
        r.iter().map(|(_, c)| *c).collect()
    }

    fn typed(s: &str) -> Seq {
        let mut r = Seq::new();
        for (i, c) in s.chars().enumerate() {
            let op = r.insert_at(i, i as u32 + 1, c).unwrap();
            r.apply(&op).unwrap();
        }
        r
    }

    #[test]
    fn appends_render_in_order() {
        let r = typed("abc");
        assert_eq!(text(&r), "abc");
        assert_eq!(r.len(), 3);
        assert_eq!(r.id_at(1), Some(&2));
    }

    #[test]
    fn a_local_insert_lands_where_it_was_meant() {
        let mut r = typed("ac");
        let op = r.insert_at(1, 9, 'b').unwrap();
        assert_eq!(op, ins(9, Some(1), 'b'));
        r.apply(&op).unwrap();
        assert_eq!(text(&r), "abc");
    }

    #[test]
    fn concurrent_siblings_put_the_greater_id_first() {
        let mut a = typed("x");
        let mut b = a.clone();
        let (lo, hi) = (ins(5, Some(1), 'l'), ins(6, Some(1), 'h'));
        a.apply(&lo).unwrap();
        a.apply(&hi).unwrap();
        b.apply(&hi).unwrap();
        b.apply(&lo).unwrap();
        assert_eq!(text(&a), "xhl");
        assert_eq!(text(&a), text(&b));
    }

    #[test]
    fn insert_at_refuses_bad_positions_and_ids() {
        let r = typed("ab");
        assert_eq!(
            r.insert_at(3, 9, 'z'),
            Err(Error::OutOfRange { pos: 3, len: 2 })
        );
        assert_eq!(r.insert_at(0, 1, 'z'), Err(Error::IdReused { id: 1 }));
        // The head already has child 1; id 0 would render after it.
        assert_eq!(r.insert_at(0, 0, 'z'), Err(Error::NotFresh { id: 0 }));
        assert_eq!(r.delete_at(2), Err(Error::OutOfRange { pos: 2, len: 2 }));
    }

    #[test]
    fn delete_tombstones_and_is_idempotent() {
        let mut r = typed("abc");
        let op = r.delete_at(1).unwrap();
        assert_eq!(r.apply(&op), Ok(Applied::Changed));
        assert_eq!(r.apply(&op), Ok(Applied::Duplicate));
        assert_eq!(text(&r), "ac");
        assert_eq!(r.len(), 2);
        assert!(r.contains(&2));
        // A tombstoned element still anchors an insert.
        r.apply(&ins(9, Some(2), 'X')).unwrap();
        assert_eq!(text(&r), "aXc");
    }

    #[test]
    fn unknown_references_are_pending_and_change_nothing() {
        let mut r = Seq::new();
        assert_eq!(
            r.apply(&ins(2, Some(1), 'b')),
            Ok(Applied::Pending { missing: 1 })
        );
        assert_eq!(
            r.apply(&Op::Delete { target: 1 }),
            Ok(Applied::Pending { missing: 1 })
        );
        assert!(r.is_empty());
        assert!(!r.contains(&2));
    }

    #[test]
    fn replay_after_delete_tells_duplicate_from_reuse() {
        let mut r = Seq::new();
        r.apply(&ins(1, None, 'a')).unwrap();
        r.apply(&Op::Delete { target: 1 }).unwrap();
        assert_eq!(r.apply(&ins(1, None, 'a')), Ok(Applied::Duplicate));
        assert_eq!(r.apply(&ins(1, None, 'b')), Err(Error::IdReused { id: 1 }));
        r.apply(&ins(2, None, 'c')).unwrap();
        assert_eq!(
            r.apply(&ins(1, Some(2), 'a')),
            Err(Error::IdReused { id: 1 })
        );
        assert_eq!(text(&r), "c");
    }

    #[test]
    fn a_long_chain_does_not_overflow_the_stack() {
        const N: u32 = 300_000;
        let mut r: Rga<u32, u8> = Rga::new();
        for id in 1..=N {
            let parent = id.checked_sub(1).filter(|&p| p > 0);
            r.apply(&Op::Insert {
                id,
                parent,
                value: b'a',
            })
            .unwrap();
        }
        assert_eq!(r.len(), N as usize);
        assert_eq!(r.iter().count(), N as usize);
        assert_eq!(r.id_at(N as usize - 1), Some(&N));
    }

    #[test]
    fn errors_display() {
        let e: Error<u32> = Error::NotFresh { id: 3 };
        assert_eq!(
            e.to_string(),
            "id 3 is not greater than every existing child of its parent"
        );
    }
}

//! The CRDT laws, over random multi-site histories.
//!
//! A history is made by three sites that edit their own replicas through
//! `insert_at`/`delete_at` with Lamport ids `(clock, site)` and now and then
//! sync. The log of ops, in authoring order, is one causal order; the laws
//! replay it in others, in batches and with duplicates.
#![allow(clippy::unwrap_used)]

use proptest::prelude::*;
use rusty_crdt::{Applied, Error, Op, Rga};

type Id = (u32, u8);
type Seq = Rga<Id, u8>;
type Edit = Op<Id, u8>;

const SITES: u8 = 3;

/// A small deterministic generator, so a proptest seed fixes a whole history.
struct Gen(u64);

impl Gen {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n.max(1) as u64) as usize
    }

    fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            v.swap(i, self.below(i + 1));
        }
    }
}

fn gen(seed: u64) -> Gen {
    Gen(seed | 1)
}

/// The ops of a random history, in authoring order.
fn history(g: &mut Gen, steps: usize) -> Vec<Edit> {
    let mut sites: Vec<(Seq, u32)> = (0..SITES).map(|_| (Seq::new(), 0)).collect();
    let mut log: Vec<Edit> = Vec::new();
    for _ in 0..steps {
        let s = g.below(SITES as usize);
        let (replica, clock) = &mut sites[s];
        if g.below(5) == 0 {
            assert!(deliver(replica, &log).is_empty());
            *clock = log
                .iter()
                .filter_map(insert_clock)
                .max()
                .unwrap_or(0)
                .max(*clock);
            continue;
        }
        let op = if !replica.is_empty() && g.below(3) == 0 {
            replica.delete_at(g.below(replica.len())).unwrap()
        } else {
            *clock += 1;
            let value = b'a' + g.below(26) as u8;
            replica
                .insert_at(g.below(replica.len() + 1), (*clock, s as u8), value)
                .unwrap()
        };
        assert_eq!(replica.apply(&op), Ok(Applied::Changed));
        log.push(op);
    }
    log
}

fn insert_clock(op: &Edit) -> Option<u32> {
    match op {
        Op::Insert { id, .. } => Some(id.0),
        Op::Delete { .. } => None,
    }
}

/// Apply `ops` in the given order, retrying the pending ones until nothing
/// more applies. Returns the ops still pending.
fn deliver(r: &mut Seq, ops: &[Edit]) -> Vec<Edit> {
    let mut waiting: Vec<Edit> = ops.to_vec();
    loop {
        let before = waiting.len();
        waiting.retain(|op| matches!(r.apply(op).unwrap(), Applied::Pending { .. }));
        if waiting.len() == before {
            return waiting;
        }
    }
}

fn replayed(ops: &[Edit]) -> Seq {
    let mut r = Seq::new();
    let stuck = deliver(&mut r, ops);
    assert!(stuck.is_empty(), "ops never became ready: {stuck:?}");
    r
}

fn seq(r: &Seq) -> Vec<(Id, u8)> {
    let out: Vec<(Id, u8)> = r.iter().map(|(id, v)| (*id, *v)).collect();
    assert_eq!(out.len(), r.len(), "len agrees with the walk");
    out
}

proptest! {
    /// Convergence: any delivery order, with duplicates, gives the same sequence.
    #[test]
    fn any_order_with_duplicates_converges(seed in any::<u64>(), steps in 1usize..80) {
        let mut g = gen(seed);
        let log = history(&mut g, steps);
        let want = seq(&replayed(&log));
        let mut mixed = log.clone();
        for _ in 0..g.below(log.len() + 1) {
            mixed.push(log[g.below(log.len())].clone());
        }
        g.shuffle(&mut mixed);
        prop_assert_eq!(seq(&replayed(&mixed)), want);
    }

    /// Idempotence: re-applying any op is a duplicate and changes nothing.
    #[test]
    fn reapplying_is_a_duplicate(seed in any::<u64>(), steps in 1usize..80) {
        let log = history(&mut gen(seed), steps);
        let mut r = replayed(&log);
        let want = seq(&r);
        for op in &log {
            prop_assert_eq!(r.apply(op), Ok(Applied::Duplicate));
        }
        prop_assert_eq!(seq(&r), want);
    }

    /// Commutativity: two ops that are both ready on a state commute.
    #[test]
    fn ready_ops_commute(seed in any::<u64>(), steps in 2usize..80) {
        let mut g = gen(seed);
        let log = history(&mut g, steps);
        if log.is_empty() {
            return Ok(());
        }
        let k = g.below(log.len());
        let base = replayed(&log[..k]);
        let ready: Vec<&Edit> = log[k..]
            .iter()
            .filter(|op| !matches!(base.clone().apply(op), Ok(Applied::Pending { .. })))
            .collect();
        let (a, b) = (ready[g.below(ready.len())], ready[g.below(ready.len())]);
        let mut ab = base.clone();
        ab.apply(a).unwrap();
        ab.apply(b).unwrap();
        let mut ba = base;
        ba.apply(b).unwrap();
        ba.apply(a).unwrap();
        prop_assert_eq!(seq(&ab), seq(&ba));
    }

    /// Associativity: split into three batches, every grouping and order of
    /// the batches reaches the same sequence.
    #[test]
    fn batch_grouping_does_not_matter(seed in any::<u64>(), steps in 1usize..80) {
        let mut g = gen(seed);
        let log = history(&mut g, steps);
        let want = seq(&replayed(&log));
        let mut batches: [Vec<Edit>; 3] = Default::default();
        for op in &log {
            batches[g.below(3)].push(op.clone());
        }
        for order in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
            let mut r = Seq::new();
            // (x then y) then z, as two deliveries.
            let first: Vec<Edit> = batches[order[0]].iter().chain(&batches[order[1]]).cloned().collect();
            // Ops of one batch may wait on a later one: carry them over.
            let mut rest = deliver(&mut r, &first);
            rest.extend(batches[order[2]].iter().cloned());
            prop_assert!(deliver(&mut r, &rest).is_empty());
            prop_assert_eq!(seq(&r), want.clone());
        }
    }

    /// Tombstones: a deleted element stays held, a replayed insert of it is a
    /// duplicate, and the same id with another value is refused unchanged.
    #[test]
    fn replays_after_delete_tell_duplicate_from_reuse(seed in any::<u64>(), steps in 1usize..80) {
        let log = history(&mut gen(seed), steps);
        let mut r = replayed(&log);
        let want = seq(&r);
        for op in &log {
            match op {
                Op::Delete { target } => {
                    prop_assert!(r.contains(target));
                    prop_assert!(want.iter().all(|(id, _)| id != target));
                }
                Op::Insert { id, parent, value } => {
                    let other = Op::Insert { id: *id, parent: *parent, value: value ^ 0x20 };
                    prop_assert_eq!(r.apply(&other), Err(Error::IdReused { id: *id }));
                    prop_assert_eq!(r.apply(op), Ok(Applied::Duplicate));
                }
            }
        }
        prop_assert_eq!(seq(&r), want);
    }

    /// Single-site model and intention: positional edits on one replica
    /// match the same edits on a `Vec`.
    #[test]
    fn one_site_matches_a_vec(edits in proptest::collection::vec((any::<bool>(), any::<u16>(), any::<u8>()), 0..120)) {
        let mut r = Seq::new();
        let mut model: Vec<u8> = Vec::new();
        for (n, (delete, at, value)) in edits.into_iter().enumerate() {
            let op = if delete && !model.is_empty() {
                let pos = at as usize % model.len();
                model.remove(pos);
                r.delete_at(pos).unwrap()
            } else {
                let pos = at as usize % (model.len() + 1);
                model.insert(pos, value);
                r.insert_at(pos, (n as u32 + 1, 0), value).unwrap()
            };
            r.apply(&op).unwrap();
        }
        let got: Vec<u8> = r.iter().map(|(_, v)| *v).collect();
        prop_assert_eq!(got, model);
    }

    /// No panic: arbitrary ops over a small id space never panic, a refused
    /// op changes nothing, and `len` always matches the walk.
    #[test]
    fn garbage_never_panics(ops in proptest::collection::vec((any::<bool>(), 0u32..8, proptest::option::of(0u32..8), any::<u8>()), 0..60)) {
        let mut r: Rga<u32, u8> = Rga::new();
        for (delete, id, parent, value) in ops {
            let op = if delete { Op::Delete { target: id } } else { Op::Insert { id, parent, value } };
            let before: Vec<(u32, u8)> = r.iter().map(|(i, v)| (*i, *v)).collect();
            match r.apply(&op) {
                Ok(Applied::Changed) => {}
                Ok(_) | Err(_) => {
                    let after: Vec<(u32, u8)> = r.iter().map(|(i, v)| (*i, *v)).collect();
                    prop_assert_eq!(after, before);
                }
            }
            prop_assert_eq!(r.iter().count(), r.len());
        }
    }
}

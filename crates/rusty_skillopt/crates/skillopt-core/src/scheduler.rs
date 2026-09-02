use std::collections::VecDeque;

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;

use crate::types::Example;

/// Deterministically shuffles the training set once per epoch and hands out
/// fixed-size batches. Deterministic given `seed` so runs are reproducible.
pub struct BatchScheduler {
    rng: StdRng,
    batch_size: usize,
}

impl BatchScheduler {
    pub fn new(seed: u64, batch_size: usize) -> Self {
        Self {
            rng: StdRng::seed_from_u64(seed),
            batch_size: batch_size.max(1),
        }
    }

    /// Returns the training examples split into shuffled batches for one
    /// epoch.
    pub fn epoch_batches<'a>(&mut self, examples: &'a [Example]) -> Vec<Vec<&'a Example>> {
        let mut order: Vec<&Example> = examples.iter().collect();
        order.shuffle(&mut self.rng);
        order.chunks(self.batch_size).map(|c| c.to_vec()).collect()
    }
}

/// Bounded FIFO of rejected-edit rationales, surfaced to the optimizer so it
/// avoids re-proposing changes the validation gate already turned down.
#[derive(Debug, Default)]
pub struct RejectionBuffer {
    capacity: usize,
    entries: VecDeque<String>,
}

impl RejectionBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::with_capacity(capacity),
        }
    }

    pub fn push(&mut self, rationale: String) {
        if self.capacity == 0 {
            return;
        }
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(rationale);
    }

    pub fn entries(&self) -> impl Iterator<Item = &String> {
        self.entries.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn examples(n: usize) -> Vec<Example> {
        (0..n)
            .map(|i| Example {
                id: format!("ex{i}"),
                input: String::new(),
                expected: String::new(),
            })
            .collect()
    }

    #[test]
    fn batches_cover_all_examples_exactly_once() {
        let ex = examples(10);
        let mut sched = BatchScheduler::new(42, 3);
        let batches = sched.epoch_batches(&ex);
        let total: usize = batches.iter().map(|b| b.len()).sum();
        assert_eq!(total, 10);
        assert_eq!(batches.len(), 4); // 3,3,3,1
    }

    #[test]
    fn deterministic_given_same_seed() {
        let ex = examples(10);
        let mut s1 = BatchScheduler::new(7, 3);
        let mut s2 = BatchScheduler::new(7, 3);
        let b1: Vec<Vec<&str>> = s1
            .epoch_batches(&ex)
            .iter()
            .map(|b| b.iter().map(|e| e.id.as_str()).collect())
            .collect();
        let b2: Vec<Vec<&str>> = s2
            .epoch_batches(&ex)
            .iter()
            .map(|b| b.iter().map(|e| e.id.as_str()).collect())
            .collect();
        assert_eq!(b1, b2);
    }

    #[test]
    fn rejection_buffer_evicts_oldest() {
        let mut buf = RejectionBuffer::new(2);
        buf.push("a".into());
        buf.push("b".into());
        buf.push("c".into());
        let entries: Vec<&String> = buf.entries().collect();
        assert_eq!(entries, vec!["b", "c"]);
    }

    #[test]
    fn zero_capacity_buffer_stays_empty() {
        let mut buf = RejectionBuffer::new(0);
        buf.push("a".into());
        assert_eq!(buf.entries().count(), 0);
    }
}

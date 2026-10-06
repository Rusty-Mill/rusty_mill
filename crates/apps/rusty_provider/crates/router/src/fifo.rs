//! `FifoMap` -- the one insertion-order-evicting bounded map behind this
//! crate's three recent-history caches (`GenerationCache`,
//! `ReasoningReplayCache`, `cache::ResponseCache`), which used to carry the
//! same `VecDeque` + `HashMap` eviction block each. Not an LRU: a lookup
//! never refreshes an entry's position, and re-inserting a present key
//! overwrites its value without taking a new slot. Crate-private on
//! purpose: `rp-core`'s `RateLimiter` has the same shape but sharing
//! across the crate boundary would need a public API this crate does not
//! want to commit to yet.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

/// Fixed-capacity map that evicts its oldest-inserted key once full. The
/// production surface is `with_capacity`, `insert`, `get` and `remove`;
/// the inspection accessors exist for tests only.
#[derive(Debug)]
pub(crate) struct FifoMap<K, V> {
    capacity: usize,
    order: VecDeque<K>,
    entries: HashMap<K, V>,
}

impl<K: Eq + Hash + Clone, V> FifoMap<K, V> {
    /// A map holding at most `capacity` entries. A capacity of zero is
    /// normalised to one, so an insert always succeeds and the newest
    /// entry is always retrievable.
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            order: VecDeque::new(),
            entries: HashMap::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(crate) fn contains_key(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    pub(crate) fn get(&self, key: &K) -> Option<&V> {
        self.entries.get(key)
    }

    /// Inserts or overwrites. A new key takes a slot and, once the map is
    /// over capacity, the oldest-inserted key is evicted; an existing key
    /// keeps its slot and position.
    pub(crate) fn insert(&mut self, key: K, value: V) {
        if !self.entries.contains_key(&key) {
            self.order.push_back(key.clone());
            if self.order.len() > self.capacity {
                if let Some(oldest) = self.order.pop_front() {
                    self.entries.remove(&oldest);
                }
            }
        }
        self.entries.insert(key, value);
    }

    /// Removes `key` from both the map and the eviction queue, so a later
    /// re-insert of the same key cannot leave a stale queue entry that
    /// would make the next eviction pop the wrong key.
    pub(crate) fn remove(&mut self, key: &K) -> Option<V> {
        let removed = self.entries.remove(key)?;
        self.order.retain(|k| k != key);
        Some(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_the_oldest_key_once_over_capacity() {
        let mut map = FifoMap::with_capacity(3);
        for k in ["a", "b", "c"] {
            map.insert(k, k.len());
        }
        assert_eq!(map.len(), 3);
        map.insert("d", 1);
        assert_eq!(map.len(), 3);
        assert!(!map.contains_key(&"a"), "oldest key evicted");
        assert!(map.contains_key(&"b") && map.contains_key(&"c") && map.contains_key(&"d"));
    }

    #[test]
    fn reinserting_a_present_key_overwrites_without_taking_a_slot() {
        let mut map = FifoMap::with_capacity(2);
        map.insert("a", 1);
        map.insert("b", 2);
        map.insert("a", 10);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(&"a"), Some(&10));
        // "a" kept its original position, so it is still the oldest.
        map.insert("c", 3);
        assert!(!map.contains_key(&"a"));
        assert!(map.contains_key(&"b") && map.contains_key(&"c"));
    }

    #[test]
    fn remove_cleans_the_queue_so_eviction_order_stays_correct() {
        let mut map = FifoMap::with_capacity(2);
        map.insert("a", 1);
        map.insert("b", 2);
        assert_eq!(map.remove(&"a"), Some(1));
        assert_eq!(map.remove(&"a"), None);
        // Re-insert "a": it is now the newest. Inserting "c" must evict
        // "b", the true oldest, not a stale queue reference to "a".
        map.insert("a", 3);
        map.insert("c", 4);
        assert!(!map.contains_key(&"b"));
        assert!(map.contains_key(&"a") && map.contains_key(&"c"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn zero_capacity_is_normalised_to_one() {
        let mut map = FifoMap::with_capacity(0);
        assert_eq!(map.capacity(), 1);
        map.insert("a", 1);
        map.insert("b", 2);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get(&"b"), Some(&2));
    }
}

//! The HPACK dynamic table (RFC 7541 §2.3.2, §4).

use std::collections::VecDeque;

/// Per RFC 7541 §4.1: each entry's size is its name and value lengths plus
/// 32 bytes of accounting overhead.
const ENTRY_OVERHEAD: usize = 32;

fn entry_size(name: &[u8], value: &[u8]) -> usize {
    name.len() + value.len() + ENTRY_OVERHEAD
}

/// A single encoder/decoder's dynamic table. Front of the deque is the most
/// recently inserted entry (combined index `STATIC_TABLE.len() + 1`).
#[derive(Debug, Default)]
pub struct DynamicTable {
    entries: VecDeque<(Vec<u8>, Vec<u8>)>,
    size: usize,
    max_size: usize,
}

impl DynamicTable {
    pub fn new(max_size: usize) -> Self {
        DynamicTable {
            entries: VecDeque::new(),
            size: 0,
            max_size,
        }
    }

    pub fn max_size(&self) -> usize {
        self.max_size
    }

    /// Apply a new maximum size (from a SETTINGS change or an in-band
    /// Dynamic Table Size Update instruction), evicting as needed.
    pub fn set_max_size(&mut self, max_size: usize) {
        self.max_size = max_size;
        self.evict_to_fit();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Current total size, per the RFC 7541 §4.1 accounting rule.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Look up a 1-based index within the dynamic table only (the caller
    /// is responsible for subtracting the static table's length first).
    pub fn get(&self, index: usize) -> Option<(&[u8], &[u8])> {
        if index == 0 {
            return None;
        }
        self.entries
            .get(index - 1)
            .map(|(n, v)| (n.as_slice(), v.as_slice()))
    }

    /// Insert a new entry. Per RFC 7541 §4.4, an entry larger than the
    /// table's maximum size empties the table instead of being stored.
    pub fn insert(&mut self, name: Vec<u8>, value: Vec<u8>) {
        let size = entry_size(&name, &value);
        if size > self.max_size {
            self.entries.clear();
            self.size = 0;
            return;
        }
        self.entries.push_front((name, value));
        self.size += size;
        self.evict_to_fit();
    }

    fn evict_to_fit(&mut self) {
        while self.size > self.max_size {
            let Some((name, value)) = self.entries.pop_back() else {
                break;
            };
            self.size -= entry_size(&name, &value);
        }
    }

    /// Find the best match for `(name, value)`, mirroring
    /// `static_table::find`'s `(index, exact_match)` contract, with indices
    /// already offset into the dynamic portion of the combined space.
    pub fn find(&self, name: &[u8], value: &[u8]) -> Option<(usize, bool)> {
        let mut name_only: Option<usize> = None;
        for (i, (n, v)) in self.entries.iter().enumerate() {
            if n.as_slice() == name {
                if v.as_slice() == value {
                    return Some((i + 1, true));
                }
                if name_only.is_none() {
                    name_only = Some(i + 1);
                }
            }
        }
        name_only.map(|idx| (idx, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_lookup() {
        let mut t = DynamicTable::new(4096);
        t.insert(b"custom-key".to_vec(), b"custom-value".to_vec());
        assert_eq!(t.len(), 1);
        assert_eq!(
            t.get(1),
            Some((b"custom-key".as_slice(), b"custom-value".as_slice()))
        );
        assert_eq!(t.size(), entry_size(b"custom-key", b"custom-value"));
    }

    #[test]
    fn most_recent_entry_is_index_one() {
        let mut t = DynamicTable::new(4096);
        t.insert(b"a".to_vec(), b"1".to_vec());
        t.insert(b"b".to_vec(), b"2".to_vec());
        assert_eq!(t.get(1), Some((b"b".as_slice(), b"2".as_slice())));
        assert_eq!(t.get(2), Some((b"a".as_slice(), b"1".as_slice())));
    }

    #[test]
    fn eviction_on_overflow() {
        let mut t = DynamicTable::new(entry_size(b"a", b"1"));
        t.insert(b"a".to_vec(), b"1".to_vec());
        t.insert(b"b".to_vec(), b"2".to_vec());
        // Only the most recent entry should remain; "a" is evicted.
        assert_eq!(t.len(), 1);
        assert_eq!(t.get(1), Some((b"b".as_slice(), b"2".as_slice())));
    }

    #[test]
    fn entry_larger_than_max_empties_table() {
        let mut t = DynamicTable::new(50);
        t.insert(b"a".to_vec(), b"1".to_vec());
        assert_eq!(t.len(), 1);
        t.insert(vec![0u8; 100], vec![]);
        assert_eq!(t.len(), 0);
        assert_eq!(t.size(), 0);
    }

    #[test]
    fn shrinking_max_size_evicts() {
        let mut t = DynamicTable::new(4096);
        t.insert(b"a".to_vec(), b"1".to_vec());
        t.insert(b"b".to_vec(), b"2".to_vec());
        t.set_max_size(0);
        assert_eq!(t.len(), 0);
    }
}

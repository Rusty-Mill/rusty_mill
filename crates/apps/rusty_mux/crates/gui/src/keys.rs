//! The window's keys for server ids: pane keys for blocks, tab keys for
//! windows.

use std::collections::HashMap;
use std::hash::Hash;

use rusty_mux_core::BlockId;

/// Each id seen gets the next key and keeps it, so a pane outlives a
/// layout change that keeps its block.
#[derive(Debug)]
pub struct Keys<T = BlockId> {
    keys: HashMap<T, u64>,
    ids: HashMap<u64, T>,
}

impl<T> Default for Keys<T> {
    fn default() -> Self {
        Self {
            keys: HashMap::new(),
            ids: HashMap::new(),
        }
    }
}

impl<T: Copy + Eq + Hash> Keys<T> {
    /// `id`'s key, given one if new.
    pub fn key(&mut self, id: T) -> u64 {
        if let Some(&key) = self.keys.get(&id) {
            return key;
        }
        let key = self.keys.len() as u64;
        self.keys.insert(id, key);
        self.ids.insert(key, id);
        key
    }

    /// The id `key` names, if any.
    pub fn id(&self, key: u64) -> Option<T> {
        self.ids.get(&key).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::Keys;
    use rusty_mux_core::BlockId;
    use rusty_uuid::Uuid;

    #[test]
    fn a_block_keeps_its_key_and_keys_name_their_block() {
        let (a, b) = (
            BlockId::from_uuid(Uuid::new_v4()),
            BlockId::from_uuid(Uuid::new_v4()),
        );
        let mut keys = Keys::default();
        let (ka, kb) = (keys.key(a), keys.key(b));
        assert_ne!(ka, kb);
        assert_eq!(keys.key(a), ka);
        assert_eq!((keys.id(ka), keys.id(kb)), (Some(a), Some(b)));
        assert_eq!(keys.id(99), None);
    }
}

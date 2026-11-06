//! The window's pane keys for server blocks.

use std::collections::HashMap;

use rusty_mux_core::BlockId;

/// Each block seen gets the next key and keeps it, so a pane outlives a
/// layout change that keeps its block.
#[derive(Debug, Default)]
pub struct Keys {
    keys: HashMap<BlockId, u64>,
    blocks: HashMap<u64, BlockId>,
}

impl Keys {
    /// `block`'s key, given one if new.
    pub fn key(&mut self, block: BlockId) -> u64 {
        if let Some(&key) = self.keys.get(&block) {
            return key;
        }
        let key = self.keys.len() as u64;
        self.keys.insert(block, key);
        self.blocks.insert(key, block);
        key
    }

    /// The block `key` names, if any.
    pub fn block(&self, key: u64) -> Option<BlockId> {
        self.blocks.get(&key).copied()
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
        assert_eq!((keys.block(ka), keys.block(kb)), (Some(a), Some(b)));
        assert_eq!(keys.block(99), None);
    }
}

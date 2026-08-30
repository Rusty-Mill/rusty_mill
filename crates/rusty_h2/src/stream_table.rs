/// Stub for stream table.
use std::collections::BTreeMap;

/// A table of streams keyed by stream ID.
pub struct StreamTable {
    streams: BTreeMap<u32, crate::connect::StreamEntry>,
}

impl StreamTable {
    /// Create a new stream table.
    pub fn new() -> Self {
        StreamTable {
            streams: BTreeMap::new(),
        }
    }

    /// Insert a stream.
    pub fn insert(&mut self, stream_id: u32, entry: crate::connect::StreamEntry) {
        self.streams.insert(stream_id, entry);
    }

    /// Get a stream.
    pub fn get(&self, stream_id: u32) -> Option<&crate::connect::StreamEntry> {
        self.streams.get(&stream_id)
    }

    /// Get a mutable stream.
    pub fn get_mut(&mut self, stream_id: u32) -> Option<&mut crate::connect::StreamEntry> {
        self.streams.get_mut(&stream_id)
    }

    /// Remove a stream.
    pub fn remove(&mut self, stream_id: u32) -> Option<crate::connect::StreamEntry> {
        self.streams.remove(&stream_id)
    }

    pub fn len(&self) -> usize {
        self.streams.len()
    }
}

impl Default for StreamTable {
    fn default() -> Self {
        Self::new()
    }
}

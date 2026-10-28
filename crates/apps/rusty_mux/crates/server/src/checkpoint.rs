//! RTCP v5 attach validation and bounded chunk assembly.
use rusty_mux_core::Size;
use rusty_term_core::{AnsiParser, Grid};

/// Maximum payload in a Checkpoint chunk.
pub const CHUNK_BYTES: usize = 64 << 10;
/// Maximum live-screen cells (alternate screen can double this).
pub const MAX_SCREEN_CELLS: usize = 65_536;
/// Maximum possible retained cells, including normal 10,000-row history.
pub const MAX_RETAINED_CELLS: usize = 4 << 20;

/// Validate dimensions before spawning, allocating, or mutating a terminal.
pub fn validate_size(size: Size) -> Result<(), String> {
    let (cols, rows) = (usize::from(size.cols()), usize::from(size.rows()));
    if cols == 0
        || rows == 0
        || cols > 4096
        || rows > 4096
        || cols * rows > MAX_SCREEN_CELLS
        || cols * (10_000 + 2 * rows) > MAX_RETAINED_CELLS
    {
        return Err("terminal dimensions exceed 4096 per axis, 65536 screen cells or 4194304 retained cells including history".into());
    }
    Ok(())
}
#[cfg(any(target_os = "linux", windows))]
pub(crate) fn grid(size: Size) -> Grid {
    Grid::new(size.cols().into(), size.rows().into())
}
/// Decode one complete RTCP v5 headless checkpoint.
pub fn restore(data: &[u8]) -> Result<(Grid, AnsiParser), String> {
    rusty_term_core::checkpoint::decode(data).map_err(|e| e.to_string())
}

/// Bounded receiver for one negotiated checkpoint. Invalid chunks invalidate
/// the entire assembly; Ready is accepted only after the declared total.
#[derive(Default)]
pub struct Receiver {
    data: Vec<u8>,
    total: Option<usize>,
    failed: bool,
}
impl Receiver {
    /// Add an ordered chunk, validating total, offset and encoded-size budget.
    pub fn push(&mut self, offset: u64, total: u64, bytes: &[u8]) -> Result<(), String> {
        let result = self.push_checked(offset, total, bytes);
        if result.is_err() {
            self.data = Vec::new();
            self.failed = true;
        }
        result
    }
    fn push_checked(&mut self, offset: u64, total: u64, bytes: &[u8]) -> Result<(), String> {
        let total = usize::try_from(total).map_err(|_| "checkpoint total overflow")?;
        if self.failed
            || total == 0
            || total > rusty_term_core::checkpoint::MAX_ENCODED
            || bytes.is_empty()
            || bytes.len() > CHUNK_BYTES
            || offset != self.data.len() as u64
            || self.total.is_some_and(|n| n != total)
            || bytes.len() > total.saturating_sub(self.data.len())
        {
            return Err("invalid checkpoint chunk/budget".into());
        }
        self.total = Some(total);
        let needed = self.data.len() + bytes.len();
        if needed > self.data.capacity() {
            let capacity = needed
                .max(self.data.capacity().saturating_mul(2))
                .min(total);
            self.data
                .try_reserve_exact(capacity - self.data.len())
                .map_err(|_| "checkpoint allocation failed")?;
        }
        self.data.extend_from_slice(bytes);
        Ok(())
    }
    /// Consume on Ready, reject incomplete transfers and decode durable state.
    pub fn finish(self) -> Result<(Grid, AnsiParser), String> {
        if self.failed || self.total != Some(self.data.len()) {
            return Err("incomplete checkpoint".into());
        }
        restore(&self.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_chunks_and_incomplete_transfers_are_rejected() {
        for (offset, total, data) in [
            (1, 8, vec![0]),
            (0, 0, vec![0]),
            (0, u64::MAX, vec![0]),
            (0, 8, vec![]),
            (0, 1, vec![0, 0]),
            (0, 100_000, vec![0; CHUNK_BYTES + 1]),
        ] {
            let mut receiver = Receiver::default();
            assert!(receiver.push(offset, total, &data).is_err());
            assert!(receiver.finish().is_err());
        }
        let mut receiver = Receiver::default();
        receiver.push(0, 8, b"1234").unwrap();
        assert!(receiver.finish().is_err());
        let mut receiver = Receiver::default();
        receiver.push(0, 8, b"1234").unwrap();
        assert!(receiver.push(4, 9, b"5678").is_err());
        assert!(receiver.push(0, 8, b"12345678").is_err());
    }
}

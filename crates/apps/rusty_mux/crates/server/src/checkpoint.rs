//! Exact, bounded Phase 1 continuation checkpoints.
//!
//! RMRP v1 replays PTY bytes and resizes from a fresh default terminal. This
//! preserves *all* parser/grid continuation state, including images and input
//! modes, without misrepresenting the drawable RTSN format as a checkpoint.
//! Requires `Features::REPLAY_SNAPSHOT`; see docs/projects/mux/REPLAY.md.

use rusty_mux_core::Size;
use rusty_term_core::{AnsiParser, Grid};

/// Maximum encoded replay bytes retained per block (including record headers).
pub const MAX_REPLAY: usize = 4 << 20;
/// Maximum cells in a screen. The alternate screen can double this allocation.
pub const MAX_SCREEN_CELLS: usize = 65_536;
/// Scrollback rows; even at the maximum width this retains at most 65,536 cells.
pub const SCROLLBACK_ROWS: usize = 16;
const HEADER: &[u8] = b"RMRP\x01";

/// Validate before spawning, resizing a PTY, or allocating a terminal.
pub fn validate_size(size: Size) -> Result<(), String> {
    let (cols, rows) = (usize::from(size.cols()), usize::from(size.rows()));
    if cols == 0 || rows == 0 || cols > 4096 || rows > 4096 || cols * rows > MAX_SCREEN_CELLS {
        return Err("terminal dimensions exceed 4096 per axis or 65536 screen cells".into());
    }
    Ok(())
}

pub(crate) fn grid(size: Size) -> Grid {
    let mut grid = Grid::new(usize::from(size.cols()), usize::from(size.rows()));
    grid.set_scrollback_max(SCROLLBACK_ROWS);
    grid
}

pub(crate) struct Replay {
    bytes: Option<Vec<u8>>,
}

impl Replay {
    pub(crate) fn new(size: Size) -> Self {
        let mut bytes = HEADER.to_vec();
        bytes.extend(size.cols().to_le_bytes());
        bytes.extend(size.rows().to_le_bytes());
        Self { bytes: Some(bytes) }
    }

    pub(crate) fn available(&self) -> bool {
        self.bytes.is_some()
    }
    pub(crate) fn snapshot(&self) -> Option<Vec<u8>> {
        self.bytes.clone()
    }

    fn record(&mut self, tag: u8, payload: &[u8]) {
        let Some(bytes) = self.bytes.as_mut() else {
            return;
        };
        if bytes.len() + 5 + payload.len() > MAX_REPLAY {
            self.bytes = None;
            return;
        }
        bytes.push(tag);
        bytes.extend((payload.len() as u32).to_le_bytes());
        bytes.extend(payload);
    }

    pub(crate) fn output(&mut self, data: &[u8]) {
        self.record(0, data);
    }
    pub(crate) fn resize(&mut self, size: Size) {
        let mut data = size.cols().to_le_bytes().to_vec();
        data.extend(size.rows().to_le_bytes());
        self.record(1, &data);
    }
}

/// Restore the complete terminal continuation. Replayed terminal responses are
/// discarded, never sent to the PTY again. Unknown versions/records, invalid
/// sizes, oversized payloads and truncation fail closed.
pub fn restore(data: &[u8]) -> Result<(Grid, AnsiParser), String> {
    if data.len() > MAX_REPLAY || !data.starts_with(HEADER) {
        return Err("invalid replay header or size".into());
    }
    let size = |data: &[u8]| -> Result<Size, String> {
        let raw: [u8; 4] = data.try_into().map_err(|_| "invalid replay dimensions")?;
        let size = Size::new(
            u16::from_le_bytes([raw[0], raw[1]]),
            u16::from_le_bytes([raw[2], raw[3]]),
        )
        .map_err(|e| e.to_string())?;
        validate_size(size)?;
        Ok(size)
    };
    let mut rest = &data[HEADER.len()..];
    let initial = rest.get(..4).ok_or("truncated replay dimensions")?;
    let mut grid = grid(size(initial)?);
    let mut parser = AnsiParser::new();
    rest = &rest[4..];
    while !rest.is_empty() {
        let header = rest.get(..5).ok_or("truncated replay record")?;
        let len = u32::from_le_bytes(
            header[1..5]
                .try_into()
                .map_err(|_| "invalid replay length")?,
        ) as usize;
        let payload = rest
            .get(5..)
            .and_then(|r| r.get(..len))
            .ok_or("truncated replay payload")?;
        match header[0] {
            0 => {
                parser.advance(&mut grid, payload);
                parser.take_responses();
            }
            1 => {
                let size = size(payload)?;
                grid.resize(size.cols().into(), size.rows().into());
            }
            _ => return Err("unknown replay record".into()),
        }
        rest = &rest[5 + len..];
    }
    Ok((grid, parser))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_resizes_and_rejects_corruption_and_overflow() {
        let mut replay = Replay::new(Size::new(80, 24).unwrap());
        replay.output(b"\x1b[31mhello\x1b[");
        replay.resize(Size::new(40, 12).unwrap());
        replay.output(b"32mX");
        let data = replay.snapshot().unwrap();
        let (grid, _) = restore(&data).unwrap();
        assert_eq!((grid.cols, grid.rows), (40, 12));
        // A truncation on a complete record boundary is valid; inside a header
        // or payload it must fail. Unknown versions and tags always fail.
        for len in [0, 4, 5, 8, data.len() - 1] {
            assert!(restore(&data[..len]).is_err());
        }
        let mut bad = data.clone();
        bad[4] = 2;
        assert!(restore(&bad).is_err());
        let mut bad = data.clone();
        bad[9] = 2;
        assert!(restore(&bad).is_err());
        let mut bad = data.clone();
        bad[5..7].copy_from_slice(&4097u16.to_le_bytes());
        assert!(restore(&bad).is_err());
        replay.output(&vec![0; MAX_REPLAY]);
        assert!(!replay.available());
        assert!(replay.snapshot().is_none());
        assert!(restore(&vec![0; MAX_REPLAY + 1]).is_err());
    }
}

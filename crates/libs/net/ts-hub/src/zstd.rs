//! A minimal zstd *encoder*: one frame of raw (stored) blocks (RFC 8878).
//! Any conforming decoder accepts it, so the hub can answer a node that
//! asked for `Compress: "zstd"` without a compressor dependency. Map frames
//! are small, so the lost ratio costs nothing here.

const MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
/// Largest block a frame may carry.
const MAX_BLOCK: usize = 128 << 10;

/// `data` as a single zstd frame of raw blocks, with the content size set.
pub fn frame(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    out.extend_from_slice(&MAGIC);
    // Frame header: single segment, so no window byte; the content size
    // takes 1, 2 or 4 bytes (2-byte values are stored minus 256).
    match data.len() {
        n if n <= 0xff => out.extend_from_slice(&[0x20, n as u8]),
        n if n <= 0xff + 0xffff => {
            out.push(0x60);
            out.extend_from_slice(&((n - 256) as u16).to_le_bytes());
        }
        n => {
            out.push(0xa0);
            out.extend_from_slice(&(n as u32).to_le_bytes());
        }
    }
    let mut blocks = data.chunks(MAX_BLOCK).peekable();
    if blocks.peek().is_none() {
        // An empty frame still needs one (last, empty, raw) block.
        out.extend_from_slice(&[0x01, 0x00, 0x00]);
    }
    while let Some(block) = blocks.next() {
        let last = u32::from(blocks.peek().is_none());
        // Block header: last flag, type 0 (raw), 21-bit size.
        let header = last | ((block.len() as u32) << 3);
        out.extend_from_slice(&header.to_le_bytes()[..3]);
        out.extend_from_slice(block);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_frame_has_the_documented_layout() {
        assert_eq!(
            frame(b"hello"),
            [
                0x28, 0xb5, 0x2f, 0xfd, // magic
                0x20, 0x05, // single segment, content size 5
                0x29, 0x00, 0x00, // last raw block of 5 bytes: 1 | 5 << 3
                b'h', b'e', b'l', b'l', b'o',
            ]
        );
    }

    #[test]
    fn an_empty_input_still_makes_a_valid_frame() {
        assert_eq!(
            frame(b""),
            [0x28, 0xb5, 0x2f, 0xfd, 0x20, 0x00, 0x01, 0x00, 0x00]
        );
    }

    #[test]
    fn sizes_pick_the_right_content_size_field() {
        let mid = frame(&[7u8; 300]);
        assert_eq!(&mid[4..7], &[0x60, 44, 0]); // 300 - 256
        let big = frame(&vec![7u8; 300_000]);
        assert_eq!(big[4], 0xa0);
        assert_eq!(&big[5..9], &300_000u32.to_le_bytes());
    }

    #[test]
    fn large_input_splits_into_blocks_with_only_the_last_flagged() {
        let data = vec![1u8; MAX_BLOCK + 10];
        let f = frame(&data);
        let first = u32::from_le_bytes([f[9], f[10], f[11], 0]);
        assert_eq!(first, (MAX_BLOCK as u32) << 3); // not last
        let tail = 9 + 3 + MAX_BLOCK;
        let last = u32::from_le_bytes([f[tail], f[tail + 1], f[tail + 2], 0]);
        assert_eq!(last, 1 | (10 << 3));
        assert_eq!(f.len(), tail + 3 + 10);
    }
}

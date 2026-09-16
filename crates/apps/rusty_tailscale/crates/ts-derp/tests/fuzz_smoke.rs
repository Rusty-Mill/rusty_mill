//! Panic-free smoke fuzzing for the DERP frame reader: `read_frame` parses a
//! length-prefixed frame off an untrusted stream, so a hostile server (or a
//! MITM before the NaCl handshake) must not be able to crash or OOM us.
//!
//! Stable-only, deterministic (xorshift seed). `&[u8]` implements tokio's
//! `AsyncRead`, so random buffers stand in for the socket.

use ts_derp::frame::read_frame;

struct Rng(u64);
impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn byte(&mut self) -> u8 {
        (self.next_u64() & 0xff) as u8
    }
    fn len(&mut self, max: usize) -> usize {
        (self.next_u64() as usize) % (max + 1)
    }
}

const MAX_FRAME: u32 = 1 << 20; // 1 MiB cap, as the real client uses

#[tokio::test]
async fn read_frame_never_panics_or_overallocates() {
    let mut rng = Rng(0x0BAD_C0DE_F00D_1971);
    for _ in 0..100_000 {
        let n = rng.len(64);
        let mut buf: Vec<u8> = (0..n).map(|_| rng.byte()).collect();
        // Sometimes forge a plausible header with a huge advertised length to
        // confirm the cap rejects it *before* allocating.
        if rng.byte() & 1 == 0 && buf.len() >= 5 {
            buf[0] = rng.byte();
            buf[1..5].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        }
        let mut slice = buf.as_slice();
        // Must yield Ok or Err — never panic, never try to allocate 4 GiB.
        let _ = read_frame(&mut slice, MAX_FRAME).await;
    }
}

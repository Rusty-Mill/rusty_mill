//! Panic-free smoke fuzzing: feed the STUN response parser a large volume of
//! pseudo-random and structure-mutated input and assert it never panics.
//!
//! Runs on stable (no libfuzzer): a deterministic xorshift PRNG drives it, so
//! a failure reproduces from the seed. The same entry point
//! (`parse_response`) is what a cargo-fuzz target would call.

use ts_stun::{TxId, binding_request, parse_response};

/// Deterministic xorshift64* PRNG — no external crate, reproducible.
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

#[test]
fn parse_response_never_panics_on_arbitrary_input() {
    let mut rng = Rng(0xF00D_BEEF_1234_5678);
    let tx = TxId([0x42; 12]);
    for _ in 0..200_000 {
        let n = rng.len(80);
        let mut buf: Vec<u8> = (0..n).map(|_| rng.byte()).collect();
        // Occasionally start from a valid-looking header so the parser gets
        // past its first length/magic checks and exercises the attribute loop.
        if rng.byte() & 1 == 0 && buf.len() >= 20 {
            buf[..20].copy_from_slice(&binding_request(tx));
            // Corrupt a random byte to explore malformed-attribute paths.
            let i = rng.len(buf.len().saturating_sub(1));
            buf[i] ^= rng.byte();
        }
        // Must return Ok or Err — never panic, never hang.
        let _ = parse_response(&buf, tx);
    }
}

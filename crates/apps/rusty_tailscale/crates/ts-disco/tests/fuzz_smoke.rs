//! Panic-free smoke fuzzing for the disco receive path: `is_disco`,
//! `source_key`, and `open` must tolerate any bytes an attacker can put on the
//! wire (disco frames arrive unauthenticated until the NaCl box opens).
//!
//! Stable-only, deterministic (xorshift seed); the same entry points a
//! cargo-fuzz target would drive.

use ts_disco::{is_disco, open, source_key};
use ts_key::DiscoPrivate;

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

/// The disco magic, so some inputs get past the `is_disco` gate.
const MAGIC: &[u8] = b"TS\xf0\x9f\x92\xac";

#[test]
fn disco_receive_path_never_panics() {
    let mut rng = Rng(0x1357_9BDF_2468_ACE0);
    let receiver = DiscoPrivate::generate();
    // Fewer iterations than the other parsers: each `open` runs a full NaCl
    // box attempt, so this is crypto-bound, not parse-bound.
    for _ in 0..40_000 {
        let n = rng.len(120);
        let mut buf: Vec<u8> = (0..n).map(|_| rng.byte()).collect();
        // Half the time, prefix the disco magic so `open` runs its key/box
        // decode against garbage (the interesting, deeper path).
        if rng.byte() & 1 == 0 {
            let mut framed = MAGIC.to_vec();
            framed.extend_from_slice(&buf);
            buf = framed;
        }
        let _ = is_disco(&buf);
        let _ = source_key(&buf);
        // Must be Err on garbage, never panic.
        let _ = open(&receiver, &buf);
    }
}

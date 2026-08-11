//! Property-style round-trip testing: generate a lot of arbitrary values
//! with a tiny hand-rolled PRNG (no `rand`/`quickcheck`/`proptest` - those
//! would be exactly the crates.io dependencies this project avoids) and
//! check `Serialize` -> `to_string` -> `from_str` -> `Deserialize` always
//! recovers the original value. Hand-picked test cases in `roundtrip.rs`
//! cover the shapes we thought of; this covers the ones we didn't -
//! particularly string escaping (control characters, quotes, backslashes,
//! surrogate-pair-requiring codepoints) and number formatting, both of
//! which are exactly the kind of thing that's easy to get subtly wrong in
//! a hand-written parser.

use std::collections::BTreeMap;

use rusty_serde::{json, Deserialize, Serialize};

/// xorshift64* - small, dependency-free, and deterministic so a failure is
/// always reproducible from the printed seed/iteration alone.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // xorshift's state must never be zero.
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn gen_range(&mut self, n: u32) -> u32 {
        (self.next_u64() % u64::from(n.max(1))) as u32
    }

    fn gen_bool(&mut self) -> bool {
        self.next_u64() & 1 == 0
    }

    fn gen_i64(&mut self) -> i64 {
        self.next_u64() as i64
    }

    fn gen_u64(&mut self) -> u64 {
        self.next_u64()
    }

    /// A finite (never NaN/infinite) f64 across a wide range of
    /// magnitudes - reinterpreting random bits as a float and retrying on
    /// the rare non-finite result.
    fn gen_f64(&mut self) -> f64 {
        loop {
            let v = f64::from_bits(self.next_u64());
            if v.is_finite() {
                return v;
            }
        }
    }

    /// A scalar value drawn from a few different interesting ranges:
    /// control characters, ASCII, Latin-1 supplement, CJK, and emoji (the
    /// last two exercise `\uXXXX` surrogate-pair escaping on the way out
    /// and back in).
    fn gen_char(&mut self) -> char {
        loop {
            let cp = match self.gen_range(5) {
                0 => self.gen_range(0x20),
                1 => 0x20 + self.gen_range(0x5F),
                2 => 0xA0 + self.gen_range(0x300),
                3 => 0x4E00 + self.gen_range(0x400),
                _ => 0x1F300 + self.gen_range(0x300),
            };
            if let Some(c) = char::from_u32(cp) {
                return c;
            }
        }
    }

    fn gen_string(&mut self, max_len: u32) -> String {
        let len = self.gen_range(max_len + 1);
        (0..len).map(|_| self.gen_char()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum FuzzValue {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Text(String),
    List(Vec<FuzzValue>),
    Map(BTreeMap<String, FuzzValue>),
}

fn gen_leaf(rng: &mut Rng) -> FuzzValue {
    match rng.gen_range(6) {
        0 => FuzzValue::Null,
        1 => FuzzValue::Bool(rng.gen_bool()),
        2 => FuzzValue::Int(rng.gen_i64()),
        3 => FuzzValue::UInt(rng.gen_u64()),
        4 => FuzzValue::Float(rng.gen_f64()),
        _ => FuzzValue::Text(rng.gen_string(16)),
    }
}

/// Builds a random tree up to `depth` levels deep, biased toward leaves so
/// it actually terminates instead of growing every branch to the cap.
fn gen_value(rng: &mut Rng, depth: u32) -> FuzzValue {
    if depth == 0 {
        return gen_leaf(rng);
    }
    match rng.gen_range(8) {
        0..=5 => gen_leaf(rng),
        6 => FuzzValue::List((0..rng.gen_range(4)).map(|_| gen_value(rng, depth - 1)).collect()),
        _ => FuzzValue::Map(
            (0..rng.gen_range(4))
                .map(|_| (rng.gen_string(8), gen_value(rng, depth - 1)))
                .collect(),
        ),
    }
}

const ITERATIONS: u64 = 2000;

#[test]
fn fuzz_roundtrip_arbitrary_values() {
    let mut rng = Rng::new(0xC0FFEE);
    for i in 0..ITERATIONS {
        let value = gen_value(&mut rng, 4);
        let json = json::to_string(&value).unwrap_or_else(|e| {
            panic!("iteration {i}: failed to encode {value:?}: {e}")
        });
        let decoded: FuzzValue = json::from_str(&json)
            .unwrap_or_else(|e| panic!("iteration {i}: failed to decode {json:?}: {e}"));
        assert_eq!(decoded, value, "iteration {i}: json was {json:?}");
    }
}

#[test]
fn fuzz_roundtrip_strings() {
    let mut rng = Rng::new(0xBADC0DE);
    for i in 0..ITERATIONS {
        let s = rng.gen_string(48);
        let json = json::to_string(&s).unwrap_or_else(|e| {
            panic!("iteration {i}: failed to encode {s:?}: {e}")
        });
        let decoded: String = json::from_str(&json)
            .unwrap_or_else(|e| panic!("iteration {i}: failed to decode {json:?}: {e}"));
        assert_eq!(decoded, s, "iteration {i}: json was {json:?}");
    }
}

#[test]
fn fuzz_roundtrip_numbers() {
    let mut rng = Rng::new(0x5EED5EED);
    for i in 0..ITERATIONS {
        let i64_val = rng.gen_i64();
        let json = json::to_string(&i64_val).unwrap();
        let decoded: i64 = json::from_str(&json)
            .unwrap_or_else(|e| panic!("iteration {i}: i64 {i64_val} -> {json:?}: {e}"));
        assert_eq!(decoded, i64_val, "iteration {i}");

        let u64_val = rng.gen_u64();
        let json = json::to_string(&u64_val).unwrap();
        let decoded: u64 = json::from_str(&json)
            .unwrap_or_else(|e| panic!("iteration {i}: u64 {u64_val} -> {json:?}: {e}"));
        assert_eq!(decoded, u64_val, "iteration {i}");

        let f64_val = rng.gen_f64();
        let json = json::to_string(&f64_val).unwrap();
        let decoded: f64 = json::from_str(&json)
            .unwrap_or_else(|e| panic!("iteration {i}: f64 {f64_val} -> {json:?}: {e}"));
        assert_eq!(
            decoded.to_bits(),
            f64_val.to_bits(),
            "iteration {i}: {f64_val} -> {json:?} -> {decoded}"
        );
    }
}

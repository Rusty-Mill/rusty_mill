//! Shared helpers.
#![allow(dead_code, clippy::unwrap_used)]

/// A tiny deterministic generator, so a failure reproduces without a dependency.
pub struct Lcg(pub u64);

impl Lcg {
    pub fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    pub fn below(&mut self, n: usize) -> usize {
        self.next() as usize % n
    }
    pub fn chance(&mut self) -> bool {
        self.next() % 2 == 0
    }
    pub fn bytes(&mut self, max: usize) -> Vec<u8> {
        (0..self.below(max + 1))
            .map(|_| self.next() as u8)
            .collect()
    }
}

/// An owned header list as the borrowed pairs the parsers take.
pub fn borrowed(pairs: &[(String, Vec<u8>)]) -> Vec<(&str, &[u8])> {
    pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_slice()))
        .collect()
}

/// `(name, value)` from literals.
pub fn h<'a>(name: &'a str, value: &'a str) -> (&'a str, &'a [u8]) {
    (name, value.as_bytes())
}

/// The value of header `name` in a list, as text.
pub fn value<'a>(pairs: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| std::str::from_utf8(v).unwrap())
}

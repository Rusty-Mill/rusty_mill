//! A small gzip encoder: LZ77 over a 32 KiB window, fixed-Huffman DEFLATE blocks.
//!
//! Enough to shrink the HTML and JSON this server sends 4–6× without a compression
//! dependency. It is an encoder only; the fixed code trades a little ratio (against
//! zlib's dynamic tables) for ~100 lines. Verified against the system `gzip -d`.

const WINDOW: usize = 32 * 1024;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
/// Hash-chain candidates tried per position: speed against ratio.
const CHAIN: usize = 24;
const HASH_BITS: u32 = 15;

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Bits go out LSB first; Huffman codes are sent most-significant bit first.
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn put(&mut self, value: u32, count: u32) {
        self.acc |= u64::from(value) << self.n;
        self.n += count;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    fn code(&mut self, code: u32, len: u32) {
        self.put(code.reverse_bits() >> (32 - len), len);
    }

    /// A literal/length symbol in the fixed code (RFC 1951 §3.2.6).
    fn symbol(&mut self, s: u32) {
        match s {
            0..=143 => self.code(0x30 + s, 8),
            144..=255 => self.code(0x190 + (s - 144), 9),
            256..=279 => self.code(s - 256, 7),
            _ => self.code(0xC0 + (s - 280), 8),
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

fn hash(d: &[u8]) -> usize {
    let v = u32::from(d[0]) | u32::from(d[1]) << 8 | u32::from(d[2]) << 16;
    (v.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
}

/// Index of the table entry covering `v` (the last base ≤ `v`).
fn slot(bases: &[u16], v: usize) -> usize {
    bases.partition_point(|&b| usize::from(b) <= v) - 1
}

const CRC_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 == 1 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(!0u32, |c, &b| {
        CRC_TABLE[usize::from((c as u8) ^ b)] ^ (c >> 8)
    })
}

/// The longest earlier match for `data[i..]` within the window: `(length, distance)`.
fn longest_match(data: &[u8], i: usize, head: &[u32], prev: &[u32]) -> Option<(usize, usize)> {
    let max = MAX_MATCH.min(data.len() - i);
    let mut best: Option<(usize, usize)> = None;
    let mut cand = head[hash(&data[i..])];
    for _ in 0..CHAIN {
        if cand == u32::MAX || i - cand as usize > WINDOW {
            break;
        }
        let c = cand as usize;
        let len = data[c..]
            .iter()
            .zip(&data[i..i + max])
            .take_while(|(a, b)| a == b)
            .count();
        if len >= MIN_MATCH && best.is_none_or(|(l, _)| len > l) {
            best = Some((len, i - c));
            if len == max {
                break;
            }
        }
        cand = prev[c % WINDOW];
    }
    best
}

/// `data` as a gzip stream.
pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut w = Bits {
        out: vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff],
        acc: 0,
        n: 0,
    };
    w.put(1, 1); // the only block is the final one
    w.put(1, 2); // fixed Huffman codes
    let (mut head, mut prev) = (vec![u32::MAX; 1 << HASH_BITS], vec![u32::MAX; WINDOW]);
    let insert = |i: usize, head: &mut [u32], prev: &mut [u32]| {
        if i + MIN_MATCH <= data.len() {
            let h = hash(&data[i..]);
            prev[i % WINDOW] = head[h];
            head[h] = i as u32;
        }
    };
    let mut i = 0;
    while i < data.len() {
        let found = (i + MIN_MATCH <= data.len())
            .then(|| longest_match(data, i, &head, &prev))
            .flatten();
        let step = match found {
            Some((len, dist)) => {
                let (l, d) = (slot(&LEN_BASE, len), slot(&DIST_BASE, dist));
                w.symbol(257 + l as u32);
                w.put(
                    (len - usize::from(LEN_BASE[l])) as u32,
                    u32::from(LEN_EXTRA[l]),
                );
                w.code(d as u32, 5);
                w.put(
                    (dist - usize::from(DIST_BASE[d])) as u32,
                    u32::from(DIST_EXTRA[d]),
                );
                len
            }
            None => {
                w.symbol(u32::from(data[i]));
                1
            }
        };
        (i..i + step).for_each(|p| insert(p, &mut head, &mut prev));
        i += step;
    }
    w.symbol(256);
    let mut out = w.finish();
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    /// Decompress with the system `gzip`, as an independent decoder.
    fn gunzip(z: &[u8]) -> Vec<u8> {
        let mut child = Command::new("gzip")
            .arg("-dc")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("gzip on PATH");
        let mut stdin = child.stdin.take().unwrap();
        let z = z.to_vec();
        let feeder = std::thread::spawn(move || stdin.write_all(&z));
        let out = child.wait_with_output().expect("gzip runs");
        feeder.join().unwrap().unwrap();
        assert!(out.status.success(), "gzip rejected the stream");
        out.stdout
    }

    fn noise(n: usize) -> Vec<u8> {
        let mut s = 0x1234_5678u32;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 24) as u8
            })
            .collect()
    }

    #[test]
    fn round_trips_through_the_system_gzip() {
        let text = "<html><body>{\"key\": \"value\", \"n\": 12345}</body></html>\n".repeat(400);
        let cases: [Vec<u8>; 6] = [
            vec![],
            b"a".to_vec(),
            b"abc".to_vec(),
            vec![7u8; 100_000], // one long overlapping run (distance 1, length 258)
            text.into_bytes(),
            noise(70_000), // incompressible, spans more than one window
        ];
        for data in cases {
            assert_eq!(gunzip(&compress(&data)), data, "{} bytes", data.len());
        }
    }

    #[test]
    fn repetitive_text_shrinks_a_lot_and_noise_only_a_little() {
        let text = "<tr><td class=\"num\">12.5</td><td>player one</td></tr>\n".repeat(2000);
        assert!(compress(text.as_bytes()).len() * 10 < text.len());
        let n = noise(50_000);
        assert!(
            compress(&n).len() < n.len() * 9 / 8 + 64,
            "fixed codes cost ≤ 12.5% on noise"
        );
    }

    #[test]
    fn matches_across_the_window_edge_still_decode() {
        let mut data = noise(WINDOW - 100);
        let head = data[..200].to_vec();
        data.extend_from_slice(&noise(400));
        data.extend_from_slice(&head); // a repeat just beyond the window: must not be referenced
        assert_eq!(gunzip(&compress(&data)), data);
    }
}

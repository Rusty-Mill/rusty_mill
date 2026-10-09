//! Differential tests against `ring` (a dev-dependency oracle, never shipped).

use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use rusty_aead::ChaCha20Poly1305;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next() as u8).collect()
    }
}

#[test]
fn seal_and_open_match_ring_across_lengths() {
    let mut rng = Rng(0x00c0_ffee_1234_5678);
    // Every length around the 16-byte Poly1305 and 64-byte ChaCha boundaries,
    // plus random ones up to several blocks.
    let lengths: Vec<usize> = (0..=200)
        .chain([255, 256, 257, 1000, 4096, 16384])
        .collect();
    for len in lengths {
        let key: [u8; 32] = rng.bytes(32).try_into().unwrap();
        let nonce: [u8; 12] = rng.bytes(12).try_into().unwrap();
        let aad_len = rng.below(70);
        let aad = rng.bytes(aad_len);
        let msg = rng.bytes(len);

        let theirs_key = LessSafeKey::new(UnboundKey::new(&aead::CHACHA20_POLY1305, &key).unwrap());
        let mut theirs = msg.clone();
        theirs_key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(&aad),
                &mut theirs,
            )
            .unwrap();

        let ours_key = ChaCha20Poly1305::new(&key);
        let mut ours = msg.clone();
        let tag = ours_key.seal_in_place(&nonce, &aad, &mut ours).unwrap();
        ours.extend_from_slice(&tag);
        assert_eq!(ours, theirs, "seal differs at len {len}");

        // ring opens what we sealed
        let mut round = ours.clone();
        let opened = theirs_key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(&aad),
                &mut round,
            )
            .unwrap();
        assert_eq!(opened, msg.as_slice());

        // and we open what ring sealed
        let (ct, tag) = theirs.split_at_mut(len);
        ours_key.open_in_place(&nonce, &aad, ct, tag).unwrap();
        assert_eq!(ct, msg.as_slice());
    }
}

#[test]
fn damaged_inputs_get_ring_verdict() {
    let mut rng = Rng(77);
    for _ in 0..300 {
        let key: [u8; 32] = rng.bytes(32).try_into().unwrap();
        let nonce: [u8; 12] = rng.bytes(12).try_into().unwrap();
        let (aad_len, msg_len) = (rng.below(40), rng.below(200));
        let (aad, msg) = (rng.bytes(aad_len), rng.bytes(msg_len));
        let theirs_key = LessSafeKey::new(UnboundKey::new(&aead::CHACHA20_POLY1305, &key).unwrap());
        let mut sealed = msg.clone();
        theirs_key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(&aad),
                &mut sealed,
            )
            .unwrap();

        let (mut aad2, mut sealed2, mut nonce2) = (aad.clone(), sealed.clone(), nonce);
        match rng.below(3) {
            0 if !aad2.is_empty() => {
                let i = rng.below(aad2.len());
                aad2[i] ^= 1 << rng.below(8);
            }
            1 => {
                let i = rng.below(sealed2.len());
                sealed2[i] ^= 1 << rng.below(8);
            }
            _ => nonce2[rng.below(12)] ^= 1,
        }
        let mut for_ring = sealed2.clone();
        let theirs = theirs_key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce2),
                Aad::from(&aad2),
                &mut for_ring,
            )
            .is_ok();
        let split = sealed2.len() - 16;
        let (ct, tag) = sealed2.split_at_mut(split);
        let ours = ChaCha20Poly1305::new(&key)
            .open_in_place(&nonce2, &aad2, ct, tag)
            .is_ok();
        assert_eq!(ours, theirs);
        assert!(!ours || (aad2 == aad && nonce2 == nonce));
    }
}

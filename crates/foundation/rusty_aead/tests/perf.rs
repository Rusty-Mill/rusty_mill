//! Throughput against `ring`. Ignored (machine-dependent); run with
//! `cargo test -p rusty_aead --release --test perf -- --ignored --nocapture`.

use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use rusty_aead::ChaCha20Poly1305;
use std::time::Instant;

fn best_mb_s(len: usize, mut f: impl FnMut()) -> f64 {
    (0..5)
        .map(|_| {
            let start = Instant::now();
            f();
            len as f64 / start.elapsed().as_secs_f64() / 1e6
        })
        .fold(0.0, f64::max)
}

#[test]
#[ignore = "timing; run manually"]
fn seal_throughput() {
    for len in [1 << 10, 16 << 10, 16 << 20] {
        let key = [7u8; 32];
        let mut buf = vec![1u8; len];
        let ours = ChaCha20Poly1305::new(&key);
        let theirs = LessSafeKey::new(UnboundKey::new(&aead::CHACHA20_POLY1305, &key).unwrap());
        let reps = ((64 << 20) / len).max(1);
        let a = best_mb_s(len * reps, || {
            for _ in 0..reps {
                std::hint::black_box(ours.seal_in_place(&[0; 12], b"", &mut buf).unwrap());
            }
        });
        let b = best_mb_s(len * reps, || {
            for _ in 0..reps {
                let tag = theirs
                    .seal_in_place_separate_tag(
                        Nonce::assume_unique_for_key([0; 12]),
                        Aad::empty(),
                        &mut buf,
                    )
                    .unwrap();
                std::hint::black_box(tag.as_ref());
            }
        });
        eprintln!(
            "{len:>9} B: ours {a:>6.0} MB/s   ring {b:>6.0} MB/s   {:.1}x slower",
            b / a
        );
    }
}

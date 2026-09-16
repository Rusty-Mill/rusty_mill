//! Where does semantic search actually get slow?
//! cargo run --release -p inventory-core --example vector_bench

use inventory_core::embed::linalg::l2_normalize;
use rand::{Rng, SeedableRng};
use std::time::Instant;

fn main() {
    let dim = 128;
    println!("{:>10} {:>14} {:>14}", "vectors", "exact scan", "per query");
    for n in [1_000usize, 5_000, 20_000, 50_000, 200_000, 1_000_000] {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(1);
        let mut data = vec![0.0f32; n * dim];
        for chunk in data.chunks_mut(dim) {
            for x in chunk.iter_mut() {
                *x = rng.gen_range(-1.0f32..1.0);
            }
            l2_normalize(chunk);
        }
        let q: Vec<f32> = data[..dim].to_vec();

        let trials = 20;
        let start = Instant::now();
        let mut sink = 0.0f32;
        for _ in 0..trials {
            let mut best = f32::MIN;
            for row in data.chunks(dim) {
                let s: f32 = row.iter().zip(&q).map(|(a, b)| a * b).sum();
                if s > best {
                    best = s;
                }
            }
            sink += best;
        }
        let per = start.elapsed() / trials;
        println!(
            "{n:>10} {:>13.1?} {:>13.3?} ms  (sink {sink:.1})",
            start.elapsed(),
            per.as_secs_f64() * 1000.0
        );
    }
}

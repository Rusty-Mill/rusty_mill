//! Planted-leak probe for `scripts/valgrind_selftest.sh`. Modes: `clean`,
//! `leaky-branch`, `leaky-index`. Under valgrind, `clean` must report nothing
//! and the other two must report at least one error.

use rusty_ct_check::taint::mark_secret;

#[inline(never)]
fn early_exit_eq(a: &[u8], b: &[u8]) -> bool {
    a.iter().zip(b).all(|(x, y)| x == y)
}

#[inline(never)]
fn masked_eq(a: &[u8], b: &[u8]) -> u8 {
    a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y))
}

#[inline(never)]
fn table_lookup(table: &[u8; 256], index: u8) -> u8 {
    table[index as usize]
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let secret = [0x5au8; 64];
    let guess = [0x5au8; 64];
    mark_secret(&secret);
    let mut table = [0u8; 256];
    for (i, t) in table.iter_mut().enumerate() {
        *t = (i * 7) as u8;
    }
    match mode.as_str() {
        "clean" => {
            std::hint::black_box(masked_eq(&secret, &guess));
        }
        "leaky-branch" => {
            if early_exit_eq(&secret, &guess) {
                std::hint::black_box(1);
            }
        }
        "leaky-index" => {
            // `black_box` hides the constant from the optimiser; without it the load
            // is folded away and memcheck has nothing to taint.
            let index = std::hint::black_box(&secret)[0];
            std::hint::black_box(table_lookup(&table, index));
        }
        _ => {
            eprintln!("modes: clean | leaky-branch | leaky-index");
            std::process::exit(2);
        }
    }
}

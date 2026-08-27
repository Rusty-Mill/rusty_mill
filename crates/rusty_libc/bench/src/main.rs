//! Micro-benchmark: the same syscalls issued through `rusty_libc` vs the `libc`
//! crate, reporting the best (lowest-noise) ns/op over several rounds. The
//! `libc` crate binds the system C library, so the comparison target is
//! whichever libc this build links — **glibc** on the `-gnu` target, **musl**
//! on the `-musl` target — and the output labels itself accordingly.
//!
//! Run with:
//! - `cargo run --release`                                 (vs glibc)
//! - `cargo run --release --target x86_64-unknown-linux-musl`  (vs musl)
//!
//! or `./run.sh` for both.
//!
//! The interesting rows are the `clock_gettime` ones: both libcs serve
//! `CLOCK_MONOTONIC` from the vDSO in userspace, and `rusty_libc` does too (its
//! vDSO fast path), so all avoid the syscall trap; `CLOCK_MONOTONIC_COARSE` is
//! faster still (on all three) since the vDSO skips its interpolation step.
//! Every other operation is a genuine syscall on both sides, so the numbers
//! should be at parity — the point being that replacing the `libc` crate costs
//! nothing at runtime.

use std::hint::black_box;
use std::os::fd::AsRawFd;
use std::time::Instant;

/// Time `iters` calls of `f`, returning ns per call (after a short warmup).
fn bench(iters: u64, mut f: impl FnMut()) -> f64 {
    for _ in 0..(iters / 20).max(1) {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    start.elapsed().as_nanos() as f64 / iters as f64
}

/// The lowest per-call time across `rounds` — the least noisy estimate.
fn best_of(rounds: u32, iters: u64, mut f: impl FnMut()) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..rounds {
        best = best.min(bench(iters, &mut f));
    }
    best
}

fn main() {
    let zero = std::fs::File::open("/dev/zero").expect("open /dev/zero");
    let zfd = zero.as_raw_fd();
    let null = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/null")
        .expect("open /dev/null");
    let nfd = null.as_raw_fd();

    let iters = 1_000_000u64;
    let rounds = 8;

    let r_getpid = best_of(rounds, iters, || {
        black_box(rusty_libc::process::getpid());
    });
    let l_getpid = best_of(rounds, iters, || {
        black_box(unsafe { libc::getpid() });
    });

    let r_getuid = best_of(rounds, iters, || {
        black_box(rusty_libc::process::getuid());
    });
    let l_getuid = best_of(rounds, iters, || {
        black_box(unsafe { libc::getuid() });
    });

    let mut buf = [0u8; 64];
    let r_read = best_of(rounds, iters, || {
        black_box(rusty_libc::fd::read(zfd, black_box(&mut buf)).unwrap());
    });
    let l_read = best_of(rounds, iters, || {
        black_box(unsafe { libc::read(zfd, buf.as_mut_ptr().cast(), buf.len()) });
    });

    let wbuf = [0u8; 64];
    let r_write = best_of(rounds, iters, || {
        black_box(rusty_libc::fd::write(nfd, black_box(&wbuf)).unwrap());
    });
    let l_write = best_of(rounds, iters, || {
        black_box(unsafe { libc::write(nfd, wbuf.as_ptr().cast(), wbuf.len()) });
    });

    // clock_gettime: both sides use the vDSO (userspace, no syscall trap).
    let r_clock = best_of(rounds, iters, || {
        black_box(rusty_libc::time::clock_gettime(rusty_libc::time::CLOCK_MONOTONIC).unwrap());
    });
    let l_clock = best_of(rounds, iters, || {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        unsafe {
            libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
        }
        black_box(ts.tv_nsec);
    });

    // The *_COARSE clocks skip the vDSO's fine-grained interpolation step, so
    // they are faster than the precise clock above at the cost of millisecond-
    // ish resolution — a good trade for a timestamp that doesn't need
    // sub-millisecond accuracy (see CLOCK_MONOTONIC_COARSE's docs).
    let r_clock_coarse = best_of(rounds, iters, || {
        black_box(
            rusty_libc::time::clock_gettime(rusty_libc::time::CLOCK_MONOTONIC_COARSE).unwrap(),
        );
    });
    let l_clock_coarse = best_of(rounds, iters, || {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        unsafe {
            libc::clock_gettime(libc::CLOCK_MONOTONIC_COARSE, &mut ts);
        }
        black_box(ts.tv_nsec);
    });

    // Name the libc we're actually linked against, so the same binary is
    // self-describing whether it was built for the gnu (glibc) or musl target.
    let libc = if cfg!(target_env = "musl") {
        "musl"
    } else if cfg!(target_env = "gnu") {
        "glibc"
    } else {
        "libc"
    };

    println!("rusty_libc vs {libc}  (best of {rounds} rounds x {iters} iters)\n");
    println!(
        "{:<22} {:>12} {:>12} {:>12}",
        "operation",
        "rusty (ns)",
        format!("{libc} (ns)"),
        format!("rusty/{libc}")
    );
    println!("{}", "-".repeat(60));
    let row = |name: &str, r: f64, l: f64| {
        println!("{name:<22} {r:>12.2} {l:>12.2} {:>11.2}x", r / l);
    };
    row("getpid", r_getpid, l_getpid);
    row("getuid", r_getuid, l_getuid);
    row("read(/dev/zero,64)", r_read, l_read);
    row("write(/dev/null,64)", r_write, l_write);
    row("clock_gettime(MONO)", r_clock, l_clock);
    row("clock_gettime(MONO_COARSE)", r_clock_coarse, l_clock_coarse);
    println!("\n(ratio < 1.00 = rusty_libc faster; > 1.00 = {libc} faster)");
}

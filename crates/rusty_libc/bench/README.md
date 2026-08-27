# rusty_libc vs libc — micro-benchmark

A small, reproducible harness that issues the **same syscalls** through
`rusty_libc` and through the [`libc`](https://crates.io/crates/libc) crate
(glibc), and reports ns/op for each.

It is a **standalone crate** (its own `[workspace]`), intentionally kept out of
the library's build: running `cargo build`/`test`/`clippy` at the repo root
never compiles it, so the library keeps its zero-dependency guarantee — `libc`
appears only here.

The `libc` crate binds the **system C library**, so the comparison target is
whichever libc the build links — **glibc** on the default `-gnu` target,
**musl** on the `-musl` target. The binary self-labels its output accordingly.

## Run

```sh
cd bench
cargo run --release                                    # vs glibc
cargo run --release --target x86_64-unknown-linux-musl # vs musl
./run.sh                                               # both
```

Requires network access to fetch the `libc` dev-dependency and a Linux host
(the crate is Linux-only). Reports the best of several timed rounds to reduce
noise. Absolute numbers vary by machine/kernel (CPU mitigations inflate syscall
cost); the **ratio** column is the meaningful part.

### musl

The musl target produces a **static** binary (Rust links a self-contained musl,
so no `musl-gcc` is needed). Add the target once:

```sh
rustup target add x86_64-unknown-linux-musl
```

## What it measures

| operation | notes |
|---|---|
| `getpid`, `getuid` | pure syscalls (modern glibc does not cache `getpid`) |
| `read(/dev/zero)`, `write(/dev/null)` | real I/O syscalls |
| `clock_gettime(MONOTONIC)` | served from the **vDSO** in userspace on both sides |
| `clock_gettime(MONOTONIC_COARSE)` | vDSO too, but skips fine-grained interpolation — faster, coarser resolution |

## Expected result

For genuine syscalls, `rusty_libc` and `libc` are at parity (±a few %): the
kernel trap dominates, and `rusty_libc`'s raw `asm!` stub only saves glibc's PLT
indirection and `errno` TLS write — a couple of nanoseconds, lost in the noise.
So swapping the `libc` crate for `rusty_libc` has no runtime cost.

`clock_gettime` is the one call the kernel accelerates via the vDSO (no syscall
trap). Both glibc and musl serve it from the vDSO, and `rusty_libc` resolves and
calls the vDSO entry too (see `../src/vdso.rs`), so it is ~an order of magnitude
faster than a raw `clock_gettime` syscall and lands close to both libcs — the
remaining gap is a fixed per-call cost (an atomic cache load plus a couple of
branches) that a libc's IFUNC-resolved call site skips entirely. `*_COARSE`
(see `CLOCK_MONOTONIC_COARSE`'s docs) trades resolution for speed and is faster
still on all three — reach for it whenever sub-millisecond accuracy isn't
needed (`$SECONDS`-style counters, throttling, a prompt timestamp). Sample run
(native x86_64; absolute numbers vary run to run — the CI-noted **ratio**
columns are the stable part):

```
rusty_libc vs glibc  (best of 8 rounds x 1000000 iters)

operation                rusty (ns)   glibc (ns)  rusty/glibc
------------------------------------------------------------
getpid                        81.61        86.45        0.94x
getuid                        81.31        82.30        0.99x
read(/dev/zero,64)           125.95       125.47        1.00x
write(/dev/null,64)          102.63       101.69        1.01x
clock_gettime(MONO)           34.19        28.36        1.21x
clock_gettime(MONO_COARSE)     8.45         5.04        1.68x

rusty_libc vs musl  (best of 8 rounds x 1000000 iters)

operation                rusty (ns)    musl (ns)   rusty/musl
------------------------------------------------------------
getpid                        85.11        86.29        0.99x
getuid                        81.17        83.59        0.97x
read(/dev/zero,64)           128.23       129.54        0.99x
write(/dev/null,64)          102.85       105.95        0.97x
clock_gettime(MONO)           34.38        29.01        1.19x
clock_gettime(MONO_COARSE)     8.44         4.24        1.99x
```

Note the coarse clock is ~4x faster than the precise one **within rusty_libc
itself** (8.4 ns vs 34 ns) — that ratio is the more actionable number if you
control which clock you call.

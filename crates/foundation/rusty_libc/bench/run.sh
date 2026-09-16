#!/bin/sh
# Run the harness against both libc implementations.
#
# - glibc: the default host target (-gnu).
# - musl:  the x86_64-unknown-linux-musl target (static binary; Rust links a
#          self-contained musl, so no musl-gcc is required). Add the target once
#          with: rustup target add x86_64-unknown-linux-musl
#
# The binary self-labels the comparison target ("glibc" vs "musl").
set -eu
cd "$(dirname "$0")"

echo "===== rusty_libc vs glibc (x86_64-unknown-linux-gnu) ====="
cargo run --release --quiet

echo
MUSL=x86_64-unknown-linux-musl
if rustup target list --installed 2>/dev/null | grep -qx "$MUSL"; then
    echo "===== rusty_libc vs musl ($MUSL) ====="
    cargo run --release --quiet --target "$MUSL"
else
    echo "musl target not installed — skipping."
    echo "Add it with:  rustup target add $MUSL"
fi

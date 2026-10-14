#!/usr/bin/env bash
# Run BoringSSL's protocol test suite (BoGo) against the hand-rolled engine.
#
#   BORINGSSL=/path/to/boringssl ./run.sh            # all tests
#   BORINGSSL=... ./run.sh -test 'Trailing*'         # extra runner flags
#
# BORINGSSL is a checkout of the commit in PINNED_COMMIT (see below); without it
# one is cloned into target/boringssl. Needs a Go toolchain and network on
# first use. Exits non-zero if any test FAILS; tests the shim does not
# implement (exit 89) and tests listed in config.json are reported, not hidden.
set -euo pipefail

# The BoringSSL commit the suite is pinned to. Bumping it will add and rename
# tests, so it is a deliberate change, reviewed like a dependency bump.
PINNED_COMMIT=0c8a01ade0e5349f5b0f9afd5e4de622599ae1da

# googlesource.com answers the occasional fetch with a transient 503 ("RPC
# failed"). That says nothing about the engine, so retry with a growing pause
# instead of failing the job; a persistent failure still fails it.
fetch_pinned() {
  local attempt
  for attempt in 1 2 3 4 5; do
    if git -C "$BORINGSSL" fetch -q --depth 1 origin "$PINNED_COMMIT"; then
      return 0
    fi
    echo "bogo: fetching $PINNED_COMMIT failed (attempt $attempt of 5)" >&2
    if [ "$attempt" -lt 5 ]; then
      sleep $((attempt * ${BOGO_FETCH_PAUSE:-5}))
    fi
  done
  return 1
}

here="$(cd "$(dirname "$0")" && pwd)"
out="${OUT:-$here/target}"
mkdir -p "$out"

if [ -z "${BORINGSSL:-}" ]; then
  BORINGSSL="$out/boringssl"
  if [ ! -d "$BORINGSSL/.git" ]; then
    git init -q "$BORINGSSL"
    git -C "$BORINGSSL" remote add origin https://boringssl.googlesource.com/boringssl
  fi
  if [ "$(git -C "$BORINGSSL" rev-parse HEAD 2>/dev/null || true)" != "$PINNED_COMMIT" ]; then
    fetch_pinned
    git -C "$BORINGSSL" checkout -q FETCH_HEAD
  fi
fi

(cd "$here" && RUSTFLAGS="--cfg rusty_tls_handrolled ${RUSTFLAGS:-}" cargo build -q --release)
(cd "$BORINGSSL/ssl/test/runner" && go test -c -o "$out/runner.test" .)

cd "$BORINGSSL/ssl/test/runner"
set +e
"$out/runner.test" -test.timeout 60m \
  -shim-path "$here/target/release/bogo_shim" \
  -shim-config "$here/config.json" \
  -allow-unimplemented -loose-errors -pipe \
  -json-output "$out/results.json" \
  -num-workers "${WORKERS:-8}" "$@" > "$out/run.log" 2>&1
status=$?
set -e

python3 "$here/summary.py" "$out/results.json" "$out/run.log" "$here/config.json"
exit $status

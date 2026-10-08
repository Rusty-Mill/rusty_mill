# BoGo shim

Runs BoringSSL's protocol test suite (BoGo) against the hand-rolled engine, as
an independent hostile peer: a corpus written by someone else, aimed at the
mistakes TLS stacks actually make.

```sh
./run.sh                    # all tests (needs Go; clones BoringSSL on first use)
./run.sh -test 'KeyUpdate*' # extra args go to the runner
```

The BoringSSL commit is pinned in `run.sh`; bumping it adds and renames tests,
so treat it like a dependency bump. Output lands in `target/` (`run.log`,
`results.json`) and `summary.py` prints counts and every failure.

## What the counts mean

At the pinned commit: 490 pass, 0 fail, the rest skipped.

| Skipped | Why |
|---|---|
| ~4700 | need a shim flag it does not implement (client auth, resumption, 0-RTT, ECH, ...) — the shim exits 89 |
| ~2000 / ~700 | DTLS / QUIC: out of scope |
| 139 | listed in `config.json` with a reason each |

`config.json` holds exact test names, never globs: a glob hides tests that
pass. Every entry is a finding or a documented difference, not a convenience.
Differences worth knowing: each `KeyUpdate` request is answered (BoringSSL
coalesces); a renegotiation request gets a `no_renegotiation` warning and the
connection carries on; 0-RTT is refused; RSA signing is PSS-only.

A failing test is an engine bug until shown otherwise. Do not add it to
`config.json` to get green.

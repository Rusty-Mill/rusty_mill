---
category: Added
changelog: rusty_conhost has coverage-guided fuzzing of hostile ConDrv packets, checked against the console host's memory and reply limits.
---
## 2026-10-31 - rusty_conhost: fuzz hostile console clients

- **Added:** `crates/libs/ui/rusty_conhost/fuzz`, a cargo-fuzz crate with two
  targets (design §8.1, PR 13). `msg_decode` feeds raw bytes to every
  packet decoder. `packet_sequence` reads its input as a script of
  CONNECT, DISCONNECT, object, raw read/write and console API packets,
  terminal input, resize, drain, hang-up and child exit, and drives the
  console the way the engine does, with `sched`'s admission and turn
  order. After every event it checks the §6.1 and §7.4 limits: the output
  queue stays within 144 KiB, at most 64 calls wait, parked payload stays
  within 1 MiB, each call is answered once and only if it was asked, a
  write-back fits the descriptor and the client's output, variable output
  fits the client's buffer, one call builds at most 1 MiB of replies, and
  the console logs at most 65 lines. Kill, Left and Ctrl go only to pids
  that joined and haven't left, and each pid is killed at most once. The
  script names the cookies the console handed out, can expire the grace
  deadline mid-traffic, detach the reader, and send headers that lie about
  their fixed size. It runs on the test code-page tables (437, 850 and
  part of 932), and half its SET_CP calls name one of those, so scripts
  reach the lead-byte and DBCS paths. Every script ends with a hang-up and the grace
  deadline, and the console must then close.
- **Added:** a `rusty-conhost-fuzz` CI job runs both targets for 30 s each
  on the daily sweep and on manual runs only, never on PRs or in the
  merge queue. It reuses `tls_fuzz_smoke.sh` and its dated nightly.
- **Added:** the driver is the crate's hidden `fuzz` module, behind a new
  `fuzz` feature that only the fuzz crate turns on. The crate's own tests
  run it on 300 seeded scripts, plus targeted cases (the 257th client,
  the 65th object, grace expiry with calls in flight, a lying header, a
  detached reader, a DBCS pair split across writes), so every PR exercises it on stable.

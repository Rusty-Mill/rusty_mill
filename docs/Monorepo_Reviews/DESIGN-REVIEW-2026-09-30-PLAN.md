# Remediation plan — crate-by-crate design review (2026-09-30)

Source: *Rusty Mill crate by crate design review* (baseline `eded76a`, addenda
at `988369a`). Plan drafted against `main` at `91a05b5`; the 25 files changed
since `988369a` are `rusty_tick` web only, so every finding below is still
live. Spot-checked at HEAD: Nexus importer `skipped += 1`, `nexus-templates`
`i + 4 <= len` / `i..i + 5` slice, croc zero-byte `File::create(&dest)`.

## Principles (from the review, adopted as-is)

- **Invariants before reorganization.** No crate merges, renames or size work
  until tranches 1–3 land.
- **One PR = one seam = fix + regression test.** The failing test is written
  first and shown red, then green. No governance-only PRs.
- **Truthful APIs.** A call that can't do the work returns `Unsupported` or
  isn't exported. It never returns success-shaped output.
- **Consolidate only where the contracts really match.** Shared code has to
  remove a repeated correctness obligation, not just shrink the line count.
- **Measure before optimizing.** Tranche 5 is measurement. Nothing gets
  optimized without data.

## Tranche 1 — Soundness and safe-API contracts (P1)

| # | Crate / seam | Fix | Regression |
|---|---|---|---|
| 1.1 | `rusty_std` `MutexGuard` | Add `T: Sync` bound on guard `Sync` (mirror `rusty_sync`). Do **not** add a `rusty_std → rusty_sync` edge (cycle). | `trybuild` compile-fail: `MutexGuard<Cell<_>>: Sync` |
| 1.2 | `platform-windows` raw handles | `from_raw` → `unsafe fn` with `# Safety` contract; audit callers | Caller audit; doc-test |
| 1.3 | `rusty_libc` process-memory write | Make self-target write `unsafe` or reject `pid == self` | Unit test on the restriction |
| 1.4 | `rusty_sync` channel | Recheck queue under coherent sync before reporting `Disconnected` | Loom/stress: last-send-then-drop never loses data |
| 1.5 | `rusty_std` Windows entropy | Chunk at `ULONG::MAX`; never report a partial fill as success | Large-buffer test |
| 1.6 | `LinuxChild::kill_single` | Refuse to signal after reap; prefer pidfd | Reaped-child test |

Follow-up (after 1.1): pick one authoritative spinlock/guard in `rusty_std`,
re-export it from `rusty_sync`, and delete the duplicate copy.

## Tranche 2 — Persisted invariants and identity (P1)

| # | Crate / seam | Fix | Regression |
|---|---|---|---|
| 2.1 | `rusty_multimodal_db` strict txn (addendum P1) | One validation path shared by live commit and replay; validate **before** journal append | Same-files restart: rejected insert+link never reappears |
| 2.2 | `rusty_multimodal_db` strict replay | Don't skip an incomplete batch after its durable prefix | Truncated-batch reopen |
| 2.3 | `rusty_multimodal_db` replication R1–R3 | Durable replay before cursor persist; Link→Delete replay across cursor window; append failure poisons the epoch | Fault-injection at each write boundary |
| 2.4 | `rusty_rusqlite` | Persist only at commit; snapshot tables **and** indexes; atomic temp+fsync+rename | Review's rollback/reopen test + index rollback test |
| 2.5 | `rusty-search-sqlite-fts5` delete | Single transaction across `idx_fts` + content | Failure-injection between the two deletes |
| 2.6 | Nexus N1 importer | Opaque cross-app `MemoryId` (or reversible mapping) that accepts `mem_…`; per-row classified failures | Import real `remind_me`-created rows; idempotent re-import |
| 2.7 | Nexus N2 sync cursor | Advance the cursor only together with applied rows or a durable dead-letter record; split applied/skipped/rejected counts | good/bad/good page; crash between apply and cursor write |
| 2.8 | Nexus N3 hub LWW | Parse and canonicalize `updated_at` (UTC, fixed precision); reject malformed | `"zzzz"`, same instant at different offsets, precision |
| 2.9 | Nexus N6 comments, `rusty_inventory` tray/CLI, `rusty_tick` moves | Atomic write helper (temp+fsync+rename); cross-process writer lock for inventory; transactional parent/child move in tick | Truncation recovery; concurrent writer; interrupted move |

2.9 uses one small shared `atomic_write` in the platform/fs layer. That makes
three real call sites, so the shared helper is justified.
*Landed in Tranche 4 as `crates/foundation/rusty_atomic_file`, a std
crate, not rustils: rustils has no filesystem backend on macOS, where Nexus
ships, and a foundation crate is usable by every layer. 16 call sites moved;
the multimodal engine keeps its own `durability` helper. The 3.2 rooted open
landed alongside it as `crates/foundation/rusty_confined_fs` (an
`openat`/`O_NOFOLLOW` walk on Linux), used by croc and the Fedora agent.*

## Tranche 3 — Fail-closed and bounded I/O (P1/P2)

| # | Seam | Fix |
|---|---|---|
| 3.1 | `rp-server` auth | Unresolved configured key/JWT → startup error; `--insecure-dev` explicit; default bind `127.0.0.1` |
| 3.2 | Receive-path confinement (croc zero-byte + ZIP, Fedora config allowlist) | One root-relative no-follow create/open (`openat`/`O_NOFOLLOW`, Windows equivalent); every create goes through it |
| 3.3 | `rusty_request` N09 | Strip/re-scope caller `Cookie` (and other sensitive headers) on origin change; one redirect-policy function shared by buffered and streaming paths |
| 3.4 | `rusty_http` N06 | Reject repeated/non-final TE, TE+CL; table-driven malformed-framing fixture reused by every adapter |
| 3.5 | `rusty_h2` N05 | Continuation cap, window-underflow reject, no interleave; one flow-control authority (drop duplicate) |
| 3.6 | RDP N08 / A2A N10 | Accept `rusty_tls::TrustPolicy`; rename insecure helpers `*_insecure`; safe webhook address classes + per-hop redirect check |
| 3.7 | Admission and deadlines: N01 llama, N02 whisper, N03 log server, N07 kafka, N11 LSP, N16 adk-mcp, multimodal staged-byte budget | Bounded admission + whole-operation deadline; poison connection on partial I/O |
| 3.8 | Panics and budgets on input: Nexus N7 templates, rush brace ranges, Myers diff | Char-safe scanning; checked stepping + item/byte budget; byte-budget cap on diff |

## Tranche 4 — Lifecycle, honesty, then proven consolidation

**Lifecycle:** Nexus N4 (cancel the child token on timeout via a drop guard),
N5 (runtime handle owned by bootstrap instance state, not a `OnceLock`), async
waker replacement (Timeout/PidfdReady/WaitJob), Yirp PID-identity teardown,
Tailscale peer removal of all derived state, multimodal drain D4/D5, rusty_gui
X11 `Drop`.

**Honest capabilities:** RustyJson derive (implement or `compile_error!`),
rusty_gui clipboard (`Unsupported`), `rusty_std` stub I/O/process
(`Unsupported`), `rusty_wiremock::MockServer` (feature-gate or remove),
`Session::commit` doc/contract, coreutils `rtail`/`rwc`/`rxargs`.

**Consolidation (only after conformance tests exist):**
- `adk-mcp` onto the shared MCP stack *(done: conformance suite first, then
  `rmcp`; see `crates/libs/rusty_adk/crates/adk-mcp/tests/conformance.rs`)*
- one stream reducer shared by adk-agent and adk-models
- HTTP parsing for llama and whisper onto `rusty_http`
- shared `atomic_write` and rooted-fs helpers (from 2.9 / 3.2)
- rush / nexus-rush shared fixtures only

## Tranche 5 — Measurement

A per-product baseline on Linux + Windows: binary size, dependency closure,
clean/incremental build, startup, idle/peak RSS. It starts with a
multimodal O(N) equality-count regression (D3) and prefix-query fan-out in
rusty_tick. After that comes the storage comparison with matched durability
settings. The results decide any later simplification.

## Execution

- Branch per row (`fix/<crate>-<slug>`), merge commit, CI green. Record
  lessons in `CHANGELOG.md`/`RELEASE_NOTES.md` per repo convention.
- Tranche 1 rows are independent and can go in parallel. Tranche 2 starts
  with 2.1–2.3 (the highest risk, and recently merged code).
- Also fix the flaky Windows `rusty_tls` async handshake test (600s timeout,
  then 0.019s pass). A flake is still a bug.

## Decisions needed from owner

1. **Nexus memory identity (2.6):** a shared opaque `MemoryId` type across
   nexus and remind_me (recommended), or a Nexus-local reversible mapping?
2. **`rusty_rusqlite` (2.4):** fix full transactional durability
   (recommended), or declare file-backed transactions `Unsupported` until
   later?
3. **coreutils daily-driver utilities:** fund conformance, or retire the
   overlapping incomplete binaries and keep only the reference consumers
   (`cat`/`ls`/`rrun`/`rpar`)?
   *Decided 2026-09-30: retire. `rtail`/`rwc`/`rxargs` removed in Tranche 4;
   the other overlapping binaries await the same call.*
4. **Public API breaks** in 1.2, 3.6 renames and the `MockServer` removal: OK
   to break now (pre-1.0), or deprecate first?
   *Decided 2026-09-30 for `MockServer`: removed now (no callers).*
5. **Tracking:** one GitHub issue per table row (≈40), or one issue per
   tranche with checklists (recommended — less noise)?

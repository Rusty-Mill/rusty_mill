# rusty_term: remaining work and roadmap (handoff)

Date: 2026-10-23. Repo `Rusty-Mill/rusty_mill`. Scope: `crates/libs/ui/rusty_term` (front-ends) and `crates/libs/ui/rusty_term/core` (`rusty_term_core`, the headless VT engine the mux will link).

This picks up from `docs/research/RUSTY-TERM-PERF-BACKLOG.md` (row numbers below refer to it) and from the owner's directives. The decision log and design context are in [RUSTY-TERM-NOTES.md](RUSTY-TERM-NOTES.md). Read both before starting.

## 1. State at handoff

| Top-5 item | Backlog rows | Status | PR |
|---|---|---|---|
| 1. Fidelity gate | 14 | Done (18 recordings, 80x24) | #857 |
| 2. Ring-buffer screen + recycled rows | 1, 2 | Done, but the speed floors are **not** met (§3.1) | #930 |
| 4. Printable fast path | 3, 4 (ASCII only) | Done for ASCII. Non-ASCII runs and batched `put_str` for other text are not done. | #936 |
| 5. Snapshot serializer | 16 (full only) | Full snapshot done. Incremental is not done. | #939 |
| 3. GUI frame path | 8, 9, 7 | Frame benchmark built. **The work itself has not started.** | #940 (bench) |

Also merged:

| PR | What | Closes |
|---|---|---|
| #851 | Core split | |
| #862 | BCE | #854 |
| #872 | Instruction-count CI gate | |
| #894 | wcwidth widths and mode 2027 | #855 |
| #925 | DECCOLM | #856 |
| #943 | ctutils 0.4.4 (RUSTSEC-2026-0385) | |

#944 is closed as a duplicate, and its branch has been deleted.

**#940 merged at `5c24bf8c`.** These docs landed in #950. No PR from this work is open.

## 2. Roadmap (recommended order)

Each step needs the owner's sign-off before it starts, because the owner gates every top-5 item. Each one ships as its own PR scoped to the rusty_term crates.

1. **GUI frame path, row 9 first: compute the view-row map once per frame.**
   - Today `abs_of_view_row` → `display_history_len` walks up to 1,024 `fold_blocks` for every cell access, about 4 accesses per cell in the CPU path. Code: `core/src/grid.rs` (`abs_of_view_row`, `display_history_len`) and `src/gui/cpu.rs`.
   - Proof: in `bench_frame_time`, 1,024-block frames match 0-block frames. The baseline is 8.79 ms live and 13.47 ms scrolled back, against 1.02 ms with no blocks.
   - Low risk; the smallest change of the three.
2. **Row 8: short lock, then render without it.**
   - Copy what the frame needs under the lock, release it, then composite and present.
   - The GUI holds every visible pane's grid lock across render **and** present/vsync (`src/gui/window.rs`, `gpu.rs` `get_current_texture`).
   - Proof: lock hold under 1 ms per frame (baseline p50 1.43-2.54 ms, p99 up to 7.28 ms), and parser MB/s while rendering close to MB/s alone.
   - Consider fair handoff so parsing cannot starve frames (Alacritty's `FairMutex`, Ghostty's lock demand), described in the backlog.
3. **Row 7: the GUI consumes dirty rows.**
   - The core already tracks `Grid::dirty`, and the TUI uses it. The CPU path does a full `buffer.fill`, and the GPU path rebuilds all instances.
   - Design the per-row change seqno here, because the same mechanism serves incremental snapshots (step 5). Agree its shape with the mux protocol owner first.
4. **Close the throughput floors (§3.1).** Measure first; there are possibly two separate problems:
   - a benchmark artifact (a fresh grid per iteration, page-faulting new scrollback);
   - the real per-line heap allocation in scrollback (row 6).
5. **Incremental snapshot.**
   - A per-row seqno plus stable row numbers.
   - Add parser state and modes, or define how a client receives them on attach.
   - A version bump of `RTSN` if the format changes.
   - Lands with the `rusty_mux_proto` work that consumes it, not before.
6. **Compact cell + style interning (row 5) and contiguous scrollback (row 6).**
   - The largest memory win: 24.4 vs 8.3 B/cell.
   - Deferred by the owner until the row and wire formats settle. Bump the snapshot version.
   - High risk: touches every Cell consumer (gui, render, snapshot).
7. **P2 and P3 rows, in any order once the above settle:**
   - 10: PTY reads of 64 KiB to 1 MiB with coalesced wakeups.
   - 11: no per-frame GPU buffer creation.
   - 13: sync-output expiry on a timer.
   - 15: reflow without a full rebuild. Measured 86 ms against 10.8 ms.
   - 12: glyph and shape caches without per-frame allocation.
   - 17: compression of idle scrollback.
   - 18: TUI output, one buffer and minimal SGR.
   - 19: SIMD scan of OSC/APC/DCS payloads.

## 3. Open problems and unknowns

1. **The #2 floors are not met in wall-clock.**
   - The owner's floors are scroll_heavy ≥ 16 MB/s and ascii_throughput ≥ 58 MB/s. Local results: scroll_heavy about 10.5 MB/s, ascii_throughput about 36 MB/s.
   - The memmove share on scroll_heavy is under 5%; that floor (under 10%) is met.
   - Instructions per byte fell a lot: ascii 376 → 48, scroll 1001 → 137.
   - About half the remaining wall-clock time is the kernel faulting in fresh scrollback, because `bench_vt_throughput` builds a new grid per iteration.
   - With glibc keeping freed memory, ascii runs at over 100 MB/s and scroll at about 40 MB/s.
   - `bench_frame_time`, which keeps one grid alive, reports 49-160 MB/s parser-only on the scrolling, ASCII, cursor, SGR and Unicode workloads (alt_screen_flicker: 7.5 MB/s).
   - Whether the floors should be judged on a long-lived grid is **for the owner to decide**.
2. **Snapshot encode is not reliably faster than libghostty:** 11.5-14.3 ms against 9.3-11.0 ms on the later host, 9.3 against 9.6 ms on the first. It is 3.7x smaller (2.09 MB against 7.81 MB). Whether encode time is a hard target is unconfirmed.
3. **Unicode is the slowest workload:** about 24 MB/s against libghostty's 44.5. The fast path covers ASCII only.
4. **Fidelity coverage gaps:**
   - 80x24 final screens only.
   - Not covered: resize/reflow, scrollback, OSC 8, images, titles, mouse modes, DECDWL/DECDHL.
   - Linux recordings only.
5. **Windows:**
   - Nothing was measured on Windows.
   - ConPTY spike 1 (`spike1-conpty.ps1`) waits on the owner's Windows machine.
   - A Windows ConPTY fidelity corpus is still owed.
6. **#850 is fixed** (#949): the core's `l13` feature no longer pulls in tokio. A new in-workspace consumer of `rusty_lsp`'s server or client must enable its `runtime` feature.
7. **Mode 2027:** grapheme clustering is opt-in only, with no config key. Whether a key is wanted is unconfirmed.
8. **Clean-room:** the backlog's technique rows were informed by reading Ghostty, Alacritty and WezTerm source, and kitty (described only). If the mux policy is strict clean-room, ask before reusing those designs (seqno and stable row index come from WezTerm).

## 4. Hard rules (owner and repo)

- **Every change goes through a PR on its own branch. Never push to `main`.**
  - Merge through the merge queue ("Merge when ready"), with a **merge commit**.
  - Never force-push or rebase shared history. To update a branch, merge `main` into it.
- **Queueing.** A PR is ready once the reviewer has cleared its exact head and CI is green on it. Record the review and a self-review in the PR body, then queue it ("Merge when ready"). The merge queue's own validation (`required-gate` on the queued entry) still applies.
  - Basis: the owner's latest instruction, as reported by the reviewer on #950: "Whenever a pr is ready and not queued, you should queue it." If in doubt, confirm with the owner.
  - *Historical:* in the 2026-10-21 to 2026-10-23 session the owner queued every PR, and the agent only reported readiness. That is not a rule for successors.
  - A queued branch cannot be pushed (GH006). Dequeue it, or have it dequeued, first.
- **Code:**
  - No `unwrap`/`expect` in production code.
  - Tests for non-trivial logic, including boundary cases: scroll regions, resize, wide glyphs at row edges, scrollback eviction.
  - Small, modular code.
- **No new third-party dependency, and no dependency or toolchain bump, without the owner's approval.** That includes `Cargo.lock` changes. Fix advisories, never suppress them.
- **libghostty is a comparison oracle only, never on a runtime path.** Read non-permissive sources, but copy nothing.
- **Deliberate divergences:**
  - DECCOLM never resizes the grid or PTY. The three vttest cases that differ from libghostty are intended; see `FIDELITY.md`.
  - Do not change default widths away from per code point.
- **Every PR needs a change fragment:** `changes/<date>-<slug>.md`, checked by `python3 .github/scripts/changes.py check`. Use the PR templates in `.github/PULL_REQUEST_TEMPLATE/`.
- **Never read `.env`; never commit or log secrets.**
- **rusty_mux crates use `rusty_tokio`, not tokio.**
  - The mux links `rusty_term_core`. Its `l13` feature is tokio-free since #949.
  - Do not link the `rusty_term` front-end crate, which depends on tokio and `parking_lot`.

## 5. How to validate

Run these from `crates/libs/ui/rusty_term` unless noted.

```sh
# tests (core + front-end)
cargo test -p rusty_term_core -p rusty_term --all-features
cargo test -p rusty_term_core --test fidelity          # gate; UPDATE_GOLDEN=1 only with a justified golden change
cargo clippy -p rusty_term_core -p rusty_term --all-features --all-targets -- -D warnings
cargo fmt --all --check

# instruction-count gate (what CI enforces; needs valgrind)
python3 bench/gen_workloads.py --size quick
cargo build --release --bin bench_vt_throughput
python3 bench/ir_gate.py --bin ../../../../target/release/bench_vt_throughput

# wall-clock (informational)
cargo run --release --bin bench_vt_throughput -- 10
cargo run --release --features gui --bin bench_frame_time            # needs a monospace font or $RUSTY_TERM_FONT

# libghostty comparison (local only; standalone crates outside the workspace)
sh ../../../../docs/research/rusty-term-perf/oracle_diff.sh --diff
```

**Ceilings** (`bench/ir_ceilings.txt`, instructions per byte):

| Workload | Ceiling |
|---|---:|
| alt_screen_flicker | 809 |
| ascii_throughput | 53 |
| cursor_thrash | 93 |
| scroll_heavy | 150 |
| sgr_churn | 128 |
| unicode_heavy | 184 |

The ratchet rule: a PR that lowers a count lowers that ceiling to about measured + 10% in the same PR.

**Other repo checks:**
- `cargo deny --workspace check advisories` reproduces the cargo-deny CI check locally, if job logs won't download.

## 6. Working notes for the next agent

- **Reviewer:** reviews arrive as PR comments from @baileyrd, pinned to an exact head SHA, with numbered P-level findings and "No admission until …" conditions.
  - Answer each finding on the PR, fix in a new commit, and cite the fixing SHA.
  - The reviewer re-reviews the delta.
- **CI behaviour:**
  - `required-gate` aggregates all jobs. Failures on superseded heads (cancelled runs) can be ignored.
  - Clippy jobs have hit the 15-minute apt-setup timeout before. Re-run once only if the job died before any test ran.
- **Grid code map:**
  - The screen is `Vec<Line>`; access rows through `Grid::row` and `Grid::row_mut`. `Grid::with_screen` is `pub(crate)`.
  - Scrollback evictions are recycled. Alt-screen rows are kept after first use.
  - The ASCII path is the parser's `printable_ascii_len` (SWAR) feeding `Grid::put_ascii` / `fill_ascii`. An equivalence test compares it with the per-character path.
- **Snapshot decoder** (`core/src/grid/snapshot.rs`): input is untrusted.
  - Keep the limits: MAX_DIM 4096, MAX_CELLS 2^25, MAX_LINES 2^20; a varint's tenth byte must be ≤ 1.
  - Keep validate-before-allocate. `tests/snapshot_alloc.rs` asserts peak allocation under 256 KiB on hostile input.
- **Frame bench internals** (`src/bin/bench_frame_time.rs`):
  - The `Painter` trait allows stand-in painters in tests.
  - Percentiles are nearest rank.
  - Fold fixtures are matched text with and without OSC 133 marks.
  - Wall-clock only; never make it a CI gate.
- **Report numbers honestly:** local wall-clock varies by host. Quote ranges and the host date, and say when a target is not met.

## 7. References

- **Docs:**
  - `docs/research/RUSTY-TERM-PERF-BACKLOG.md`
  - `docs/research/MUX-ADOPT-DECISIONS.md`
  - `docs/research/rusty-term-perf/FIDELITY.md`
  - `docs/projects/mux/{CHARTER,DECISIONS,RISKS,SPIKES}.md`
  - `crates/libs/ui/rusty_term/bench/README.md`
  - `docs/projects/mux/RUSTY-TERM-NOTES.md`
- **Pull requests:** #851, #857, #862, #872, #894, #925, #930, #936, #939, #940, #943, #944 (closed).
- **Issues:** #850 (closed by #949), #854, #855, #856 (closed).
- **Branch used:** `claude/new-session-s68lms`. Start new work from `origin/main` on a fresh branch.

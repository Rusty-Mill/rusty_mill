# rusty_term notes: handoff to Rusty Mux

Source: the rusty_term performance session of 2026-10-21 to 2026-10-23 (branch `claude/new-session-s68lms`). Facts come from the merged change fragments, `docs/research/RUSTY-TERM-PERF-BACKLOG.md`, `docs/research/rusty-term-perf/FIDELITY.md` and `docs/projects/mux/DECISIONS.md`. **No `rusty_mux_*` crate exists yet.**

⚠ marks a point that touches the handoff constraints: rusty_tokio runtime, clean-room, no Rex/tmux compatibility, no new third-party dependencies without approval.

## 1. Decisions

Owner confirmed = the owner stated it or accepted it in this session, or it is recorded as an owner decision in `docs/projects/mux/DECISIONS.md`.

| # | Decision | Reason | Owner confirmed |
|---|---|---|---|
| D1 | `rusty_term_core` is the single VT engine for the mux. libghostty-vt is a comparison oracle only, never on a runtime path. | Reuse rule; no Zig or C toolchain; sovereignty | Yes (DECISIONS 2026-10-21) |
| D2 | The headless split is a pure crate move: `src/core/` became `rusty_term_core` (#851). | The mux and server link the core without the GUI | Yes (backlog accepted 2026-10-21) |
| D3 | Accept the perf backlog: 19 rows, top 5 in order. | Measured: the core was 1.6-4.5x slower than libghostty-vt | Yes (2026-10-21) |
| D4 | The CI perf gate counts instructions (cachegrind, glibc memmove pinned). MB/s is informational and never fails a build. Ceilings ratchet down in the PR that lowers a count. | Deterministic on shared runners | Yes |
| D5 | DECCOLM never resizes the window, grid or PTY. Mode ?40 is honoured and off by default. With ?40 set, DECCOLM clears the screen, resets the margins and homes the cursor at the current size. With ?40 off, it is ignored. (#856, PR #925) | In a mux the server owns the PTY size and each client its own viewport | Yes. Ghostty resizes on DECCOLM; that difference was reported, PR #925 was merged, and the case is recorded as a deliberate divergence. |
| D6 | The three vttest DECCOLM cases (`vttest_1_2`, `1_3`, `2_3`) stay different from libghostty on purpose. Do not "fix" them. | Follows from D5 | Yes |
| D7 | Widths are per code point by default (wcwidth-like); regional indicators take 2 columns. Grapheme clustering is opt-in with DEC mode 2027. (#855, PR #894) | vim and other per-code-point apps misplaced the cursor | Yes (owner asked for the fix and merged it) |
| D8 | BCE: erases fill with the current SGR background. (#854, PR #862) | The terminfo advertises `bce` (inherited from xterm-256color) | Yes |
| D9 | Row layout: the screen is `Vec<Line>`, the same type scrollback holds. Scrolling rotates rows, and evicted scrollback lines are recycled. (PR #930) | memmove was 92-97% of instructions | Yes |
| D10 | The row representation must stay serializable. The compact cell (backlog row 5) is deferred until the row and wire formats settle. | Change the cell format only once | Yes |
| D11 | Work order after the ring buffer: #4 printable fast path, then #5 snapshot serializer, then #3 GUI frame path. | Only #4 could close the speed floors | Yes ("Concur") |
| D12 | Snapshot format `RTSN` v1 on `rusty_wire`. Full snapshots only; incremental updates come with the mux protocol. (PR #939) | Mux attach prerequisite | Yes (approved "go ahead with #5") |
| D13 | The fast path is a std-only SWAR scan; no `memchr`. | Avoid a new third-party dependency | Yes (owner's constraint) |
| D14 | #850 (the `l13` feature pulls tokio in through `rusty_lsp`) is low priority. Leave it alone. | The mux does not need `l13` in phase 1 | Yes |
| D15 | Fix RUSTSEC-2026-0385 with a separate lockfile PR, never a suppression. #943 (`ctutils` 0.4.4) landed. My duplicate #944 (0.4.5) was closed. | cargo-deny was blocking every PR | Yes ("a", "Close 944") |
| D16 | `rusty_tokio` is the async runtime for every `rusty_mux_*` crate; no tokio in any mux binary. | Sovereignty | Yes (DECISIONS 2026-10-22, recorded before this handoff) |
| D17 | The frame-time benchmark is a local tool, not a CI gate. | Wall-clock is unstable on shared runners | Unconfirmed (not objected to; #940 merged as such) |

## 2. Design context

**Architecture**
- Crates:
  - `crates/libs/ui/rusty_term/core` (`rusty_term_core`): the headless engine.
  - `crates/libs/ui/rusty_term` (`rusty_term`): the front-ends (TUI, winit GUI, PTY backends). It re-exports the core as `rusty_term::core`.
- Core dependencies:
  - Without features: `rusty_base64`, `rusty_hex`, `rusty_wire`, `unicode-width`, `unicode-segmentation`.
  - Feature `gui` adds `rusty_regx`. Feature `l13` adds `rusty_term_l13`, which pulls in tokio.
- ⚠ The `rusty_term` front-end crate depends directly on `tokio = "1"` (rt-multi-thread, net, signal, time, sync) and `parking_lot`. A mux crate must link `rusty_term_core` without `l13`, not `rusty_term`, or it brings in tokio and conflicts with D16.

**Grid API**
- Rows: `Grid::row(y)` and `Grid::row_mut(y)`. `Grid::cells` is no longer public.
- `Line` holds the cells, a soft-wrap bit and the line size (DECDWL/DECDHL).
- Scrollback is `SCROLLBACK_MAX = 10_000` lines, still one heap allocation per line.
- The alternate screen keeps its rows after first use, which costs one extra screen of memory.
- The cell is 24 bytes (24.4 B/cell measured, against 8.3 for libghostty).

**Snapshot (`core/src/grid/snapshot.rs`): what a mux attach can use today**
- API: `Grid::encode_snapshot` and `Grid::decode_snapshot`.
- **Carries:** size, cursor, scroll region, autowrap, title, cluster and hyperlink tables, scrollback, live screen, and the stashed primary screen while the alt screen is active.
- **Does not carry:** parser state, input modes, palette, images. A client attaching mid-escape-sequence or with non-default modes needs these from elsewhere.
- **Encoding:** style runs; each style is defined inline on first use and then referenced by id; repeat runs; varints.
- **Decoder hardening:** input is untrusted. Limits are MAX_DIM 4096, MAX_CELLS 2^25 and MAX_LINES 2^20; overflowing varints are rejected; the grid is built only after full validation; a counting-allocator test checks peak allocation.
- **Size and speed:** 200x50 plus 10k lines encodes to 2.09 MB (1.04 B/cell), against libghostty's 7.81 MB.
  - Encode is 9.3-14.3 ms, which does not reliably beat libghostty's 9.3-11.0 ms.
  - Decode is 26-34 ms.
- **Incremental sync is not built.** The planned design is a per-row change seqno plus a stable row index, an idea taken from WezTerm (see the clean-room flag below). The per-row seqno would also serve the GUI dirty-row path (backlog row 7).

**VT behaviour relevant to a mux**
- DECCOLM: see D5. The PTY size belongs to the server, and the engine never changes size by itself.
- Widths: see D7. Mode 2027 is per terminal and reported by DECRQM; RIS resets it, DECSTR keeps it. There is no config key for it. Server and client must agree on width rules.
- BCE: see D8. RIS and DECSTR reset the erase colour.
- Synchronized output (mode 2026) is honoured with an 800 ms timeout. Expiry is only noticed on the next event.
- The XTVERSION reply reads the core crate's version (0.1.1, kept in lockstep with `rusty_term`).

**Rendering (measured by `bench_frame_time`, PR #940)**
- **Lock hold:** the GUI holds the grid lock for the whole frame, including present/vsync. Hold time is 1.43-2.54 ms at p50, up to 7.28 ms at p99, with a 12.32 ms max. The parser is blocked for all of it. The target is under 1 ms (backlog row 8).
- **Fold blocks:** 1,024 command fold blocks make a frame 8.6x slower at the live view and 13x slower scrolled back. The cause is the O(cells × fold_blocks) view mapping (backlog row 9).
- **Dirty rows:** the core tracks dirty rows (`Grid::dirty`), and the TUI uses them. The GUI ignores them (backlog row 7).

**Performance status** (local, instructions per byte from the CI gate; ceilings: alt 809, ascii 53, cursor 93, scroll 150, sgr 128, unicode 184)
- scroll_heavy: about 3.3 → 10.5 MB/s after the ring buffer. The floor is 16.
- ascii_throughput: about 11 → 24 MB/s with the ring buffer, then 28 → 36 MB/s with the fast path (a separate local run). The floor is 58.
- unicode_heavy: about 13 → 24 MB/s. The libghostty target is 44.
- memmove share on scroll_heavy is under 5%; that floor (under 10%) is met.
- About half the run time is page-faulting fresh scrollback, because the benchmark builds a new grid per iteration. With glibc keeping freed memory, ascii runs at over 100 MB/s and scroll at about 40.

**PTY / ConPTY**
- PTY reads use a 4 KiB buffer, with one `advance` and one redraw event per read (backlog row 10, unmeasured).
- Windows is not measured. The fidelity corpus is Linux only.
- Spike 1 (ConPTY, `spike1-conpty.ps1`) runs on the owner's Windows machine; results are pending.

**Fidelity gate**
- `cargo test -p rusty_term_core --test fidelity` replays 18 recordings at 80x24. It also requires byte-at-a-time replay to match 4 KiB chunks.
- 15 of 18 match libghostty; the 3 remaining are D6.
- Not covered: resize/reflow, scrollback, OSC 8, images, titles, mouse modes, DECDWL/DECDHL.

**⚠ Clean-room flags**
- The perf research read source code: Ghostty, Alacritty and WezTerm (MIT/Apache) and kitty (GPL-3.0, described in my own words only). Nothing was copied, but the backlog's "who does it" column and the planned seqno/stable-row design were informed by reading that code. If the mux requires strict clean-room, treat those rows as tainted design input and re-derive from public docs.
- The oracle harness `docs/research/rusty-term-perf/ghbench` builds a third-party fork (phall1 `libghostty-rs@fd9d9001`) and fetches a prebuilt `.a` at build time. It is a standalone crate outside the workspace and never on a runtime path.
- The fidelity corpus includes `tmux_split`, a recording of tmux output used only as a VT test input. It is not tmux compatibility.
- The phrase "Rex parity" does not appear anywhere in this session. Its relation to "no Rex/tmux compatibility" is unconfirmed.

**⚠ Dependencies**
- No new third-party dependency was added in this session.
- `unicode-width` and `unicode-segmentation` were already in the core.
- The only lockfile change was the transitive `ctutils` security bump (D15), which the owner approved.

## 3. Open questions

1. The speed floors for #2 (scroll_heavy ≥ 16 and ascii ≥ 58 MB/s) are **not met** in wall-clock. Should they be re-measured with a long-lived grid, the way `bench_frame_time` does? Unconfirmed.
2. Is snapshot encode speed against libghostty a hard target? It is not reliably met on the second host.
3. Incremental snapshot design: per-row seqno, stable row index, and how parser state and modes travel on attach. Undecided.
4. When does the compact cell (backlog row 5) land, and how does it bump the snapshot version? Deferred by D10.
5. Starting #3 (GUI frame path; #9 first) needs the owner's go-ahead. Not given yet.
6. ConPTY spike 1 results and a Windows fidelity corpus: pending on the owner's side.
7. A config key to make mode 2027 clustering the default: none exists. Is one wanted? Unconfirmed.
8. Is the backlog's source reading compatible with the mux's clean-room rule? See the ⚠ above. Unconfirmed.

## 4. Action items

| Item | Touches | Status |
|---|---|---|
| PR #940: frame-time benchmark (`bench_frame_time`, `render::composite`) | `rusty_term` (gui) | Merged (`5c24bf8c`) |
| Backlog #3 = rows 8 + 9 + 7 (short lock, row map once per frame, dirty rows) | `rusty_term` gui, `rusty_term_core` | Not started; needs the owner's go-ahead |
| Incremental snapshot: per-row seqno plus stable row index | `rusty_term_core`, future `rusty_mux_proto` | Not started; belongs to the mux protocol work |
| Compact cell (row 5) and contiguous scrollback (row 6) | `rusty_term_core`, snapshot version | Deferred (D10) |
| PTY read batching (row 10) | `rusty_term` backends; relevant to `rusty_mux_server` | Not started (P2) |
| Issue #850: `l13` pulls in tokio through `rusty_lsp` | `rusty_term_core`, `rusty_lsp` | Open, low priority (D14) |
| ConPTY spike 1 | the mux PTY layer on Windows | Owner pending |
| Stale branch `claude/ctutils-rustsec-2026-0385` (from the closed #944) | repo hygiene | Still on the remote; delete if wanted |

## 5. References

**Pull requests** (Rusty-Mill/rusty_mill; all merged unless noted)

| PR | What |
|---|---|
| #851 | Core split and perf backlog |
| #857 | Fidelity corpus |
| #862 | BCE |
| #872 | Instruction-count gate |
| #894 | Widths and mode 2027 |
| #925 | DECCOLM |
| #930 | Ring scroll |
| #936 | ASCII fast path |
| #939 | Snapshot |
| #940 | Frame bench |
| #943 | ctutils 0.4.4 (not from this session) |
| #944 | ctutils 0.4.5 duplicate, closed |

**Issues:** #850 (tokio in `l13`, open), #854 (BCE, closed by #862), #855 (widths, closed by #894), #856 (DECCOLM, closed by #925).

**Branch:** `claude/new-session-s68lms`, reused for every PR in this session.

**Files and docs**
- Research:
  - `docs/research/RUSTY-TERM-PERF-BACKLOG.md`
  - `docs/research/MUX-ADOPT-DECISIONS.md`
  - `docs/research/rusty-term-perf/FIDELITY.md`, `oracle_diff.sh`, `rtbench/`, `ghbench/`
- Mux project: `docs/projects/mux/{CHARTER,DECISIONS,RISKS,SPIKES}.md`
- Core engine:
  - `crates/libs/ui/rusty_term/core/src/grid.rs`
  - `crates/libs/ui/rusty_term/core/src/grid/snapshot.rs`
  - `crates/libs/ui/rusty_term/core/src/parser.rs`
- Core tests:
  - `crates/libs/ui/rusty_term/core/tests/fidelity.rs`
  - `crates/libs/ui/rusty_term/core/tests/fidelity/{corpus/,record.py}`
  - `crates/libs/ui/rusty_term/core/tests/snapshot_alloc.rs`
- Front-end:
  - `crates/libs/ui/rusty_term/src/bin/{bench_vt_throughput,bench_frame_time}.rs`
  - `crates/libs/ui/rusty_term/src/gui/{render,bench}.rs`
- Benchmarks: `crates/libs/ui/rusty_term/bench/{ir_gate.py,ir_ceilings.txt,README.md}`
- Change fragments: `changes/2026-10-2{1,2}-rusty-term-*.md`, `changes/2026-10-21-mux-adopt-decisions.md`, `changes/2026-10-23-ctutils-security-update.md`
- Brief: `RUSTY-TERM-PERF-BRIEF.md` (owner upload, not in the repo).

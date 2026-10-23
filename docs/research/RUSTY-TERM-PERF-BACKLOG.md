# rusty_term performance backlog from prior art

Date: 2026-10-21 · Status: **accepted by the owner 2026-10-21** (backlog order and headless split) · Base: `main` @ `434ed13b`

Paths below are as of that base. The headless split has since moved `src/core/` to the `rusty_term_core` crate at `crates/libs/ui/rusty_term/core/src/`; file names and line numbers are unchanged apart from `mod.rs`, which became `lib.rs`.

This answers the brief "rusty_term performance backlog from prior art" (owner upload, not in the repo). It follows `MUX-ADOPT-DECISIONS.md`, which picked `rusty_term` as the single `term` engine for the mux and named its speed and fidelity against libghostty as the main open risk.

**Reference clones (read-only, outside the repo):**

| Project | Commit | License |
|---|---|---|
| Ghostty | `521cf2c5` | MIT |
| Alacritty | `5421470`, with vte 0.15.0 | Apache-2.0 |
| WezTerm | `e2a98e1` | MIT |
| kitty | `8e4780f` | GPL-3.0; described in my own words, nothing copied |
| Zed/GPUI | public blog posts only | gpui is Apache-2.0 |
| Warp | public blog posts only | closed source |

No reference code is pasted below. Reference paths are relative to each project's root. rusty_term paths are relative to `crates/libs/ui/rusty_term/`.

## 1. Summary

- **rusty_term core is 1.6x to 4.5x slower than libghostty-vt** on every workload, when both keep 10k lines of scrollback.
- **It is up to 30x slower** than libghostty-vt with libghostty's default configuration, which keeps almost no history.
- **One function causes most of it.** `Grid::scroll_up` memmoves the whole screen on every line feed (`src/core/grid.rs:1938-1940`) and heap-allocates a fresh `Vec<Cell>` for the scrollback line (`:1905-1908`). callgrind puts **92% to 97% of all instructions** in that memmove.
- **Memory is 3x higher:** 24.4 B/cell against 8.3 B/cell.
- **Resize is 6x to 13x slower:** 86 ms against 10.8 ms for a 10k-line reflow.
- **There is no snapshot serializer.**
- **Fidelity looks good on the synthetic corpus.** Base glyphs match libghostty on all six workloads. vttest and captured sessions were not run (section 3).
- **The headless split is cheaper than the mux note assumed.** `src/core/` already uses no tokio, gui, runtime, config or parking_lot. Its only external crates are `unicode-width` and `unicode-segmentation`. The split is a crate move.

## 2. Baseline results

Machine: Linux cloud container, 4 vCPU Intel Xeon @ 2.10 GHz, rustc 1.99.1, `--release`. All numbers are from this one machine, so they are comparable with each other only. **Windows was not run** (no Windows host here).

Commands to rerun everything:

```sh
cd crates/libs/ui/rusty_term
python3 bench/gen_workloads.py                                   # 6 × 4 MB deterministic workloads
cargo run --release --bin bench_vt_throughput -- 10              # rusty_term core, 120x40, 4 KiB chunks
cd ../../../../docs/research/rusty-term-perf
(cd rtbench && cargo run --release)                              # memory / resize / snapshot proxy
(cd ghbench && cargo run --release)                              # same, libghostty-vt oracle
(cd ghbench && cargo run --release -- ../../../../crates/libs/ui/rusty_term/bench/workloads)
(cd rtbench && cargo run --release -- dump <file.vt>)            # fidelity dumps; diff against ghbench's
```

The harnesses are standalone crates with their own `[workspace]`, so they are not workspace members.

- `ghbench` pulls libghostty-vt from the phall1 fork (`libghostty-rs@fd9d9001`, prebuilt `.a` fetched at build time). It is a **comparison oracle only** and is not on any rusty_term path.
- Upstream libghostty-vt was not measured: its from-source Zig build is blocked here (`MUX-ADOPT-DECISIONS.md` spike 3).

### 2.1 Throughput (headless parser + grid, no renderer)

MB/s, 120x40, 4 KiB `advance`/`vt_write` chunks. Each figure is the mean of 2 runs; the two runs differed by 10% or less.

| Workload | rusty_term | libghostty-vt, 10k lines kept | libghostty-vt, default config (≈ no history) | rusty_term gap (10k lines) |
|---|---:|---:|---:|---:|
| ascii_throughput | 13.4 | 58.3 | 262.6 | 4.4x |
| unicode_heavy | 12.9 | 44.5 | 45.8 | 3.4x |
| sgr_churn | 11.6 | 25.8 | 27.0 | 2.2x |
| cursor_thrash | 90.3 | 148.3 | 138.7 | 1.6x |
| scroll_heavy | 4.3 | 16.3 | 130.8 | 3.8x |
| alt_screen_flicker | 7.3 | 32.2 | 26.5 | 4.4x |

**Like-for-like comparison.** libghostty's default byte limit is 10,000 *bytes*, which keeps almost no history. The like-for-like column sets `set_scrollback_max_bytes(None)` and `set_scrollback_max_lines(10_000)`; rusty_term keeps `SCROLLBACK_MAX = 10_000` lines (`src/core/grid.rs:21`).

**Observation.** libghostty's own scroll throughput drops 8x when it has to prune by line count (131 → 16 MB/s). The cause was not investigated. Do not copy page-granular line pruning without measuring it.

**Profile.** `valgrind --tool=callgrind bench_vt_throughput 1 <300 KB slice>`:

| Slice | memmove share of instructions | Caller |
|---|---:|---|
| scroll_heavy | **97.1%** | `Grid::scroll_up` |
| ascii_throughput | **91.8%** | `Grid::scroll_up`, 31,944 calls |

Next are `put_char` (2.8%), `set_cell` (1.7%) and `ground_byte` (1.4%). By Amdahl's law, removing the memmove bounds the gain at **≈12x to 34x** on those slices. That figure is *estimated from the profile, not measured*.

### 2.2 Scrollback memory

`rtbench` and `ghbench`: fill 10,000 lines of history at 200x50, one SGR colour change per line.

| | rusty_term | libghostty-vt |
|---|---:|---:|
| Fill time | 121-172 ms | 24 ms |
| RSS delta | 47,816 KiB | 16,388 KiB |
| Bytes per cell | **24.4** | **8.3** |
| Peak RSS (VmHWM) | 52,232 KiB | 21,660 KiB |
| Rows retained | 10,000 + 50 | 9,768 + 50 (page granularity) |

- rusty_term's `Cell` is 24 bytes: `ch` u32, `cluster` u16, `fg`/`bg` u32 each, `flags` u16, `link` u16, `underline_color` u32 (`src/core/cell.rs:155-176`).
- On top of that, each scrollback `Line` is a separate heap `Vec<Cell>` (`src/core/grid.rs:815-818`).
- Per 10k lines at 200 columns: **≈46.7 MiB against ≈16 MiB**.

### 2.3 Resize and reflow

Full 10k-line history at 200x50, consecutive resizes in this order:

| Resize to | rusty_term | libghostty-vt |
|---|---:|---:|
| 150x50 | 86.0-111.4 ms | 10.8 ms |
| 200x50 | 20.1-27.3 ms | 3.4 ms |
| 80x24 | 17.4-18.2 ms | 2.9 ms |
| 200x50 | 14.2-14.7 ms | 1.5 ms |

### 2.4 Snapshot

| | rusty_term | libghostty-vt (fork) |
|---|---|---|
| Full-state encode (200x50 + 10k lines) | **absent**: no serializer exists | `encode_snapshot`: 11.17 ms, 7,813,238 bytes (≈3.9 B/cell) |
| Closest existing read | `snapshot_viewport` (viewport clone only, `src/core/grid.rs:4772`): 0.011 ms | n/a |

### 2.5 Fidelity

Each of the six workloads was fed to both engines, and the final 120x40 screens were diffed as text.

- **Base glyphs match on all six.**
- The 1-line diffs on ascii, scroll and sgr come from my harness, not the engines: libghostty's formatter trims trailing blank rows, which shifts the 40-line window by one.
- unicode_heavy differs only in combining marks. The rusty_term dump prints `Cell::ch` and drops grapheme clusters, so marks were not compared.

**Not run:**
- vttest (interactive).
- Captured tmux, vim/neovim, htop and RTL sessions; none were captured here.
- A cell-level diff of attributes and colours.

These remain open. A real corpus plus an attribute-level dump is the next fidelity step (backlog row 14).

### 2.6 Not run here

- **vtebench.** It must run inside the terminal under test on a real TTY, so it needs rusty_term's GUI under a display server. Command, from the Alacritty study:
  ```sh
  xvfb-run -a <terminal> -e sh -c 'vtebench -b ./benchmarks --dat out.dat'
  ```
  Not attempted for rusty_term's winit window.
- **GPU frame time under flood.** Needs a GPU and display.
- **Windows.** Needs the owner's box.

I did not estimate any of these.

## 3. Technique backlog

Columns:
- **Who:** where a reference does it.
- **rusty_term today:** with file:line.
- **Win:** measured, or "unmeasured".
- **Effort:** S < 1 day, M = days, L = a week or more.
- **Prio:** P1 = do first, P3 = later.

| # | Technique | Who does it (file) | rusty_term today | Expected win | Effort | Risk | Prio |
|---|---|---|---|---|---|---|---|
| 1 | **Scroll by index rotation, not memmove** (ring of rows or a row-index map) | Alacritty `alacritty_terminal/src/grid/storage.rs` (`rotate`, `compute_index`); kitty line buffer with a line-index map; Ghostty `PageList.grow` (row-count bump) and `eraseRow` rotating 8-byte row headers | `scroll_up` copies `(bottom-top)*cols` cells per line (`src/core/grid.rs:1938-1940`); likewise `scroll_band_*` (`:1561-1595`) and insert/delete line (`:3206-3270`) | measured hot spot: 92-97% of instructions; bound ≈12-34x (estimated) | M | M: `cells[y*cols+x]` indexing is spread through grid.rs (4,969 lines); needs one row accessor first | **P1** |
| 2 | **Recycle the evicted scrollback row** instead of `to_vec` + `pop_front` drop | Alacritty `Row::reset` on the rotated-in row (`grid/row.rs`); Ghostty reuses the popped first page as the new last page | new `Vec<Cell>` alloc per scrolled line, free on eviction (`src/core/grid.rs:1905-1912`) | unmeasured (alloc/free per line feed) | S | L | **P1** (with #1) |
| 3 | **Printable-run fast path in the parser**: scan to the next ESC/C0, validate UTF-8 once, hand the run to the grid | vte 0.15 `advance_ground` (memchr for ESC + `str::from_utf8`); Ghostty `stream.zig` `nextSlice` + SIMD `utf8DecodeUntilControlSeq` (commit 083d9709: 272 → 287 MB/s); kitty SIMD decode until ESC | one `match` per byte, `ground_byte` → `put_char` per char (`src/core/parser.rs:230-233`, `:707-780`) | unmeasured; matters only after #1 (parser is ≈2% today) | M | L | **P2** |
| 4 | **Batched `put_str`**: wrap, insert, charset and link checks once per run; fill cells row-wise; mark dirty once per row | Ghostty `Terminal.zig` `printSlice`/`printSliceFast` (commit 47e26df6: ASCII 784 → 138 ms) | per-char `put_char` → `set_cell` with wide-partner checks (`src/core/grid.rs:1975-2055`, `:1360`) | unmeasured; Ghostty reports 5.7x on its ASCII case | M | M: wide/grapheme edge cases; covered by `src/core/tests.rs` | **P2** |
| 5 | **Compact cell + style interning**: style id per cell, styles in a refcounted table; rare attrs (link, underline colour, cluster) off the hot cell | Ghostty `page.zig` Cell `packed struct(u64)`, `style.zig` + `ref_counted_set.zig`; Alacritty `CellExtra` behind `Option<Arc>`; WezTerm `FatAttributes` box; kitty CPU/GPU cell split | 24-byte `Cell` with inline fg/bg/underline colour (`src/core/cell.rs:155-176`) | measured gap: 24.4 vs 8.3 B/cell (2.9x memory); fill 5-7x slower | L | H: touches every Cell consumer (gui, render, snapshot); `Copy` semantics change | **P2** |
| 6 | **Scrollback in contiguous blocks** instead of one heap `Vec` per line | Ghostty pages (mmap, offset-addressed, memcpy-cloneable); kitty history in lazily mapped 2048-line segments; Alacritty one `Vec<Row>` ring | `VecDeque<Line{Vec<Cell>, bool}>` (`src/core/grid.rs:815`, `:418`) | unmeasured (allocator overhead, locality); prerequisite for cheap snapshot | M | M | P2 (with #5) |
| 7 | **GUI consumes dirty rows**; rebuild only changed rows' instances; per-line render cache | Ghostty `renderer/generic.zig` `rebuildCells` (partial vs full); WezTerm `line_quad_cache` keyed by line seqno + hash; kitty reshapes only lines with dirty text | core keeps `Grid::dirty`, but the GUI ignores it: full `buffer.fill` on CPU (`src/gui/render.rs:136`), all instances rebuilt on GPU (`src/gui/gpu.rs:1285-1388`); "no per-row dirty tracking yet" (`src/gui/shape.rs:52-58`) | measured by `bench_frame_time`: frame 1.3-2.7 ms p50 at 120x40 with every row repainted | M | M | **P1** for GUI |
| 8 | **Short lock, then render without it** (two-phase snapshot); fair handoff so parse cannot starve frames | Ghostty `render.zig` `beginUpdate`/`endUpdate` (commit 446f80f4: lock hold 2.7-11x shorter), `lockDemand`/`yieldToDemand` (d34b54e9); Alacritty `FairMutex` + `lease` + `MAX_LOCKED_READ` 64 KiB | GUI holds every visible pane's grid lock across render **and present/vsync** (`src/gui/window.rs:1296-1389`; `gpu.rs:1564` `get_current_texture` can block) | measured by `bench_frame_time`: lock hold 0.95-2.63 ms p50, 2.1-7.3 ms p99, up to 12.3 ms max per 60 Hz frame over two hosts (target < 1 ms) | S-M | M | **P1** for GUI |
| 9 | **Remove the O(cells × fold_blocks) view mapping**: compute `display_history_len` and the row map once per frame | n/a (rusty_term-specific) | `abs_of_view_row` → `display_history_len` → walks up to 1,024 `fold_blocks` per cell access, about 4 accesses per cell in the CPU path (`src/core/grid.rs:2139-2178`, `src/gui/cpu.rs:192,236,333,363`) | measured by `bench_frame_time`: with the same text, 1,024 fold blocks take a 120x40 frame from 1.02 to 8.79 ms p50 (live, 8.6x) and from 1.02 to 13.47 ms (scrolled back, 13x) | S | L | **P1** for GUI |
| 10 | **Larger PTY reads + coalesced wakeups**: 64 KiB to 1 MiB buffer, parse until EAGAIN or a budget, one wakeup per batch | Ghostty `termio/Exec.zig` (4 × 64 KiB rotating buffers, 3 ms budget); Alacritty 1 MiB read buffer, 64 KiB per lock; WezTerm 1 MiB read + 3 ms coalesce; kitty `input_delay` 3 ms | 4 KiB heap buffer per read (`src/backend/unix.rs:290`, `windows.rs:339`); one `advance` + one `Redraw` event per read on an unbounded proxy queue (`src/gui/window.rs:385-402`), each event takes the grid lock ~4 times (`:5356-5378`) | unmeasured | S | L | P2 |
| 11 | **Instanced GPU draw with persistent buffers**; bg as a flat per-cell colour buffer; cursor as an overlay | Ghostty `renderer/cell.zig` (flat `bg_cells`, per-row `fg_rows`, cursor in reserved rows); kitty one instanced draw of rows×cols; Alacritty `BATCH_MAX` instancing; GPUI one instanced call per primitive type | already instanced (1 base + 1 overlay + 1 per image), but **3 new GPU buffers via `create_buffer_init` every frame** (`src/gui/gpu.rs:1391-1412`) and per-frame `FrameLists`/`Resolved` Vecs (`:1302`, `:891`) | unmeasured | S | L | P2 |
| 12 | **Glyph and shape caches without per-frame allocation** | Ghostty `font/shaper/Cache.zig` (fixed table keyed by run hash); WezTerm borrowed-key glyph lookup; Alacritty preloads ASCII per face | glyph cache `HashMap<(char,Style), Rc<Glyph>>` hit per cell per frame (`src/gui/font.rs:124,298`); shape cache keyed by `Vec<u16>`, cloned on hit, wholesale clear at 4,096 (`src/gui/shape.rs:35,115-133`); a miss re-parses the face (`:120`); atlas full → drop everything and repack (`gpu.rs:621-636`) | unmeasured | M | L | P3 |
| 13 | **Synchronized output (2026): buffer the bytes, timer-driven expiry** | vte `ansi::Processor` buffers raw bytes, 150 ms timeout, 2 MiB cap, and Alacritty's poll uses the deadline; Ghostty 1 s timer that wakes the renderer | honoured with an 800 ms timeout (`src/core/grid.rs:28`, `:1522-1537`); expiry is only noticed on the next event; TUI `render_once` paints mid-sync on input/resize (`src/render.rs:249`) | correctness/smoothness, not throughput | S | L | P2 |
| 14 | **Fidelity corpus + attribute-level diff harness** | Ghostty and Alacritty ref tests (recorded sessions + expected grids) | `src/core/tests.rs` (6k lines of unit tests); no recorded-session corpus | prerequisite for safely doing #1, #4 and #5 | M | L | **P1** (gate) |
| 15 | **Reflow: recycle buffers, avoid full rebuild when width is unchanged** | Ghostty `PageList.resize` (no reflow if cols are unchanged; recycles source pages as destinations; 25 ms resize coalescing in `termio/Thread.zig`); Alacritty `grid/resize.rs` lines-only path | `reflow_history` rebuilds scrollback + screen (`src/core/grid.rs:916`, `:2942-2990`) | measured gap: 86 vs 10.8 ms (150-col shrink, 10k lines) | M | M | P3 |
| 16 | **Snapshot serializer: full + incremental** (per-row change seqno, stable row index, style/link side tables) | Ghostty fork `src/terminal/snapshot/` (CRC-framed records, READY marker, parser continuation); WezTerm `Line.seqno` + `changed_since`, `StableRowIndex`, `GetPaneRenderChanges` with dirty ranges + "bonus lines", hyperlinks sent once (`codec/src/lib.rs`, `wezterm-mux-server-impl/src/sessionhandler.rs`) | **absent**; viewport clone only (`src/core/grid.rs:4772`); `epoch` is per-grid, not per-row (`:394`) | target to beat: 11.2 ms / 7.8 MB for 200x50 + 10k lines (libghostty fork) | M-L | M | **P1** for mux |
| 17 | **Scrollback compression of idle history** | Ghostty LZ4 for cold pages (`src/terminal/compress/`, upstream); kitty pager-history ring | none; `rusty_compress` is stored-block DEFLATE only (`crates/foundation/rusty_compress/src/lib.rs:3-7`) | unmeasured | L | M | P3 |
| 18 | **TUI output: one frame-sized buffer, fewer full SGRs** | Ghostty formatter emits minimal SGR deltas | per-row `write!` through std's 1 KiB `LineWriter`; `sgr_for` allocates a `String` per style change and always emits full 24-bit SGR (`src/render.rs:83,141-207`) | unmeasured | S | L | P3 |
| 19 | **SIMD scan of OSC/APC/DCS bodies** (Kitty/sixel payloads) | Ghostty `consumeOscString`/`consumeApcString`; kitty two-byte terminator search | per-byte state machine | unmeasured; only matters for image-heavy streams | S | L | P3 |

Rows 1-6 and 13-17 are core (headless). Rows 7-12 and 18 are front-end.

## 4. Top 5 recommended changes, in order

Each change comes with the benchmark that proves it. All core changes run `cargo test -p rusty_term_core -p rusty_term --all-features` (the core tests live in `rusty_term_core` since the split) plus the row 14 fidelity diff before merge.

1. **Fidelity gate (#14), small first.** Record a corpus (vttest screens, vim, htop, tmux, wide/RTL) as `.vt` files plus expected grids from libghostty-vt. Add `rtbench dump` with attributes and clusters, and diff in CI against stored expectations, not against libghostty at runtime (sovereignty). Proof: 0 cell diffs on the corpus before and after each of the next changes.
2. **Ring-buffer screen + recycled scrollback rows (#1 + #2).** Introduce a row accessor, swap `cells: Vec<Cell>` for rows addressed through an offset or index map, and reuse the evicted history row. Proof:
   - `bench_vt_throughput` scroll_heavy ≥ 16 MB/s and ascii_throughput ≥ 58 MB/s (libghostty-vt parity at 10k lines).
   - callgrind memmove share < 10%.
   - `rtbench` fill time < 30 ms.
3. **GUI frame path (#8 + #9 + #7).** Snapshot under the lock, render without it; compute the view-row map once per frame; rebuild only dirty rows. Proof:
   - Frame-time bench: `bench_frame_time` (`crates/libs/ui/rusty_term/bench/README.md`) drives a `Grid` and the CPU frame path offscreen and reports p50/p99 frame ms and lock hold time while the workloads stream in. Baseline (2026-10-22/23, two hosts): lock hold 0.95-2.63 ms p50; 1,024 fold blocks make a frame 8.6x (live) to 13x (scrolled back) slower.
   - Target: lock hold < 1 ms per frame, steady frame time with 1,024 fold blocks.
4. **Printable fast path + batched `put_str` (#3 + #4).** Use std-only SWAR (u64-at-a-time) to scan for ESC/C0 and `str::from_utf8` for validation. No new dependency: `rusty_simd` holds ML kernels, not byte scanning, and `memchr` would be a new third-party dependency to justify. Proof: ascii_throughput and sgr_churn after #2; callgrind `ground_byte` + `put_char` share halves.
5. **Snapshot serializer (#16)**, built with #5/#6 in view, on `rusty_wire` (varint already planned in the mux note). Add a per-row change seqno. Proof: `rtbench` encode of 200x50 + 10k lines ≤ 11.2 ms and ≤ 7.8 MB (fork oracle); round-trip decode equals the source grid under the #14 dump.

**Deferred.** Compact cell (#5) is the largest memory win (≈3x) but the riskiest change, because it touches every Cell consumer. Do it after #2 and #16 settle the row and serialization shape, so the cell format changes only once.

## 5. Where the headless split and the snapshot serializer fit

- **Headless split first, as a pure move.** `src/core/` already depends only on `std`, `unicode-width` and `unicode-segmentation`. A grep for `tokio`, `crate::gui`, `crate::runtime`, `crate::config` and `parking_lot` under `src/core/` finds nothing.
  - The `gui`/`test`-gated modules (bidi, arabic, search via `rusty_regx`) become features of the new crate.
  - `rusty_base64` and `rusty_term_l13` stay behind their existing gates.
  - Doing the move before #1 means every perf PR lands in the crate the mux and server will link, and `bench_vt_throughput` moves with it. **Needs owner approval** (it is prerequisite 1 in `MUX-ADOPT-DECISIONS.md`).
- **The serializer comes after the storage change.** Serializing today's `VecDeque<Line>` of 24-byte cells would bake the current layout into a wire format that #1/#5/#6 then break.
  - Order: split → ring buffer (#1/#2) → per-row seqno + stable row index (WezTerm pattern) → serializer (#16) → compact cell (#5) behind a version bump of the snapshot envelope.
  - The per-row seqno serves both the GUI dirty path (#7) and incremental mux sync. It is one mechanism with two callers, so it is not speculative.

## 6. What rusty_term already does well

Nothing was measured faster than libghostty-vt. The following are the qualitative strengths found:

- **Zero C/Zig toolchain.** The core is pure Rust with two Unicode crates. libghostty-vt needs Zig and a 34-package graph from source, or a prebuilt `.a` fetched at build time.
- **Exact line-count scrollback.** rusty_term keeps exactly 10,000 lines. libghostty prunes at page granularity (it kept 9,768 here), and its scroll throughput fell from 131 to 16 MB/s under a line limit.
- **Dirty rows already tracked in the core** (`Grid::dirty`, `snapshot_dirty`) and used by the TUI renderer. Ghostty and Alacritty built this too; rusty_term only needs the GUI to consume it.
- **Hyperlink and cluster tables are already Arc-snapshotted** per frame (`links_arc`, `clusters_arc`, `src/core/grid.rs:443-454`). That is O(1) per frame, the same idea as WezTerm's side tables.
- **The GPU path is already instanced** with one shelf atlas. kitty and Ghostty use the same shape, and WezTerm does not instance at all.
- **Synchronized output (2026)** is honoured in both front-ends with a timeout.

## 7. Sources read

**rusty_term:**
- `src/core/{cell,grid,parser,mod}.rs`
- `src/gui/{window,cpu,gpu,render,shape,font,access}.rs`
- `src/render.rs`
- `src/runtime/tokio_rt.rs`
- `src/backend/{unix,windows}.rs`
- `src/bin/{bench_vt_throughput,bench_metrics}.rs`
- `bench/README.md`

**Ghostty:**
- `src/terminal/{parse_table,Parser,stream,Terminal,page,PageList,Screen,style,ref_counted_set,hyperlink,render,formatter}.zig`
- `src/terminal/snapshot/`
- `src/simd/vt.{zig,cpp}`
- `src/renderer/{Thread,generic,State,cell}.zig`
- `src/font/{Atlas,SharedGrid}.zig`, `src/font/shaper/Cache.zig`
- `src/termio/{Exec,Thread,Termio,stream_handler}.zig`
- `src/config/Config.zig`
- Commits 083d9709, 47e26df6, 446f80f4, d34b54e9, 2f0e6659, e7969ed4, 68beeeb3

**Alacritty:**
- `alacritty_terminal/src/{grid/{storage,mod,row,resize},term/{mod,cell},event_loop,sync}.rs`
- `alacritty/src/{display/{mod,damage},renderer/{text/{glyph_cache,atlas,glsl3,gles2,mod},rects},scheduler}.rs`
- vte 0.15.0 `src/{lib,ansi}.rs`
- vtebench `src/{bench,cli}.rs`, `benchmarks/`

**WezTerm:**
- `wezterm-cell/src/lib.rs`
- `wezterm-surface/src/{line/{line,storage,clusterline},cellcluster}.rs`
- `term/src/{screen,terminalstate/mod}.rs`
- `vtparse/src/{lib,transitions}.rs`
- `wezterm-gui/src/{glyphcache,shapecache,quad,renderstate,termwindow/render/*}.rs`
- `mux/src/{lib,pane,renderable}.rs`
- `codec/src/lib.rs`
- `wezterm-mux-server-impl/src/sessionhandler.rs`
- `wezterm-client/src/pane/renderable.rs`

**kitty** (read and described only, nothing copied):
- `kitty/{vt-parser,simd-string,history,line-buf,screen,fonts,shaders,child-monitor,gl}.c`
- `line.h`
- `options/definition.py`

**Web:**
- Zed, "Leveraging Rust and the GPU to render user interfaces at 120 FPS", https://zed.dev/blog/videogame
- Warp, "How Warp Works", https://www.warp.dev/blog/how-warp-works
- Warp, "Warp for Linux", https://www.warp.dev/blog/warp-for-linux

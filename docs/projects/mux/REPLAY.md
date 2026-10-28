# Phase 1 attach: RTCP v4 headless checkpoints

The former experimental RMRP lifetime replay mechanism is removed. There is
no cumulative output cutoff, no replay log, and no reduction of the engine's
normal 10,000-row history. Each attach captures the *current* Grid and parser.
A block can be detached and reattached repeatedly after arbitrarily much past
output, subject to current-state memory limits.

## Negotiation and transfer

`Features::CHECKPOINT_V4` is bit 16 (value 65536). Bits 0, 1, 2 and 3 are
reserved for rejected RMRP replay, incomplete RTCP v1, superseded fixed-width
RTCP v2 and superseded RTCP v3 respectively; none enables attach. Hello/Welcome
negotiate bit 16; Attach without it returns Failed. The server offers no older
version, and the client refuses a Welcome without bit 16, so a v3 peer on
either side fails closed instead of restoring v3's baked-in default colours.
Existing frame layouts are unchanged. The drawable RTSN snapshot is a separate
format: it writes v2 (default fg/bg in the header, #1163) and still decodes v1.

A successful attach sends Done, then ordered `Checkpoint` messages (type `0x88`),
then Ready, then raw Output and subsequent model events. A Checkpoint contains
block (field 4, UUID), offset (field 15, varint), total encoded length (field 16,
varint), and bytes (field 9). Each payload is at most 65,536 bytes. Offsets begin
at zero and must be contiguous; total is positive, constant, and at most 256 MiB.
Ready is valid only once exactly total bytes have arrived and RTCP decoding
succeeds. The server queues Ready only after all checkpoint chunks. Encoding,
queue, or writer failures close the connection without emitting an incomplete
Ready. `server::checkpoint::Receiver` implements bounded assembly and validation.

Capture and queue insertion run synchronously under the single-threaded state
borrow, so the checkpoint precedes every later output/resize mutation. The
writer emits its chunks lazily before draining subsequent queued output/events.
There is at most one checkpoint pending/in flight per connection; another Attach
fails while transfer is pending. Other requests can proceed. No checkpoint-sized
collection of separately encoded frames is queued.

The current checkpoint image is bounded separately from the **8 MiB live-tail
queue**. Overflow evicts the connection and drops the checkpoint, queued tail,
reader/writer futures and socket halves. It does not modify the block or its
ability to produce a fresh checkpoint for another attach. A blocked non-reading
peer cannot retain the transfer indefinitely after eviction.

## RTCP v4 binary format

The codec is `rusty_term_core::checkpoint::{encode,decode}`. Magic `RTCP` is
followed by version byte `4`, then Grid fields and AnsiParser fields in the
explicit order in their `Wire` implementations. Field order is part of v4;
changing it or its meaning requires another version. This is an independent
continuation format, not a reinterpretation of RTSN's drawable snapshot.

- Integers are fixed-width little endian; usize/counts use u64 and reject
  overflow on narrower hosts. Characters are validated Unicode scalar u32.
- Booleans and Option presence are exactly byte 0 or 1; a present Option is
  followed by its value. Invalid tags fail.
- Strings are u64 byte length plus validated UTF-8. Vec, VecDeque and Arc slices
  are u64 element count plus elements; arrays omit the count; tuples concatenate.
- Enum tags are zero-based u8 in each explicit `variants!` list. Cells, lines,
  saved screens, palettes, image frames and transmissions use the corresponding
  explicit `fields!` order. These lists are the authoritative schema beside
  the types they encode; they never serialize Rust object memory or pointers.
- A line is a LEB128 varint cell count (at most 4096, checked before
  allocating), then runs that cover exactly that many cells, then the wrapped
  flag and line attribute. A run is a style reference, a varint
  `len << 1 | repeat`, then one UTF-8 scalar repeated `len` times or `len`
  UTF-8 scalars. A style is every Cell field but `ch`, in Cell's field order.
  Style reference 0 defines the next style in a per-checkpoint table; `i + 1`
  reuses entry `i`. The writer uses a repeat run for four or more identical
  cells. Zero-length runs, runs past the count, unknown style indices, varint
  overflow and invalid UTF-8 fail.
- Process-local timer Instants store elapsed seconds u64 and nanoseconds u32 at
  capture, rebased at restore. Transfer latency is excluded from timer ages.
- Unknown versions, trailing bytes, truncation, invalid values/indices and
  exceeded budgets fail before returning a terminal for continued parsing.

The headless profile includes live/primary screens, full retained history,
wrapped/line attributes, tables and current hyperlink, cursor/saved cursor and DECTCEM visibility,
tabs, vertical/horizontal margins, origin/insert/wrap/grapheme modes, input
modes, title and keyboard stacks, palette/reset theme, erase colors, pending
multipart notifications, stored Kitty images/frames/virtual placements and
animation state. The parser includes its pen/protection, all charsets and
shifts, REP character, partial UTF-8/CSI/OSC/DCS/APC, private/intermediate markers,
partial Kitty transmission, palette, and pending terminal responses.

Front-end selection/search/folds, render caches and spare allocations, pending
host clipboard/notification/bell effects, and L13 application channel state are
outside this *headless VT* profile. Pixel overlays present only in GUI builds
are front-end state; headless Sixel/iTerm display cells and Kitty stored pixels
are preserved. Enabling a library feature does not change RTCP v4's schema.
The Phase 1 server does not enable GUI or L13. Clients must not treat RTCP as a
GUI/L13 session checkpoint or replay host actions during restore.

## Size and speed

Fixed-width v2 cells cost 22 bytes each, so a 200x50 terminal with 10,000
history rows checkpointed to 44.3 MB. Run encoding brings it to the drawable
RTSN snapshot's size. Measured in release builds on one machine, the best of
15 runs in each of three sessions, on 12,000 lines of eight-colour text with
ragged tails:

| Grid | v2 size | v3 size | RTSN v1 | v2 encode | v3 encode | v2 decode | v3 decode |
|---|---:|---:|---:|---:|---:|---:|---:|
| 120x40 + 10k | 26.61 MB | 0.62 MB | 0.63 MB | 29.1 ms | 9.5 ms | 20.7 ms | 11.7 ms |
| 200x50 + 10k | 44.32 MB | 0.64 MB | 0.65 MB | 67.3 ms | 18.9 ms | 45.1 ms | 24.6 ms |

Encoding still runs under the server's state borrow, so its time is a stall
for the server's other blocks. The table above predates the 8-byte cell.
Re-measured on 2026-10-27 (`checkpoint::encode` alone, same workload and
method, a 4-vCPU cloud host; the bytes are identical before and after):

| Grid | before the 8-byte cell (#1058) | 8-byte cell (#1090) | per-run style checks |
|---|---:|---:|---:|
| 120x40 + 10k | — | 4.1 ms | 2.2 ms |
| 200x50 + 10k | 17.0 ms | 5.4 ms | 2.8 ms |

The last column checks each cell's style id once per run while writing,
instead of in a separate pass over every cell, and finds a run's wire style by
grid style id instead of hashing the resolved style. Encode instructions fell
by 46% (callgrind). What remains is the run scan and UTF-8 output.
`coloured_history_checkpoint_stays_compact` fails if the 200x50 case grows
past twice the RTSN size.
Incremental deltas for reconnecting clients are designed in
[DELTA.md](DELTA.md) and not built.

## Limits

Encoded state is limited to **256 MiB**, independent of output lifetime. The
decoder charges collection/string allocations and rebuilt table caches against
an aggregate **512 MiB** allocation budget (plus fixed object/stack overhead),
checks allocation arithmetic before reserving memory, and validates dimensions,
cursor/margin/table indices, screen widths, parser and image invariants. This
is a current-state budget; an over-budget image fails that attach rather than
permanently disabling later attaches after state changes.

Creation/resize validates before spawn or mutation: at most **4096 per axis**,
**65,536 live-screen cells**, and **4,194,304 potential retained cells**, calculated
as `cols * (10_000 + 2 * rows)` for normal history plus live/alternate screens.
The history row cap stays 10,000. These are screen/history allocation limits,
not a claim that all terminal images/tables fit inside the cell budget.

Linux Phase 1 and rusty_tokio remain unchanged. Child Process ownership survives
reaper cancellation; normal close signals the process group with HUP, escalates
to KILL after one second, and reaps it. Setup/teardown fallback kills and waits.

## Durable-field audit

The review correction adds cursor_visible symmetrically after tab_stops in the
Grid schema and used version 2/capability bit 2 so old v1 layouts fail closed;
v3 (bit 3) changes only the line encoding, so v2 layouts fail closed too.
v4 (bit 16, #1163) keeps v3's layout but changes the meaning of cell and pen
flags: `ATTR_FG_DEFAULT` / `ATTR_BG_DEFAULT` mark a colour that follows the
default (OSC 10/11), and a colour without them is explicit. A v3 image has no
such bits, so it is rejected rather than restored with every colour explicit.
An independent inventory test covers all 87 Grid fields: 60 durable fields are
encoded and decoded, and 27 excluded fields have explicit profile reasons.
Exclusions are rebuildable caches/allocations, client view/render configuration,
GUI-only overlays/folds/timers, pending host effects, or L13 channel/capture state.
The inventory test fails if a new field lacks either encoding or an audited
exclusion. A separate regression asserts restored cursor_visible and concrete
DECRQM responses directly, rather than relying on RTCP re-encoding equality.

# Phase 1 attach: RTCP v1 headless checkpoints

The former experimental RMRP lifetime replay mechanism is removed. There is
no cumulative output cutoff, no replay log, and no reduction of the engine's
normal 10,000-row history. Each attach captures the *current* Grid and parser.
A block can be detached and reattached repeatedly after arbitrarily much past
output, subject to current-state memory limits.

## Negotiation and transfer

`Features::CHECKPOINT_V1` is bit 1 (value 2). Bit 0 is reserved for the rejected
experimental RMRP capability and does not enable attach. Hello/Welcome negotiate
bit 1; Attach without it returns Failed. Existing frame layouts and the drawable
RTSN v1 API remain unchanged.

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

## RTCP v1 binary format

The codec is `rusty_term_core::checkpoint::{encode,decode}`. Magic `RTCP` is
followed by version byte `1`, then Grid fields and AnsiParser fields in the
explicit order in their `Wire` implementations. Field order is part of v1;
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
- Process-local timer Instants store elapsed seconds u64 and nanoseconds u32 at
  capture, rebased at restore. Transfer latency is excluded from timer ages.
- Unknown versions, trailing bytes, truncation, invalid values/indices and
  exceeded budgets fail before returning a terminal for continued parsing.

The headless profile includes live/primary screens, full retained history,
wrapped/line attributes, tables and current hyperlink, cursor/saved cursor,
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
are preserved. Enabling a library feature does not change RTCP v1's schema.
The Phase 1 server does not enable GUI or L13. Clients must not treat RTCP as a
GUI/L13 session checkpoint or replay host actions during restore.

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

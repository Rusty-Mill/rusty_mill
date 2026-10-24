# Phase 1 exact attach checkpoint (RMRP v1)

`rusty_term_core::Grid`'s RTSN v1 snapshot is a drawable snapshot. It is not
sufficient to continue parsing raw PTY output: it omits parser state, input
modes, palette, images, and other state. The Linux Phase 1 server therefore
uses an exact replay checkpoint in `Message::Snapshot.data`.

The existing frame/message layouts and RTSN format are unchanged. Peers must
negotiate `Features::REPLAY_SNAPSHOT` (bit 0, value 1) in Hello/Welcome before
attaching. A client without that feature receives Failed on Attach, never a
misleading Grid snapshot followed by raw bytes. This is a bounded companion
protocol extension, not a new meaning for RTSN. The server crate exports
`checkpoint::restore` for the Phase 1 client. Other implementations can use
the following format with the same terminal engine defaults.

All integers below are fixed-width little endian:

- Header: `RMRP`, version byte `1`, initial columns `u16`, initial rows `u16`.
- Zero or more records: tag `u8`, payload byte length `u32`, payload bytes.
- Tag 0: one complete server parser `advance` call's raw PTY bytes. Drain and
  discard parser responses after each record; do not send them to the PTY.
- Tag 1: resize, payload columns `u16`, rows `u16`. Apply Grid::resize in order.

Begin with Grid::new and AnsiParser::new, using default theme and 16 scrollback
rows. Preserve record boundaries. On success retain both reconstructed objects
for subsequent Output frames. Apply BlockResized events to attached terminals.
Discard reconstructed UI side effects (clipboard requests, notifications) when
installing a checkpoint; do not execute them as new host actions. Restoring a
checkpoint must not generate input to the PTY. Unknown versions, tags,
truncation, invalid dimensions and payloads over the replay budget are errors.
Ready is sent only after the required Snapshot was successfully encoded and
queued; encoding or queue failure disconnects the client.

## Explicit Phase 1 limits

Each block retains at most **4 MiB of encoded replay**, including headers and
resize records. Crossing that budget discards the replay and makes all later
Attach requests for that block fail. The PTY and clients already attached
continue normally. Detach/reattach is therefore guaranteed only while the
block's replay remains available. There is no truncation, guessed parser state,
or fallback to a drawable-only snapshot. A future full engine checkpoint can
remove this lifetime restriction using a separately negotiated version.

Creation and resize accept at most **4096 cells per axis and 65,536 cells per
screen**. Validation precedes spawning, model changes, or PTY/grid mutation.
With the alternate screen and 16 scrollback rows, screen/history cell storage
is bounded by 196,608 cells (excluding parser buffers, images and intern tables,
which retain the terminal engine's existing limits). This is a practical
screen allocation budget, not a promise to bound all process memory. The
outbox budget remains 8 MiB per client plus at most one in-flight output frame.
On eviction, EOF or write failure one owning connection future drops both
halves and its queue, even if the writer is blocked on a non-reading peer.

Closing a block releases the master and transfers its separately owned Process
to a reaper task. It sends SIGHUP to the process group, escalates to SIGKILL
after one second, and polls until reaped. Setup errors and server teardown use
the Process drop fallback (SIGKILL plus consuming wait). This preserves the
concurrent child-lifecycle correction in commit 98979191.

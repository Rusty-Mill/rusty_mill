# Incremental attach: design only

Status: designed, not built. On 2026-10-25 the owner chose to ship compact
RTCP v3 now (#1055) and to keep deltas on paper until a consumer needs them.
The consumers are C05 (resumable event stream, Phase 2) and C13 (roaming,
Phase 3). Nothing here changes a wire format yet.

## Why it waits

A Phase 1 attach is a fresh client with no state, so a delta has nothing to
apply to. The worst Phase 1 case, 200x50 with 10,000 history rows, is already
0.64 MB and about 19 ms to encode in v3 (see [REPLAY.md](REPLAY.md)). Deltas
pay off only when a client reconnects holding recent state, which is what C05
and C13 add.

## What exists

Row 7 (#993) gave `Grid` a process-unique `id`, a strictly increasing
`stamp`, a per-row `row_stamp` for the live screen, and `hist_stamp`, bumped
on any scrollback change (push, eviction, clear, reflow, retheme). None of
these are in the checkpoint: the inventory test lists them as renderer state.

They are not enough on their own. `hist_stamp` says history changed, not how:
it cannot tell "50 lines appended" from "reflowed on resize". The row stamps
cover only the live screen, so the saved primary screen behind the alternate
screen has none.

## Additions (when built)

- `hist_seq: u64`: lines ever pushed into scrollback. The oldest retained line
  is `hist_seq - scrollback_len`, so evictions need no separate count.
- `hist_epoch: u64`: bumped when history is rewritten rather than appended
  (reflow, clear, retheme). A delta never crosses an epoch.
- `screen_gen: u64`: bumped on every alternate-screen enter and leave. Row
  stamps cannot describe the hidden saved screen, so this is what tells the
  server it may have changed.
- A base token: `Base { server, block, grid_id, stamp, hist_seq, hist_epoch,
  screen_gen, cols, rows }`. `server` is a random incarnation id chosen at
  server start and `block` is the block's UUID, so a token is bound to one
  block in one server lifetime. `grid_id` alone is only process-local.
- `Attach { block, since: Option<Base> }` and a feature bit for deltas.

## Keeping the token exact

A token is valid only for the exact grid state it describes. The client keeps
applying raw Output after Ready, so a token sent only with Ready goes stale at
once: history the client already holds would be appended again on reconnect.

So the token advances with the stream. With the delta feature negotiated, every
Output frame carries the token for the server's grid just after that frame's
bytes were applied, and Ready carries it for the checkpoint or delta. Frames are
applied whole and in order, so the client replaces its token only after it has
applied the frame that carries it. Its token then always matches its grid
byte for byte, at a cost of about 40 bytes a frame. A client that cannot
vouch for its grid, for example after a failed apply or a local edit, drops
the token and attaches without one.

## Server choice

Send a delta only when all of these hold:
- `server`, `block` and `grid_id` match the block's live grid;
- `hist_epoch` and the dimensions are unchanged;
- `since.hist_seq` is no older than the oldest retained line;
- the estimated delta is smaller than half the full checkpoint.

Otherwise send a full v3 checkpoint as today. The fallback is always correct,
so every rule above can be conservative.

## Delta format (RTCD v1, proposed)

Header `RTCD\x01`, then:

1. The base it applies to. The client rejects a mismatch and asks for a full
   checkpoint.
2. All non-line state, in the v3 Grid and parser schema minus the screens and
   history: cursor, modes, tabs, margins, palette, titles, parser state. This
   is a few KB.
3. History: the new `hist_seq`, then the lines with seq in
   `(since.hist_seq, hist_seq]` in v3 line encoding. Lines evicted since the
   base follow from the new oldest seq.
4. Screens. If `screen_gen` is unchanged since the base, a bitmap of live rows
   whose `row_stamp > since.stamp`, then those rows; no saved screen exists or
   it is untouched. If `screen_gen` changed, by any number of switches, the live
   screen goes in full and so does the saved screen if one exists. The saved
   cursor and the other per-screen state are in section 2 either way. This
   covers alternate, then primary with an edit, then alternate again: the
   switches end on the same side, but `screen_gen` moved, so the replaced
   saved primary is sent.

Styles use one table across sections 3 and 4, as in v3. The decoder keeps
v3's budgets, applies into a copy, runs `validate_checkpoint`, and swaps only
on success, so a bad delta leaves the client's grid untouched.

## Expected sizes

- Typing at a shell prompt after a reconnect: a few rows plus the non-line
  state, about 2 to 5 KB against 0.64 MB.
- `cat` of 10,000 lines since the base: every history line is new, so the
  delta is about the size of the full checkpoint and the server sends the full
  one.

## Test plan

- Property test over generated streams split at k: decode the full
  checkpoint at k, apply the delta from k to the end, and require the result
  to equal a full checkpoint of the end state. Compare both RTSN and a
  re-encoded RTCP.
- Fallback cases: resize, reflow, scrollback clear, eviction past the base,
  a different grid id, an alternate-screen switch. Each must choose a full
  checkpoint or apply exactly.
- Screen round trip: base taken in the alternate screen, then leave, edit the
  primary and its saved cursor, and enter the alternate screen again. The
  delta must restore the new saved primary.
- Stale-token reconnect: attach, apply Output that pushes history, disconnect,
  reconnect with the client's current token. The result must hold every
  history line exactly once. Repeat with a disconnect mid-frame, where the
  client must still hold the previous token.
- Wrong lifetime: a token from a restarted server, or from another block with
  the same `grid_id` value, gets a full checkpoint.
- Hostile deltas: a wrong base, a bitmap longer than the row count, a history
  range past `hist_seq`, and budget overruns.

## Open questions

- C05 event sequence numbers against grid stamps. Proposal: the base stays the
  grid stamp, and C05 maps an event seq to the stamp current when that event
  was sent. That keeps one shared seqno for rows and the mux protocol, as
  roadmap row 7 intended.
- Kitty image store. Images can dwarf the cells. They need their own change
  stamp so a delta can skip unchanged images. Today they would go in full in
  section 2.
- Whether C13's predictive echo wants row deltas at a finer grain than whole
  rows. Decide when C13's spike runs.

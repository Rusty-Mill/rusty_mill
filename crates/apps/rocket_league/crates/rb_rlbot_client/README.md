# rb_rlbot_client

A blocking RLBot v5 client on `std::net`, on top of [`rb_rlbot_wire`](../rb_rlbot_wire).
Stage 3 of the native RLBot port (`../../rlbot/PLAN.md`). No dependencies beyond the wire crate.

- **`Connection`:** `connect`, `send` / `send_all` (one write), `recv`, `recv_timeout`,
  `handshake` (for named agents; an empty `agent_id` is `Error::EmptyAgentId`, since core sends it no team information, so an id-less match runner sends and reads for itself). Incoming bytes are buffered, so a timeout in the middle of a frame loses nothing,
  and a zero timeout is a true poll (a message already buffered or readable now, never a wait). The match runners (`rb_match_log`, `rb_run_tapes`) use this
  directly: start a match with `MatchConfiguration`, set state with `DesiredGameState`, end it
  with `StopCommand`.
- **`run_bots` / `run_hivemind`:** the handshake, `InitComplete`, then the packet loop for an
  `Agent` (`tick(&GamePacket, &mut Outbox)`). One agent per car for bots, one for everything for a
  hivemind or script. Pings are answered; a disconnect signal ends the run with `Ok`; a socket
  that closes mid-run is `Error::Closed`.
- **`Environment`:** `RLBOT_SERVER_ADDR` (or `_IP` / `_PORT`, default `127.0.0.1:23234`) and
  `RLBOT_AGENT_ID`, read the way RLBot passes them to a launched process.

One thread, no `mio`, no channels: a bot answers one packet at a time over one local socket, and
`rb_tape_bot` ticks at most a few cars. Per-car threads would be a change to `run_bots` alone if
a bot ever needs them.

Not carried over from the `rlbot` crate: comms, ball prediction and rendering callbacks, which
`rb_tape_bot` does not use (the wire crate reports them as `CoreMessage::Other`).

Tested against an in-process fake core that speaks the same frames (`tests/common`): messages
split into single bytes, two messages in one read, timeouts mid-frame, handshake order, ping
replies, garbage frames, and each way a connection can end. Whether the real core accepts
what this sends is stage 4's one-time check against the game.

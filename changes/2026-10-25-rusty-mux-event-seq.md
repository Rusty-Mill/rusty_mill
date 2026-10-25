---
category: Added
changelog: rusty_mux resumable event stream; numbered events and replay of missed events after a reconnect.
---
## 2026-10-25 - rusty_mux resumable event stream

- **Added:** feature bit `Features::EVENT_SEQ` and `rusty_mux_proto::Cursor` (a server run's random `epoch` plus the `seq` of the last event delivered). When both peers set the bit, every `Message::Event` carries its `seq`, numbered from 1 in each server run, and `Welcome` carries the cursor where the client's stream starts. A client that reconnects passes its last cursor in `Hello { resume }`. If the server still holds every event after it (the newest 4096), the Welcome echoes that cursor and the missed events follow, oldest first, before any live event: nothing is lost and nothing is repeated. Otherwise the Welcome gives the current position, and the client knows it must start afresh. The new `server/tests/events.rs` covers resuming without a gap or a repeat, a cursor from another run, and a client that does not set the bit.
- **Changed:** `Message::Hello` gains `resume`, `Message::Welcome` gains `cursor`, and `Message::Event` is now `{ seq, event }`. The new fields are optional on the wire (field ids 17 and 18), so peers that do not set the bit send and read the same bytes as before.
- **Known limitations:** history is kept in memory for one server run and is not configurable. A reconnecting client gets a new client id, and the leases and attachment it held are gone; it re-attaches as before.

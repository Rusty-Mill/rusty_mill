---
category: Added
changelog: `rb_rlbot_wire` models the client-to-core `StartCommand`, `DisconnectSignal` and `PingRequest`, and reports other schema members as `InterfaceMessage::Other(tag)`, so a stand-in core can read a whole client session.
---
## 2026-10-11 - rb_rlbot_wire: client messages a server needs

- **Added:** `InterfaceMessage::{StartCommand, DisconnectSignal, PingRequest, Other}` and `StartCommand`. Bytes verified against `rlbot_flat` 0.6.0 both ways (reference bytes decode here; our bytes decode there), pinned in `tests/golden.rs` and `tests/verified.rs`.
- **Changed:** a client message with a schema tag this crate does not model (render groups, comms, loadouts, ...) decodes to `Other(tag)` instead of `Error::UnknownUnion`, as `CoreMessage` already does; only tags outside the schema's union (0 or above 17) are errors. Exhaustive matches on `InterfaceMessage` need the new arms (none in this tree).
- **Known limitations:** the unmodelled members still carry no fields. Step 1 of `docs/design/RLBOT-PORT.md`; the rest of the server-side messages arrive with the server that needs them.

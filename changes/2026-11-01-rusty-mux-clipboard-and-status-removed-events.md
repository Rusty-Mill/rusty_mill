---
category: Added
changelog: rusty_mux clipboard and status-removed events: the server announces a block's program writing a selection (OSC 52) and each of its status records ending.
---
## 2026-11-01 - rusty_mux clipboard and status-removed events (C38)

- **Added:** `Event::ClipboardWritten` (kind 27) and `Event::StatusRemoved` (kind 28), behind the new `Features::MORE_PROGRAM_EVENTS` (bit 23); a client without it never receives them, live or on replay. A clipboard write names the selection, `clipboard` or `primary`, but not the text, which may be a secret. A status record that ends (cleared, ended by a prompt or a reset, or evicted by a newer one) is announced by its id, before the `BlockStatus` of the same change.
- **Added:** `rusty-mux-ctl events`, `watch` and `wait --event` know them as `clipboard-written block=… selection=…` and `status-removed block=… id=…`, and `on` rules can name them with those fields.
- **Fixed:** the server no longer keeps, for a block's lifetime, the text of the last clipboard write.
- **Known limitations:** a clipboard write whose data is not base64 of UTF-8 text raises no event. Several writes read at once are one event per selection. The `active window changed` and `config reloaded` events of C38 are not built.

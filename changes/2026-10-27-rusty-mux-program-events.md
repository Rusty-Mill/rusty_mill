---
category: Added
changelog: rusty_mux program events: the server announces a block's program exiting (with its exit code or signal), ringing the bell, showing progress (OSC 9;4) and asking for a notification (OSC 9/777), and answers the OSC 7501 `?` query.
---
## 2026-10-27 - rusty_mux program events (C29)

- **Added:** `Event::ProgramExited` (kind 16), `Event::Bell` (17), `Event::ProgressChanged` (18) and `Event::Notification` (19), behind the new `Features::PROGRAM_EVENTS` (bit 10); a client without it never receives them, live or on replay. A program that exits by itself raises `ProgramExited` with its exit code, or the signal that ended it, before its block's `BlockClosed`; closing a block raises none. A burst of bells read at once is one event. Progress is announced when it changes, as cleared, normal, error, indeterminate or paused with its percentage. A notification carries its title (empty when the program gave none) and body.
- **Added:** `rusty-mux-ctl events` and `watch` print them as `program-exited block=… code=… signal=…`, `bell`, `progress-changed block=… state=… percent=…` and `notification block=… title=… body=…`, and `on` rules can name them with those fields, such as `${code}`.
- **Added:** the server answers the Program Status Protocol query, `OSC 7501 ; ?`, with `OSC 7501 ; ? ST`: every state and kind of draft 0.4. `rusty_term_core` writes the answer for any host that sets `Grid::collect_program_status`, in order with its other replies, so a program that follows the query with DA1 reads the answer first.
- **Changed:** an attach to a block whose output has ended, while its program is reaped, is refused ("block … is closing") instead of being accepted with no checkpoint.
- **Fixed:** the server no longer keeps, for a block's lifetime, the bytes a terminal host would relay to its own terminal, such as each bell.
- **Known limitations:** on Windows, bell, progress and notification events need ConPTY to pass those sequences through (R1); exit codes do not. A program whose output a background job keeps open is reported only when the block's output ends.

---
category: Added
changelog: rusty_mux's server acks typed keys once their echo grace has been drained (`ECHO_ACK`, `EchoAck`) and announces who echoes on a block's terminal (`EchoMode`), the server half of predictive echo.
---
## 2026-10-30 - Echo acks and echo mode

- **Added:** feature bit `ECHO_ACK` (bit 20) with two server-to-client messages, `EchoAck { block, req }` and `EchoMode { block, mode }`, and the `Echo` mode (`Kernel`, `Hidden`, `Program`, `Unknown`) and the 50 ms `ECHO_GRACE` in `rusty_mux_proto`. An unknown mode value decodes as `Unknown`.
- **Added:** the server records each `Input` write from a client with `ECHO_ACK` and acks the newest key whose grace a drain of the PTY covered, after that drain's Output frames, or from a timer when nothing was printed. It reads the terminal's termios before forwarding each read, before each `Input` write and on its 500 ms poll, and sends `EchoMode` on a change, after each attach and after a takeover that keeps the attachment. A PTY write that finishes after another connection took the client over is not acked, since request numbers start afresh there. Windows sends `Unknown`.
- **Changed:** a Windows block's read is now safe to cancel: the output receiver is handed back when a read is dropped, where it used to be dropped with it.
- **Known limitations:** no client uses these messages yet (build PRs 2 and 3 of `docs/projects/mux/PREDICTIVE-ECHO.md`). On Windows an ack only means the reader thread had nothing queued. Under output that never pauses, no drain completes and acks wait.

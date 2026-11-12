---
category: Added
changelog: rusty_mux block owners can signal a block's program and release its terminal without the mux killing the program.
---
## 2026-11-12 - rusty_mux signal and release

- **Added:** C76 (card #750). This sits behind `Features::SIGNAL_RELEASE` (bit 44), which only a Linux server offers.
  - `SignalBlock` (type 0x28) sends hup, int, term, kill, stop or cont to the block's program group. No other signal can be sent.
  - `ReleaseBlock` (type 0x29) closes the block's terminal and waits for the program without ever killing it. A held block stays once the program exits, and any other block closes.
  - Only the owner of an owned block may send either; a block with no owner refuses both. Both are refused with `exited` once the program is gone.
- **Added:** `rusty-mux-ctl own` takes `signal` and `release` requests.

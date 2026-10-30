---
category: Fixed
changelog: Keep cancelled commands out of later own-console reads and resume input safely under backpressure.
---
## 2026-10-30 - Own-console input cancellation and resumption

- **Fixed:** the opt-in Windows own-console backend serves waiting cooked reads before later Ctrl+C input, so a cancelled command cannot enter the next read. Long pastes reach the waiting read before the record cap applies; echo pauses retain one bounded input slice and stop later admission.
- **Fixed:** close/disconnect resumes successor reads immediately, every output drain wakes potentially held echo, and reply scheduling accounts for actual bytes built. Count, the input event and peek agree while earlier reads own input; ANSI lookahead remains available to record reads with its key metadata.
- **Fixed:** an empty overlapping read no longer overwrites kept line rest, echo-off never prints a dangling surrogate, and wide Backspace erases both columns.
- **Fixed:** cancellation also discards a cooked read's prefix when an earlier raw reply exhausts the 1 MiB event budget, while preserving queued input owed to an earlier raw or record reader. Tab/Backspace and Tab/Escape retain the prior one-column erase instead of treating Tab as a zero-width printable character.
- **Known limitations:** wrapped-line erase and full tab-span layout still need the cursor-layout work tracked in #1256; Tab retains its prior one-column erase. Ctrl input remains ordered behind input paused on output backpressure. Default ConPTY behavior is unchanged.

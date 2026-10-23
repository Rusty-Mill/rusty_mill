---
category: Fixed
changelog: A BBP harness's slot holder now starts before the harness, so a moderator that dies mid-launch can no longer leave a harness running without its slot.
---
## 2026-10-22 - Slot holder starts before its harness

- **Fixed:** the slot holder started just after the harness, so a moderator that died between the two launches left a harness that had replaced its stdin running without its slot. The moderator now starts `bbp slot-hold` first, with the lock on its stdout, as the leader of a new process group, and starts the harness (and a confined harness's broker) in that group. The holder waits for its stdin to close, which happens when the moderator has started the harness or has died. It then keeps the slot until no other process is left in its group. If the moderator dies before starting the harness, the slot is freed.
- **Changed:** `bbp slot-hold` no longer takes `--group`. The group it watches is the one it leads, whose id is its own pid. That id cannot be reused while the holder or its group lives, which removes the earlier reused-group-id caveat.
- **Tests:** `a_slot_holder_covers_its_harness_from_the_start` frees an unused holder's slot, and keeps the slot for a harness that drops its stdin after the holder's stdin closes. With the holder ignoring its group, this test and the stdin crash test fail.

---
category: Fixed
changelog: `rb_rlbot_core` reports `BroadcastHeld` as soon as a started self-paced backend stops being ready, even when it has no packets to poll, and `BroadcastResumed` on the first real broadcast after.
---
## 2026-10-12 - rb_rlbot_core: notice a hold without a packet

- **Fixed:** the readiness check added in #629 ran only when a packet had been polled. A game that has just loaded a new map has no cars, so `LinkBackend` yields `Idle`: the bots went quiet with no `BroadcastHeld`, and the recovery could pass with neither event. Readiness is now looked at on every pass of the loop; `BroadcastResumed` stays tied to the first actual broadcast after a hold. Reviewer finding on #629.
- **Tests:** a stand-in backend that polls nothing while unready; the old code reports no events for it, the new code reports `Held` then `Resumed`. The stand-in now reports readiness as of its last poll, as a real game does (the earlier stand-in's readiness could change between a poll and the check after it, which hid this).
- **Known limitations:** the event does not say why the backend is not ready; a destroyed bot car or a demolition that destroys the actor would hold the broadcast until the next map (board cards b24, b22 follow-ups). Not run against a real Rocket League.

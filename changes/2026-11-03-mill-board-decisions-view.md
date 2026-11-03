---
category: Added
changelog: Mill Board Decisions view and /inbox page over the BBP decision inbox, with a waiting-decision badge on cards (#595).
---
## 2026-11-03 - Mill Board decision inbox: Decisions view

- **Added:** a Decisions view in the Mill Board web UI, also opened by `/inbox` (#595). It lists the open BBP escalations: irreversible first, each with its reason in plain words, the claim's confidence and budget, the strongest evidence, and the candidates side by side for a taste call. The view says "Nothing needs you." when the list is empty.
- **Added:** Approve, Reject, Choose and Refine intent buttons. Each one first shows BBP's dry run of the command and the events it would write, and acts only when you confirm.
- **Added:** a count on the header button, and a ⚑ badge on each card with decisions waiting. Both are computed on read and stored nowhere. The button is hidden for credentials without `admin` and on a board with no BBP host.
- **Added:** a failed inbox read (the BBP host erred or did not answer) keeps the decisions last read on screen, marked stale with the error and the time they were read, and flags the button with ⚠. Only a refusal (no admin credential, no host configured) hides the inbox. The browser check covers a failure and the recovery after it.
- **Known limitations:** the inbox is re-read every 30 seconds, not live, and there is no push notification (owner decision for v0).

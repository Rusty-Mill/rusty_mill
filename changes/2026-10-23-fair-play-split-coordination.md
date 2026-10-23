---
category: Fixed
changelog: Fair Play waits for pending card updates before splitting, avoiding a stale owner version during rapid deal-to-split actions.
---
## 2026-10-23 - Coordinate Fair Play deal and split

- **Fixed:** splitting waits for this tab's outstanding updates to the same card, then sends the acknowledged version. A failed update aborts the dependent split; external conflicts and other errors remain visible without retries or overwrites.
- **Verified:** deterministic delayed-owner-response browser coverage and store tests cover acknowledgement ordering, unrelated-card independence, failed owner updates, and external conflicts. The serial split flow explicitly checks the acknowledged owner before opening the dialog.
- **Known limitations:** the original queue artifact contains no transport trace; this reproduces and repairs a matching product race without claiming its exact historical cause is proven.

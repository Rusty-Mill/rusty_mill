---
category: Fixed
changelog: **`rusty_remind_me`** ADR-0028 now names `auto_capture`'s real input as the capture contract.
---
## 2026-10-26 - remind-me: ADR-0028 capture contract corrected

- **Fixed:** ADR-0028 described a `{session_id, dialog, summary?}` capture input that `remind_me_auto_capture` does not take, and its `minimal`-tier schema rule excluded `auto_capture` itself. The ADR now names the existing flat `AutoCaptureInput` (`conversation` and `summary` required) as the contract, allows its two required strings and the optional `metadata` object, and scopes the messages adapter to the client side. Docs only.

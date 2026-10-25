---
category: Fixed
changelog: "`rusty_tonic`'s cancel test no longer fails about half its runs on a race the server documents."
---
## 2026-10-25 - rusty_tonic cancel test accepts both documented outcomes

- **Fixed:** since #1009 a client cancel drops the handler, and only a handler mid-read "can see `CANCELLED` first" (`server` docs). `a_client_that_cancels_makes_the_handlers_read_fail_cancelled` assumed the read always won, so it timed out whenever the drop did: 32 of 60 local runs on `main`, and three nextest tries in a row in CI. It now passes when the handler either reads `CANCELLED` or is dropped, and still fails if neither happens. 0 of 100 local runs fail.

---
category: Added
changelog: Mill Board web UI asks for a token when it has none or one is refused, keeps it in the browser, and has Sign out.
---
## 2026-10-28 - Mill Board sign-in box

- **Added:** the Mill Board web UI shows a sign-in box when a call is refused with 401 (no token, or a wrong or
  rotated one), says which, keeps the entered token in this browser and reloads, so a token for another
  agent brings that agent's actor, role and projects. A 401 for a token since replaced by your own rotation is
  retried with the new one. Before, every call failed silently and the board looked empty, with no project list and settings refused.
- **Changed:** a `#token=` link still signs in, but the token is moved into the browser's storage and dropped from
  the address bar. "Sign out" forgets it. Rotating your own token in Settings stores the new one.
- **Known limitations:** the token sits in `localStorage`, readable by script on the page, as the `#token=` address
  already was. A board with no credentials never asks.

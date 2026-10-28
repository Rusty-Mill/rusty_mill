---
category: Fixed
changelog: Mill Board web UI says why when roles, identity or the project list fail to load, instead of showing an empty panel.
---
## 2026-10-28 - Mill Board shows why board metadata failed to load

- **Fixed:** the web UI loaded roles, who you are and the project list with their errors swallowed, so a refusal
  looked like a board with no projects. It now shows the operation and the server's reason, and carries on with the
  same fallback as before.
- **Known limitations:** the message is a short toast; it is not repeated on later polls.

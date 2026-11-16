---
category: Security
changelog: Shared mode: a guest can no longer rename another principal's connection, the server's account may lease a guest's open block, and a guest can ask for an open block's lease.
---
## 2026-11-16 - Shared mode guest limits

- **Security:** on a shared server (D232), `rename-client` of another principal's connection is refused unless the asker is the server's account or the target's own principal, as for disconnect.
- **Fixed:** the server's account can take the lease on an open block a guest created. `RequestLease` on an open block of a shared server now asks the clients of the principal that may grant it (the creator's) instead of answering "acquire its lease".
- **Known limitations:** no wire change. Whether attach may read another principal's screen is Nano's open question.

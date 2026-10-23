---
category: Fixed
changelog: Give crypto taint examples unique target names to avoid shared executable and PDB output collisions.
---
## 2026-10-23 - Unique crypto taint example outputs

- **Fixed:** Rename the auto-discovered `rusty_aead`, `rusty_pk` and `rusty_sha2` examples to `rusty_aead_taint`, `rusty_pk_taint` and `rusty_sha2_taint`. Their constant-time scripts use the corresponding build selectors and executable paths; the PK planted-branch symbol assertion and synthetic verdict fixtures follow `rusty_pk_taint::main`. All example behavior, report requirements, negative controls and disassembly budgets are preserved. The already unique `taint_aes` target is unchanged.
- **Known limitations:** The inherited shared `taint.exe`/`taint.pdb` names collided in Windows CI; the process holding the executable in the reported LNK1104 failure is unidentified. Windows validation must run in CI.

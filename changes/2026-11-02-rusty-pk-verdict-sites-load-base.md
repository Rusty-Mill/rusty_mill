---
category: Fixed
changelog: rusty_pk constant-time evidence derives valgrind's load base from a runtime anchor instead of assuming 0x108000.
---
## 2026-11-02 - rusty_pk verdict-site checker: derived load base

- **Fixed:** `scripts/verdict_sites.py` no longer hardcodes valgrind's PIE load base (0x108000), which
  failed every verdict site on the Ubuntu 26.04 runner image ("no instruction at ..."). The taint example
  now prints the runtime address of `rusty_pk_taint::main`; the checker subtracts that symbol's ELF
  address to get the base. A missing, repeated or conflicting anchor, a missing or duplicate symbol, or a
  negative or unaligned base fails before any report is classified. Symbol cross-checks, the
  branch-after-`ensure` rule, expected counts, planted controls and disassembly budgets are unchanged.
- **Known limitations:** the anchor is one symbol; a wrong base is still caught only by the existing
  per-report symbol-name cross-check.

---
category: Added
changelog: **`rusty_bbp`: role-name audit for v0.5.0.** Classifies every hardcoded role name in the spec, engine, tests and host as capability, orch rule or example, with the capability set that replaces them.
---
## 2026-10-13 - rusty_bbp: role-name audit for v0.5.0

- **Added:** `docs/bbp-role-audit.md`. Every occurrence of Planner, Coder, Tester or Reviewer in the v0.4.1 spec, `rusty_bbp` and `rusty_bbp_host`, classified (a) generic capability, (b) orch rule, (c) example, with the proposed replacement and the nine-field `Role` capability set.
- **Known limitations:** audit only; no spec or engine change. Four open decisions are listed at the end of the document.

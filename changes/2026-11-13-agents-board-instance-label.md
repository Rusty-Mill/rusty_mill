---
category: Changed
changelog: AGENTS.md asks agents sharing a Mill Board credential to act as `<credential>/<label>`.
---
## 2026-11-13 - AGENTS.md: label your Mill Board instance

- **Changed:** agents sharing one Mill Board credential or connector pass `actor` as `<credential>/<label>` (label = thread or session id) on every write, so claims and history tell instances apart. Uses the instance labels added in #1658.
- **Known limitations:** the label is self-declared attribution, not authority; claims made under the plain credential name still need the plain name.

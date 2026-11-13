---
category: Security
changelog: Update ammonia to 4.1.9 to fix a mutation XSS through embedded SVG and MathML in Hister's HTML sanitizer (RUSTSEC-2026-0533).
---
## 2026-11-13 - Fix Hister sanitizer SVG/MathML mutation XSS

- **Security:** Raise `rusty-hister-extractor`'s ammonia pin from `~4.1.8` to
  `~4.1.9` and the lockfile from 4.1.8 to 4.1.9, which fixes RUSTSEC-2026-0533
  (GHSA-934p-vp7m-gx3c). Hister's sanitizer allows both `math` and `svg`
  elements, the configuration the advisory affects. No code change.

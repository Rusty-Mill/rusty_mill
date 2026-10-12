---
category: Security
changelog: Update ammonia to 4.1.7 to fix CSS URL escaping in Hister's HTML sanitizer (RUSTSEC-2021-0157).
---
## 2026-10-12 - Fix Hister sanitizer CSS URL escaping

- **Security:** Update the workspace lockfile from ammonia 4.1.4 to exactly
  4.1.7, which fixes RUSTSEC-2021-0157. Hister permits style attributes and
  uses `filter_style_properties`, the configuration affected by the advisory.
- **Tests:** Cover quoted and unquoted CSS URL injection through both Hister
  HTML sanitizer policies, decoded attribute escaping, stable reparsing, and
  preservation of safe styles and the trusted-only layout exception.

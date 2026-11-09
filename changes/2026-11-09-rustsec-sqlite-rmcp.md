---
category: Security
changelog: Upgrade rmcp and bundled SQLite to resolve four RustSec advisories.
---
## 2026-11-09 - RustSec dependency remediation

- **Security:** Upgrade rmcp 3.1.4 to 3.9.0 for RUSTSEC-2026-0457, RUSTSEC-2026-0461 and RUSTSEC-2026-0462. It remains a development-only interop oracle; native MCP implementations and OAuth/network policy are unchanged.
- **Security:** Upgrade rusqlite to 0.41.1 and shared libsqlite3-sys to 0.39.1 (bundled SQLite 3.54.1) for RUSTSEC-2026-0459. A verified, temporary SQLx 0.9.0 manifest patch backports upstream's compatible SQLite range; see `vendor/README.md` for provenance and removal. Imported packaging carries canonical upstream license texts and non-executable file modes.
- **Changed:** SQLite consumers, including the standalone Rusty Keys desktop shell, now declare Rust 1.89, as required by rusqlite; the workspace toolchain is unchanged. The desktop lock is reconciled with its current path dependencies and patched SQLite. Regenerate unicode61 tables against the patched amalgamation (table contents unchanged).
- **Tests:** Exercise both SQLite drivers against the same WAL database and actual linked engine, including a rejected constraint violation; retain legacy and stateless MCP interop scenarios with explicit legacy protocol selection.
- **Known limitations:** Unrelated Quinn advisories remain outside this remediation. Existing advisory exceptions are unchanged; no alerts are silenced.

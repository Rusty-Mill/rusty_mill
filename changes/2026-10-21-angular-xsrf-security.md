---
category: Security
changelog: "**AGUI Angular:** development dependencies `@angular/common` and `@angular/platform-browser` were updated to 20.3.34, which enables XSRF protection for same-origin absolute URLs."
---
## 2026-10-21 - AGUI Angular XSRF dependency update

- **Security:** update the AGUI Angular package's `@angular/common` and `@angular/platform-browser` development dependencies from 20.3.33 to 20.3.34, which enables XSRF protection for same-origin absolute URLs.
- **Scope:** this changes development dependencies used to validate the Angular binding; it does not change the binding's consumer runtime dependency requirements.

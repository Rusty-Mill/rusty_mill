---
category: Fixed
changelog: GUI file drop and paste on a Windows mux server now types the uploaded file's path instead of refusing its `\\?\` prefix.
---
## 2026-11-28 - GUI types a Windows server's upload path

- **Fixed:** a Windows mux server reports an upload's place as a verbatim `\\?\C:\...` path; the GUI's quoting refused the `?` and showed "File sent, path not typed". The prefix is now dropped before quoting (`\\?\UNC\h\s` becomes `\\h\s`). Found testing card #1630; fix for card #1639.
- **Known limitations:** other characters a Windows shell would act on still stop the path from being typed.

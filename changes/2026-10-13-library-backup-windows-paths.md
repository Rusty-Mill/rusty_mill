---
category: Fixed
changelog: `backup-before-edit` now recognises secret-looking files on Windows paths (backslash separators) and joins a Windows home without a doubled separator.
---
## 2026-10-13 - `backup-before-edit`: Windows paths

- **Fixed:** the secret-name check only recognised `/`, so on Windows a path such as `C:\work\.env`, `C:\work\.ENV`, `C:\work\credentials.json` or `C:\Users\u\.ssh\id_ed25519` was not treated as a secret and was copied into the backup directory. The check now accepts `/` and `\` as separators, and the key/certificate pattern excludes both from the file-name part.
- **Fixed:** a home directory ending in a separator (`C:\Users\u\`) produced a doubled separator in the backup path; trailing separators are now trimmed.
- **Verified (run):** `claude plugin test` on the mod, 18 tests (up from 10): six Windows secret spellings, a Windows home with a trailing backslash (flat backup name, no backslash after the home prefix), and two controls that look like secrets but are not (`.environment`, `monkey.txt`) and must still be backed up. Run against the old code, 6 of the new tests fail (the five Windows secrets that were being copied and the doubled-separator check); with the fix all 18 pass. `claude plugin validate` passes.
- **Known limitations:** tested on a POSIX runner with a fake file system, not on a real Windows engine; the test strips the cwd prefix the engine adds to Windows-style paths. UNC paths, alternate data streams and 8.3 short names are not specially handled. The secret-name list is still a heuristic. CI does not run these mod tests.

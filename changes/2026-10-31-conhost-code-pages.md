---
category: Added
changelog: rusty_conhost accepts any installed code page, so `chcp 850` or `chcp 932` changes how `A` APIs read and write text (#1222 PR 9).
---
## 2026-10-31 - Own console host supports code pages

- **Added:** our own console host keeps conhost's two code pages. `SetConsoleCP` and `SetConsoleOutputCP` take any code page Windows has installed, and refuse others with `E_INVALIDARG` as conhost does, where before only 65001 was accepted. WriteConsoleA and raw writes decode with the output code page, keeping a DBCS lead byte that ends one write for the next; ANSI reads, ANSI input records and the `A` title calls use the input code page. `chcp 850` in cmd now works on the own host.
- **Added:** `platform_windows::condrv::{is_valid_code_page, decode, encode}` (`IsValidCodePage`, `MultiByteToWideChar`, `WideCharToMultiByte`), public API additions under design §11.2, with the `Win32_Globalization` windows-sys feature in platform-windows' own list. No new dependency and no lockfile change.
- **Known limitations:** both code pages still start at 65001, where conhost starts at the OEM code page (divergence V4). GetConsoleLangId still always fails, since it would need the system's ANSI code page (V13). A GB 18030 four-byte character split across two WriteConsoleA calls is not reassembled; conhost only keeps two-byte lead bytes either.

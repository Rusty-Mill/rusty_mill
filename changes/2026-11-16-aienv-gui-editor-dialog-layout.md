---
category: Fixed
changelog: The Ai Env Manager editor dialog no longer collapses in the desktop window.
---
## 2026-11-16 - Ai Env Manager: editor dialog layout in WebKitGTK

- **Fixed:** in the real Tauri window (WebKitGTK) the editor dialog's body collapsed to about 25px and the footer drew over the file list and viewer. The body now sizes to its content and scrolls inside the dialog. Chromium, which the Playwright scripts use, did not show it.
- **Known limitations:** guarded by a stylesheet test only; the window check was done by hand under Xvfb.

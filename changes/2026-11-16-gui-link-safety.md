---
category: Security
changelog: The GUI asks before opening links to untrusted sites or other apps, refuses unknown schemes, and no longer lets a `&` in a URL run a second command on Windows.
---
## 2026-11-16 - GUI link safety (C63)

- **Security:** Ctrl+click and the Ctrl+Shift+O list used to open http, https, ftp, file and mailto links with no prompt. Now only http/https to a trusted origin opens at once. Other http/https links ask (Cancel, Open once, Always trust). mailto, ftp and file always ask, with no Always trust. Any other scheme, a URL with `user@` or no host, and a URL that is empty, over 2048 bytes or holds whitespace or control characters is refused with a "Link not opened" notice.
- **Fixed:** on Windows the opener was `cmd /C start "" <url>`, where an `&` in a URL could run a second command. It is now `rundll32 url.dll,FileProtocolHandler`, with no shell.
- **Added:** trusted origins (scheme, host, port) are saved in `trusted_origins.toml` beside `config.toml`. The prompt is the take-over confirm panel, now general, with Cancel focused and the whole URL shown.
- **Added:** the prompt always stays inside the window (narrowed, moved from the edges) and a URL too long to show scrolls with the buttons in view. A failed save of the trusted sites no longer trusts the site for the rest of the window.
- **Known limitations:** an OSC 8 link's visible text is not shown beside its target yet (Q73). There is no Settings row to review or remove trusted origins yet (Q72): edit the file.

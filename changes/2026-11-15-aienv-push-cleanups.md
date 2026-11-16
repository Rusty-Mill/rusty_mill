---
category: Fixed
changelog: Re-marking a pushed copy follows no links, as the editor's save does, and a crash-left staging file no longer blocks it.
---
## 2026-11-15 - Push-back cleanups

- **Fixed:** after a push, re-marking the copy (`remark_if_equal`) now checks that the copy folder and its marker are a real folder and a regular file (a link is refused, as in the save path), and writes the staging file with create-new so a planted link at that path is never written through.
- **Fixed:** a staging file left by a crash no longer blocks re-marking: a leftover regular file is removed first (a link there is still refused).
- **Docs:** D42 names `push_back`, but the code has `push_preview` and `push_apply`; a new decisions row (D44) records the correction, as the file is append-only.

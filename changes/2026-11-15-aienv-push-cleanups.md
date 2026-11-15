---
category: Fixed
changelog: Re-marking a pushed copy follows no links, as the editor's save does, and the push decision text names the real functions.
---
## 2026-11-15 - Push-back cleanups

- **Fixed:** after a push, re-marking the copy (`remark_if_equal`) now checks that the copy folder and its marker are a real folder and a regular file (a link is refused, as in the save path), and writes the staging file with create-new so a planted link at that path is never written through.
- **Fixed:** D42 in the Ai Env Manager decisions named `push_back`; it now names `push_preview` and `push_apply`, the functions in the code.

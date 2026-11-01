---
category: Fixed
changelog: bbp's build identity and the bbp pairs clean-tree check see untracked files whatever status.showUntrackedFiles says, and a failed git status is no longer a clean tree.
---
## 2026-11-01 - bbp clean-tree checks ignore status.showUntrackedFiles

- **Fixed:** `rusty_bbp_host`'s `build.rs` (`BBP_BUILD_SHA`) and the `bbp pairs` pin ran plain `git status --porcelain`, which honours a user's `status.showUntrackedFiles=no`. With that setting an untracked source file was invisible, so a dirty build reported a clean commit and a dirty checkout passed the pin. Both now pass `--untracked-files=normal` (#1328).
- **Fixed:** the `bbp pairs` pin ignored `git status`'s exit status, so a status git could not produce (a corrupt index) printed nothing and read as clean. It now refuses.
- **Known limitations:** files hidden by a user's global excludes file (`core.excludesFile`) or `.git/info/exclude` are still not seen; both checks treat ignored files as clean, as before.

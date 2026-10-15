---
category: Fixed
changelog: The super-board merge gate no longer passes --delete-branch to gh pr merge, which GitHub rejects when the merge queue is enabled.
---
## 2026-10-15 - Merge gate works with the merge queue

- **Fixed:** `super-board-merge-gate.sh` ran `gh pr merge --delete-branch`, which GitHub refuses with "Cannot use -d/--delete-branch when merge queue enabled". The gate then raised a GitHub halt with the PR still unmerged. It now omits the flag.
- **Known limitations:** head branches are no longer deleted by the gate. Enable the repository's "Automatically delete head branches" setting to keep branch cleanup.

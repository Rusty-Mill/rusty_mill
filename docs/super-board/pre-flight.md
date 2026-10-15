# Super-board pre-flight

Needed before `/super-board run` (policy: [autonomy-policy.md](autonomy-policy.md)).

- [ ] `gh auth status` shows scopes `repo`, `project`, `workflow` for the account that runs the board.
- [ ] The narrow permission rules from [proposed-settings.md](proposed-settings.md) are in your own
  `.claude/settings.local.json` (otherwise the agent asks before each command).
- [ ] Rust toolchain 1.98.1 is installed (`rust-toolchain.toml`), Node is present, and on Windows
  `PYTHONUTF8=1` is set for the Python scripts.
- [ ] `/super-board lint` passes for every Ready card.
- [ ] You verified, with one card, that the merge gate enqueues on the queue-protected `main`
  (rollout step 1 in the policy). Nothing else has tested that against real GitHub.

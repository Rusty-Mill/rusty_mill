---
category: Added
changelog: `rusty_orch lens run --alien` runs the alien lens's hunts after the ordinary passes and saves their rows (#708).
---
## 2026-11-05 - Alien lens hunts in `lens run --alien`

- **Added:** `rusty_orch lens run --alien`, off by default (#708 slice 4c, D99). After the ordinary passes end quiet or at `--max-passes`, the config's `alien` lens hunts each eligible claim whose current candidate it has not hunted yet, runs the probes through the sandboxed probe runner, and saves the hunt and probe rows that `lens alien report` reads. It reads through its own `bbp mcp` session and posts nothing.
- **Added:** a claim is eligible when it is irreversible (the trigger until #596's value score exists, D75), every review lens that follows it has recorded its blind pass and any owed look-harder in the claim's epoch, and its candidate's diffs touch a `.rs` file. A claim that can't be hunted before any model call (the role may not read the candidate, the diffs are over 128 KiB, the base is not a full commit id, no provider is cleared for its class, or its probes can't be set up) gets one stderr line and no row, so the next run tries it again. A ceiling ends the pass.
- **Added:** the lens config takes an optional `"alien": {"id", "prompt", "providers"}`. Its id is its own BBP role, its providers must be CLI ones (its calls use read-only tools), and it follows every domain.
- **Changed:** `--alien` checks the toolchain, the `--repo` repository and the `alien` entry, and removes leftover probe trees, before any pass, so a missing piece costs nothing.
- **Known limitations:** the "already hunted" memory is keyed by task id, like the meter, so two BBP directories with the same task id sharing one `--state` share it. Diffs over 128 KiB are not hunted (Q18). The alien pass does not run after a halted round or a task ceiling.

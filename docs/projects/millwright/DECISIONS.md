# Millwright: decisions

Only what Nano has stated. Each row gives the date, the reason, and where he said it
(a remind-me id). Recommendations that are not yet answered stay in
[QUESTIONS.md](QUESTIONS.md). Agents do not add defaults here.

| Key | Decision | Why | Date | Source |
| --- | --- | --- | --- | --- |
| name | The programme is called Millwright. It is a label, not a product or a repository. | A millwright builds and maintains the mill's machinery. | 2026-11-11 | `mem_a6e15090b0104ad2ab9c71eba524d1ad` |
| board-committed | Mill Board holds committed work only. Ideas live in remind-me. | A backlog that is 95% ideas stops steering agents. | 2026-11-10 | `mem_798c3623c9504740bd949b4a691cb4e7` |
| board-store | Mill Board owns its own SQLite store. rusty_tick is archived, not deleted. Supersedes the 2026-10-26 one-store decision. | rusty_tick is unused, so a view over it paid integration cost for nothing. | 2026-11-10 | `mem_c6d02995042b4dedbb6a18e84730853f` |
| third-party-db | "No third-party database" is a preference, not a constraint. | Open to what makes sense for remind-me. | 2026-11-10 | `mem_9b5d6e8bfb6e4d9eb96ea68c9d0e1be7` |
| pure-rust | Pure Rust is a preference he would trade for not maintaining storage code. A C-backed store is acceptable for remind-me. | remind-me already compiles bundled SQLite. | 2026-11-10 | `mem_24c63c828a7f41dda641501a4f587197` |
| experiments | Ledger, Conflux and Swarm OS were deliberate experiments: a different model designing memory or coordination without being told about the existing systems. | To see whether another model would produce a different design. | 2026-11-10 | `mem_07c124e9f8f74ec4a742f6496b9ca070`, `mem_c3d5bbd1aa7d445688e01f1a05dbec75` |
| M-merge | Gates should not be human approvals. The 2026-10-27 rule that agents never queue or merge was wrong: agents may queue, merge and deploy once machine-verified evidence checks pass. **Not in effect** until those checks and per-agent credentials exist; until then AGENTS.md stands as written. | Nano mostly rubber-stamps approvals and does not want to be the bottleneck. | 2026-11-10 | `mem_3dc7f50871404442837cc6368f522fb3` |
| duetius | Duet, called Duetius, is the Claude-based agent, alongside Dottius (ChatGPT-based). | — | 2026-11-10 | `mem_1c5dc78d5a114ddd99f4d2e197c33604` |
| lel-freeze | Let Em Loose is under a design freeze until its first run. Answers L9. Recorded as Let Em Loose D127. | Run before designing more. | 2026-11-12 | `mem_1bae9f1358544fc3a517998b983b896a` |

## Not decided

Everything in QUESTIONS.md. In particular, the ask-first list (migrations, deletions,
dependency and toolchain bumps, public API changes) is assumed to stand under M-merge;
question M5 asks Nano to confirm it.

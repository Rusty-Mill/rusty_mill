---
category: Changed
changelog: **`rusty_bbp_host`: roster in, role names out.** `bbp open` takes `--roster FILE`; launchers are keyed by role id and carry their own `builtin_tools` policy; the four-role enum code and the hard-coded "only the Coder keeps built-in tools" rule are gone. The first proving run's record is archived as log schema 1.
---
## 2026-10-13 - rusty_bbp_host: roster in, role names out

- **Changed:** `admin::open_task` takes the roster (`Vec<Role>`) and `bbp open --roster FILE` reads it (`admin::read_roster`); `admin::assign` and the human `ask` take any role id, and the core rejects an id outside the roster (`UnknownRole`). `parse_role` and `role_name` are removed. `Launchers` is a map from role id to `Launcher`; the four named fields are gone. The `post` tool text no longer names a role.
- **Changed (rule moved to config):** which role keeps its built-in tools was a test assertion about the Coder. It is now each launcher's `builtin_tools` field (default `false`); `docs/proving/agents.json` sets it for the `coder`, and the config test checks that every launcher's `--tools ""` argument agrees with its own declaration.
- **Added:** `docs/proving/roster.json`, the four-role example (spec Appendix C.1), used by the host tests; tests for launchers keyed by an arbitrary role id and for a roster without a finalizer refused at open and corrected afterwards.
- **Changed (docs):** `docs/proving/2026-10-09-slug-1.md` moves to `docs/proving/archive/` with a schema-1 banner and an archive README; `bbp open` in the runbook and README gains `--roster`. The first run's raw store lived outside this repository, so no log file moves.
- **Known limitations:** a schema-1 data directory is refused by `FsStore::open` until its logs are moved aside.

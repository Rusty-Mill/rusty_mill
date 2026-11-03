---
category: Added
changelog: rusty_bbp_host refuses claims-mode candidates that touch protected test paths (#597 slice 3); test changes go through an accepted `tests` claim.
---
## 2026-11-03 - Protected test paths (Let Em Loose #597, slice 3)

- **Added:** the claims-mode profile set may name `protected` globs. The host's Agent API reads each candidate diff's paths as `git apply` does, including the old side of a rename, and refuses `protected_path`, naming each path, when a diff adds, modifies or deletes one. Nothing is stored or debited (owner answer G2: refuse, don't restore).
- **Added:** a claim in the `tests` domain may carry a candidate that touches only protected paths. Once that claim is accepted, other candidates may list its diffs. The core accepts a `tests` claim only with support from a principal who authored neither its candidate nor its diffs.
- **Changed (review):** `protected` requires a full-commit `baseline` in the profile set, and every candidate on the task must build on it, so an older base with weaker tests cannot slip past a diff-only check. Protected globs outside the matcher's grammar (`[ab]`, braces, a leading `/` or `./`, `..`) are refused at open (`bad_claims_config(protected)`) rather than silently protecting nothing.
- **Fixed (review):** a committed candidate op replays its stored answer even after the exemption it used is gone, and the rule is re-judged on the reloaded state when a dispatch hits a revision conflict. `Driver::dispatch_guarded` (new in `rusty_bbp`) carries a host rule through that retry.
- **Fixed (review):** a candidate naming a diff that is not stored when the host reads its paths is refused `ref_unresolved` on every attempt, so a diff another session stores at that id after the read cannot pass a guard built without its paths.
- **Fixed (review):** protected globs with a control character or a Windows drive prefix (`C:/tests/**`) are refused at open; they looked absolute or unprintable and would protect nothing. `!` stays refused rather than literal. `test_paths` validation is unchanged here.
- **Fixed (review):** the old side of every file section counts, not only `rename from` lines: a `diff --git a/tests/x b/src/x` with plain `---`/`+++` headers is a rename to git, and `--numstat` names only the new side. A section whose source cannot be read from the patch is refused.
- **Added:** traces CL-78 (core), CL-79 (host) and CL-94 (guarded dispatch).
- **Known limitations:** with `protected` set the interim `tests_unendorsed` rule never fires; its fields stay for tasks opened without `protected` and are removed in a later, separate change. A path git quotes in a rename or `---` header fails closed, and a hunk line that looks like a `---`/`+++` pair adds a path (over-refusal, never a miss).

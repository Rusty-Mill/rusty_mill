# #1381 Gateway skill libraries: delta to 726 §8

Status: §8 of `726-sandbox-design.md` is approved (D180); its §10 makes that the design for #1381.
This note lists only what §8 left open. Implementation defaults are D230.

## What §8 already settles

- Skill packs are served by a gateway-local source as MCP **resources** `skill://<pack>/<path>`.
  No MCP prompts and no resource templates (the card's "prompts/resources" predates §8).
- A token sees and reads only the packs its verified lens policy lists in `[skills].packs`.
  Unknown lens, digest mismatch or no token: no skills. Upstream targets' prompts and resources
  stay denied on lens-policy routes, as `lens_policy.rs` (#1378) already does.
- Each listing carries the pack's digest.

## Slices

1. **Core (this PR).** `agentgateway-mcp/src/skills.rs`: a checked, immutable catalog over
   `(pack, path, bytes)` triples, pack digests, filtered list and read. `[skills]` in a lens
   policy is validated strictly (`packs` only) and exposed as `LensPolicies::skill_packs`.
   Not wired: nothing is served yet, and no config changes.
2. **Serving.** Federation advertises `resources` when a catalog is loaded, lists and reads
   `skill://` resources through `skill_packs`, puts the digest in each resource's `_meta`
   (`lel.pack_digest`), logs once a pack a policy names but the catalog lacks. Test: a lens token
   lists only its allowed skills (the card's done line).
3. **Loader and config.** The fs adapter that reads the catalog, and the config key that points at it.
   Waits on A and B.

## Open, asked of Nano on 2026-11-28

- **A. What is a pack on disk?** §8 assumed `knowledge/<pack>/`, but Factory `knowledge/` is flat.
  (a) a `domains.json` token (#723, D207); its files are the knowledge and standards the map lists;
  (b) one flat `knowledge/<stem>.md` file; (c) restructure Factory into folders. Recommended: (a).
  Slices 1 and 2 work for any answer: they take `(pack, path, bytes)`.
- **B. Where the gateway finds the catalog.** A new config field, so ask-first (§11, D223).
  (a) an optional key inside `lelPolicy` (a Factory checkout root); (b) a new route field
  `skills: {dir}`. Recommended: (a).

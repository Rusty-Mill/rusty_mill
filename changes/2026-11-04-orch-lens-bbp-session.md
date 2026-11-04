---
category: Added
changelog: "`rusty_orch::lens::bbp` reaches a claims task through one `bbp mcp` session per lens (D54); `orch-lens` builds a `Card` from BBP's claim view (`Card::from_view`)."
---
## 2026-11-04 - rusty_orch: the lens runner's BBP session

- **Added:** `rusty_orch::lens::bbp` (Let Em Loose #594, D54). `Child` starts `bbp mcp` as one lens's session (`BBP_SESSION=yes`, `BBP_PRINCIPAL=<lens>`), removes inherited BBP, API and push credentials, and bounds every request at 30 seconds in total (replies to other ids do not extend it). `Session` keeps the lens's view: new messages from paged `read` calls (claim bodies and evidence notes), and the cards from `claims_delta` since the last revision. It is the lens's `Board`: each post refs the task brief and has its own op id (`lens-<run id>-<n>`), so identical posts from independent jobs, epochs or later runs are each posted; a call lost in transport is resent once with the same op, which BBP replays. A session refuses to post for another lens.
- **Added:** `orch_lens::card::Card::from_view` builds the runner's card from a `ClaimView`, the claim's body and its evidence notes. Its accept line follows the task's claims config, and its data class is the highest of its domains' classes.
- `read` is charged per call (addendum R11), so a session only reads messages it has not seen. Adding notes to `claim_card` would avoid those reads, but it is a `rusty_bbp` API change and is open question Q12.
- Docs: D54 and Q12 in the Let Em Loose handoff. Nothing calls the session yet; the `lens run` command comes next. No new dependency.

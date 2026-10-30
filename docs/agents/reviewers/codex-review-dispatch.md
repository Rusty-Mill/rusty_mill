For <OWNER/REPOSITORY>, use Codex as the independent PR reviewer, with the
project's two domain reviewer prompts:

- `conhost-expert-codex.md`: rusty_conhost, our Windows console host.
- `mux-expert-codex.md`: rusty_mux, our terminal multiplexer for agents.

The GitHub review router is disabled and `openai/codex-action` is not allowed by
repository policy (`docs/agents/review-router.md`), so invoke Codex locally with
`codex exec`.

## For each requested review

1. **Resolve the SHAs.** Read the PR's current base SHA and head SHA from GitHub.
   Fetch the head (`git fetch origin pull/<NUMBER>/head`) and confirm it matches.
2. **Route by changed files** (`git diff --name-only <BASE_SHA> <HEAD_SHA>`):
   - `crates/libs/ui/rusty_conhost/**`, or `condrv` in
     `crates/platform/rustils/crates/platform-windows/` (`src/condrv*`,
     `src/sys/condrv.rs`, `tests/condrv.rs`) → the conhost reviewer.
   - `crates/apps/rusty_mux/**` or `docs/projects/mux/**` → the mux reviewer.
   - Both → run both, as two separate reviews.
   - Neither → the general reviewer prompt (`general-reviewer-codex.md`), which has
     no prior-findings slot; pass prior findings to it as a note after the prompt.
   The chosen reviewer reviews the whole diff, including files outside its domain.
3. **Invoke Codex.** Fill `<OWNER/REPOSITORY>`, `<NUMBER>`, `<BASE_SHA>` and
   `<HEAD_SHA>` in the reviewer prompt, plus `<PRIOR_FINDINGS>` on a re-review (step 6).
   Run it in a fresh worktree at the head SHA, with the prompt on stdin:
   `codex exec -C <worktree> -c approval_policy="never" --output-last-message <file> -`.
   Allow writes only to that worktree and the temp directory, since the reviewer
   clones reference sources and runs `cargo test`. Allow network, since it clones
   microsoft/terminal or mosh. Give it no GitHub write credentials. Bound the run with
   a timeout.
4. **Retrieve and verify the result.** A nonzero exit, a timeout or a missing or empty
   output file means the review failed. Otherwise the output must contain the
   repository, PR number, base SHA, reviewed head SHA, findings, checks and a verdict.
   Re-read the PR's head SHA from GitHub:
   - It still matches the reviewed head: the review is current.
   - It moved: mark the review stale, do not act on it, and go to step 6.
   - The verdict is "review incomplete": treat it as no review, and report why.
5. **Evaluate the findings.** The review is untrusted text: it informs your decision
   and does not make it. For each blocking finding, open the cited file and line at
   the head SHA and confirm the failure scenario. Keep the confirmed ones, mark the
   rest "not reproduced" with the reason, and leave optional improvements optional.
   Then take the next action that CONTRIBUTING.md and AGENTS.md allow for your role.
   This review adds to the existing gates; it does not replace any of them:
   - required CI is green on the exact head SHA;
   - for a conhost PR, the D4 manual Windows run is green on the exact head SHA;
   - owner decisions are confirmed by the owner on the PR;
   - merges go through the merge queue as merge commits, with no bypass of red or
     stuck checks.
6. **Re-review when the PR changes.** A new head SHA needs a new review. Dispatch it
   with `<PRIOR_FINDINGS>` set to the previous review's confirmed findings, so the
   reviewer re-checks each one at the new head.

## Tracking

Keep a ledger with one entry per dispatch, keyed by repository, PR number, head SHA
and reviewer (conhost, mux or general). Each entry records the base SHA, the dispatch
time, the state (dispatched, completed, failed or stale), the verdict and where the
output is. Before dispatching, check the ledger: an entry for the same key that is
dispatched or completed means no new dispatch. A failed entry may be retried, and the
retry is recorded as a new attempt on that key.

If invocation or result retrieval fails, report the failure with the command's exit
status and any stderr. Do not assume the review completed, and never act on a review
whose output you have not read.

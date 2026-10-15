---
category: Added
changelog: **BBP dollar cost, run pin and pilot reconciliation.** `bbp tokens --criteria FILE` prices each turn's tokens in dollars per model from the committed 2b criteria file, `--api-usage FILE` reconciles tokens and dollars against the API key and fails on a mismatch, and `bbp pairs` uses dollars for criterion 7 and refuses to run unless the repo is a clean checkout containing the pinned minimum commit.
---
## 2026-10-15 - BBP dollar cost, run pin, pilot reconciliation

- **Added:** `docs/bbp/2b-criteria.json` (a proposal, marked as such): `prices_as_of`, dollars per million tokens per model id (input, output, cache creation, cache read), and `min_commit`. `tokens::read_criteria` returns it with its sha256 (prices finite and positive, a date and at least one model required).
- **Dollars, per model:** `bbp tokens --criteria FILE` prices each assistant message by its own model (a turn may use a helper model), once per message id, and prints raw parts, the plain sum and `usd` side by side with the file's sha256 and date. **A model with no price is an error, never zero.**
- **Reconciliation:** `--api-usage FILE` (the four counts, and `cost_usd` when the report has it) is compared with the capture: a part or the dollars more than 2% off, or any turn with no harness output, is an error; the pilot's stop.
- **Pin:** `bbp pairs` reads `repo` and the criteria's `min_commit` and refuses unless the checkout is clean and its HEAD contains `min_commit` (the merge of the verdict-binding fix); a criteria file with no `min_commit` refuses. The output records `git_sha` and `criteria_sha256`. It pins the checkout the manifest names; it cannot prove the `bbp` binary was built from it.
- **Changed:** the pairs manifest needs `criteria` and `repo` paths; criterion 7 uses dollars.
- **Not verified:** prices are from the Claude API documentation (2026-10-06) and need the owner's confirmation; the stream-json line shapes are as I know them, not from a captured log; the reconciliation has hand-made numbers only.

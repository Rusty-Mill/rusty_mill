---
category: Added
changelog: **BBP cost-weighted tokens and pilot reconciliation.** `bbp tokens --criteria FILE` prices the four token counts with the weights in the committed 2b criteria file, `--api-usage FILE` reconciles the capture against the API key's usage and fails on a mismatch, and `bbp pairs` uses the weighted figure for criterion 7.
---
## 2026-10-15 - BBP cost-weighted tokens

- **Added:** `docs/bbp/2b-criteria.json` (model and weights; **a proposal, marked as such, not approved**). `tokens::read_criteria` returns it with its sha256 (positive finite weights only; unknown keys ignored). `bbp tokens --criteria FILE` prints the raw parts, the plain sum (`sum`, `total`) and `weighted` side by side, plus the file's sha256. `bbp tokens --api-usage FILE` reconciles the four counts against the key's usage (FILE: `input_tokens`, `output_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`): a part more than 2% off, or any turn with no harness output, is an error, which is the pilot's stop.
- **Changed:** `bbp pairs` prices each run with the manifest's `criteria` file (the manifest gains a required `criteria` path) and criterion 7 uses the weighted figure; the output carries `criteria_sha256`.
- **Why:** raw sums let cache reads dominate a cost they barely carry. The weights are per model, taken from the documented prices (cached 2026-10-06; to be confirmed by the owner), and frozen by being committed with the criteria.
- **Not done:** the 2% tolerance and the weights are proposals; neither is approved. Not checked against a live usage report.

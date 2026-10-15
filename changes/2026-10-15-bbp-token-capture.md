---
category: Added
changelog: **BBP token capture.** `bbp tokens` reports model tokens per turn (revoked turns marked) from the harness output the moderator already keeps, so the 2b run can report tokens per accepted outcome and enforce its token cap.
---
## 2026-10-15 - BBP token capture

- **Added:** `tokens::of` and `bbp tokens --dir D --task T`: for every granted turn, the tokens its harness printed (`input`, `output`, `cache_creation_input`, `cache_read_input`), whether the turn was revoked, and whether the harness printed a final `result` line. Totals cover every turn, revoked ones included, plus the revoked share. A turn with no output file is counted in `without_output` (cost unknown, not zero).
- **How:** it reads `agents/<task>/turn-N.log`, which the moderator already writes from the harness's stdout; with `claude -p ... --output-format stream-json` (as `docs/proving/agents.json` does) the final `result` line carries the usage. A harness that was killed has no `result`; its tokens are the sum of the assistant messages it printed, once per message id, a lower bound marked `complete: false`. No launcher wrapper and no moderator change.
- **Total:** `usage.total()` sums all four counts. Cache reads are large; the parts are in the output so the owner can weigh them. The 2b note says "input and output"; this reports the total and the parts, and the note is to say which one the cap and criterion 7 use.
- **Not verified against a live run:** the line shapes follow the Claude Code stream-json format as I know it; the unit tests use that shape, not a captured log. The pilot run is the check.

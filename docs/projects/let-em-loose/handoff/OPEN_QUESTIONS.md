# Open questions: let-em-loose

Claude Code and the `let-em-loose-expert`: add an item here instead of stopping when a question isn't blocking.
Format: `- [ ] Q<n> (<date>, blocking? y/n): <question> — interim choice: <what you did>`
Resolved items are checked and point to the D# or QA entry that answers them. Only the expert ticks items.

## Not sourced (no record answers them)

- [ ] Q1 (2026-11-03, n): What did the 2026-11-03 design chat decide beyond Nano's handoff prompt? The share link renders without the conversation, so its content couldn't be read. — interim choice: rely on the repo docs, remind-me and cards; nothing is attributed to the chat.
- [ ] Q2 (2026-11-03, n): Is the #596 pre-registered erosion criterion ("treatment arm's erosion delta no worse than baseline on 4 of 5 runs", erosion-gate-design §6) adopted? Nano's §11 answers and D41–D45 don't mention it. — interim choice: not part of the #596 rule until Nano confirms.
- [ ] Q3 (2026-11-03, n): Are the #599 changes in 599-gaps-design.md §6 approved? Nano answered Q1–Q4 but no comment approves §6. — interim choice: treat §6 as needing approval (Ask first: schema/API).
- [ ] Q12 (2026-11-04, n): Should `claim_card` carry each evidence item's note (and strength), so the #594 lens runner needn't spend charged `read` calls to fill them? Additive `rusty_bbp` public API change. — interim choice: incremental cached `read(after)` per lens session (D54).

## Waiting on Nano (sourced as open)

- [ ] Q4 (2026-11-02, n): Golden set sign-off for #600 and Phase B. (../QUESTIONS.md Q28)
- [ ] Q5 (2026-11-02, y for #598 build): Approval of the #598 schema and API items in 598-gaps-design.md §6 (`task_paused`, `intent_revised`, data classes). (card #598, 22:07Z)
- [ ] Q6 (2026-11-02, n): Do tool-using agents join the board over MCP? D14 is still Proposed.
- [ ] Q7 (2026-11-02, y for #600 build): The #600 builder-session design isn't posted or approved. (card #600)
- [ ] Q8 (2026-11-03, n): #708 alien review lens: trigger policy, per-language defect lists, scoring, and how reproduced findings enter BBP. Design not approved. (card #708 open questions 1–4)
- [x] Q10 (2026-11-03): What happens when report-only reaches its time box before Nano has set limits? → Settled 2026-11-03: new affected tasks pause until thresholds are approved; open tasks keep their settings and frozen profile digests. Source: Nano, recorded by dottiusmaximus in [#1384 comment 6614620497](https://github.com/Rusty-Mill/rusty_mill/pull/1384#issuecomment-6614620497). Corrected D42 text: #1449 (merged). See QA.md § Erosion gate.
- [x] Q11 (2026-11-03): The D44 cleanup rule when a metric is already 0. → Settled 2026-11-03: a cleanup passes when at least one metric improves and neither worsens; it is skipped, with the reason recorded, when both baseline metrics are 0. Source: Nano, recorded by dottiusmaximus in [#1384 comment 6614620497](https://github.com/Rusty-Mill/rusty_mill/pull/1384#issuecomment-6614620497). Corrected D44 text: #1449 (merged). See QA.md § Erosion gate.
- [ ] Q9 (scheduled, n): Erosion limits. Due after report-only ends (2 weeks or 10 tasks from go-live); Nano sets them from the calibration numbers. (D42)

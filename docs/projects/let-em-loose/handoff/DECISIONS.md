# Decisions: let-em-loose

The decision log is [../DECISIONS.md](../DECISIONS.md). It is not copied here, and its D-numbering continues (D46 onward).

Tag mapping for rows written before this pack:
- **Agreed** (D1–D45, except D14) = `[agreed]`: Nano approved it. Don't change it without asking Nano.
- **Proposed** (D14) = `[open]`.

Provenance: "Agreed" rows are author-reported approvals. Their evidence is Nano's answers in Mill Board card comments and remind-me decision memories, so cite one of those alongside the D# when you can. The #1384 review asks for this to be made explicit. Merging docs authorizes nothing: implementation starts, live or paid model calls, spending and deployment each still need Nano's go on the card.

Still disputed on #1384 (open review, 2026-11-03). Don't answer these from the D-row alone:
- D34 "stored records keep old provider names; readers map them": the merged #1408 contract (save-time conversion) wins. Read the code and ADR-0019.
- D37 "only Nano can commit SIGNOFF (CODEOWNERS)": CODEOWNERS doesn't authenticate the author. Use the merged #597 host golden-set checks.
- D43 says "inbox Approve is the override". The merged inbox's budget Approve only extends the allowance and reopens or resumes, and the block stays. Only an explicit human `decision accept` or `choose` (addendum C-R8) bypasses it.
- D42 has no defined behaviour when report-only ends before Nano sets limits, and D44 can schedule a cleanup that can't pass when a metric is already zero. Both are design gaps: OPEN_QUESTIONS Q10 and Q11.
- QUESTIONS Q13, Q14 and Q23 are weaker summaries. The normative text is the merged claims addendum (`crates/libs/protocol/rusty_bbp/docs/spec/bbp-claims-addendum.md`): tool-read blindness, authenticated runs, and revision-fenced human decisions (C-R8).

Rows from D46 on carry a v1.2 tag (`[stated]`, `[agreed]`, `[default]`) and a `remind-me:` line with the memory id, or `pending` when remind-me wasn't connected. From D52 on they also carry a stable `Key.` (`lel:<card>:<slug>`), which the expert uses to reconcile duplicates. Recording is best effort, not exactly-once; see the expert's Record section. Only the `let-em-loose-expert` subagent appends rows; see [.claude/agents/let-em-loose-expert.md](../../../../.claude/agents/let-em-loose-expert.md).

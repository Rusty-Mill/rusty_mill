# Decisions: let-em-loose

The decision log is [../DECISIONS.md](../DECISIONS.md). It is not copied here, and its D-numbering continues (D46 onward).

Tag mapping for rows written before this pack:
- **Agreed** (D1–D45, except D14) = `[agreed]`: the author's report of Nano's approval. Treat it as Nano's; don't change it without asking him.
- **Proposed** (D14) = `[open]`.

Provenance: "Agreed" rows are author-reported approvals. Their evidence is Nano's answers in Mill Board card comments and remind-me decision memories, so cite one of those alongside the D# when you can. ../DECISIONS.md § Authority, provenance and current contracts states this too. Merging docs authorizes nothing: implementation starts, live or paid model calls, spending and deployment each still need Nano's go on the card.

Corrected by [#1449](https://github.com/Rusty-Mill/rusty_mill/pull/1449) (merged 2026-11-04, 515bfb3f). These rows were disputed on #1384; the log now carries the corrected text, and [§ Authority, provenance and current contracts](../DECISIONS.md#authority-provenance-and-current-contracts) links the merged contracts behind them. Where a row and a merged contract still differ, the contract wins.
- D34: stored provider names convert at save time per ADR-0019; old binaries can't read new snapshots, so rollback means not resuming them.
- D37: golden-set SIGNOFF is human digest-attestation checked by the host. CODEOWNERS and file presence don't prove who wrote it.
- D42: at report-only expiry without approved limits, new affected tasks pause; open tasks keep their settings and frozen digests.
- D43: a budget Approve only extends the allowance and reopens or resumes, and the failure block stays. Only an explicit, revision-fenced human `accept` or `choose` (addendum C-R8) bypasses it.
- D44: a cleanup passes when both deltas are ≤ 0 and at least one is < 0, and is skipped with the reason recorded when both baseline metrics are 0. The D42/D44 choices are Nano's, recorded in [#1384 comment 6614620497](https://github.com/Rusty-Mill/rusty_mill/pull/1384#issuecomment-6614620497).
- QUESTIONS Q13, Q14, Q23 (and Q19, Q28, Q30) were rewritten to point at the merged addendum and inbox contracts.

Rows from D46 on carry a v1.2 tag (`[stated]`, `[agreed]`, `[default]`) and a `remind-me:` line with the memory id, or `pending` when remind-me wasn't connected. From D52 on they also carry a stable `Key.` (`lel:<card>:<slug>`), which the expert uses to reconcile duplicates. Recording is best effort, not exactly-once; see the expert's Record section. Only the `let-em-loose-expert` subagent appends rows; see [.claude/agents/let-em-loose-expert.md](../../../../.claude/agents/let-em-loose-expert.md).

---
category: Added
changelog: Let Em Loose smoke read-back writes a grounding bundle for every escalated claim (D155), before the operator acts on it.
---
## 2026-11-18 - Let Em Loose grounding bundle in the read-back

- **Added:** `docs/projects/let-em-loose/1166-grounding.sh`: for every claim in the task's inbox it writes the brief (numbered), each scoring vote with its note and what it cites, mechanical flags (cites a claim as a run, cites itself or the question), wording the note shares with the claim but the brief lacks, and the scores for all votes, the latest vote per author and the first vote per author. `1166-smoke-run.sh` and `1166-smoke-resume.sh` write it to `~/smoke-N/grounding.md` and tell the operator not to act before the audit (D155). The audit itself, the exact brief sentence each note relies on, stays Claude's.
- **Known limitations:** tested on a throwaway `bbp` store with a claim shaped like smoke-4's claim 6 (it flags the claim cited as a run, the self-citation and the floor/86400/stat wording), not on BaileyAi.

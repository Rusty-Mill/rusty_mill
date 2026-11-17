---
category: Fixed
changelog: rusty_orch lens runner re-reads a question when a new candidate answers it, so candidates are reviewed within one run.
---
## 2026-11-17 - Lens seats re-read a question when a candidate answers it

- **Fixed:** a lens seat refreshed claim views from `claims_delta`, which returns only claims that changed. A new candidate that answers a question does not mark the question changed, so within one `lens run` the question's cached view kept `candidates: []`, and `attack_ready` (two candidates needed) never let the lenses review them; the question stayed open. The seat now re-reads, with `claim_card`, each question it already follows when a delta carries a claim that answers it (D148). A restarted run was unaffected, because it re-read everything from revision 0, which is why run 1's candidates were reviewed only in its resume.
- **Known limitations:** one extra `claim_card` call per question per refresh. The protocol-side alternative (`delta_ids` including the answered question) is not taken: it changes the `claims_delta` contract in the BBP core.

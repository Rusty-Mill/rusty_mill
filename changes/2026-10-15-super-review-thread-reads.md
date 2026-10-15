---
category: Fixed
changelog: The super-review and super-qa skills no longer tell lane agents to read reviewThreads through gh pr view, which halted the run.
---
## 2026-10-15 - Review lanes read PR threads with valid gh calls

- **Fixed:** the "State protocol" bullet in `super-review` and `super-qa` said to read PR review threads. Lane agents turned that into `gh pr view --json reviewThreads`, which is not a valid field, so the GitHub read helper halted the run twice. Both skills now name `gh pr view --json comments` and `gh api repos/<owner>/<repo>/pulls/<PR>/comments`.
- **Known limitations:** lane agents still choose their own `gh` calls, so another invalid field can still halt a run.

---
category: Fixed
changelog: Mill Board's header buttons work as soon as the page shows who you are, which also removes a one-in-four flake in the Settings browser test.
---
## 2026-11-16 - Mill Board: header buttons wired before the page loads its metadata

- **Fixed:** the web UI set the actor field, then awaited `board_projects`, and only then wired the Settings, view,
  Add and sign-out controls. A click in that gap did nothing. `tests/ui/settings.mjs` waits for the actor field and
  clicks the gear, so it lost the race about one run in four (`#settings[open]` never appeared). The controls are
  now wired before `loadMeta`. 45 consecutive local runs of the test pass.

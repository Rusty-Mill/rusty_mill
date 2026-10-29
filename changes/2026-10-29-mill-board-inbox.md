---
category: Changed
changelog: Mill Board's web UI shows the default project as the Inbox, always listed first in the side panel.
---
## 2026-10-29 - Mill Board Inbox

- **Changed:** the web UI names the `default` project, where a card that names no project lands (rusty_tick's
  Inbox list), "Inbox" on chips, the roadmap heading, the card dialog's project picker and the agent project
  picker. The side panel lists it first and no longer hides it when empty, so there is always somewhere to
  file an unsorted card.
- The project's name is unchanged in the API, MCP tools, credentials and projects file: no migration.

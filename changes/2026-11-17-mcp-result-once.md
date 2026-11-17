---
category: Changed
changelog: Mill Board MCP tool results are sent once, as text, instead of twice (text and structuredContent), halving their size.
---
## 2026-11-17 - Mill Board MCP results are sent once

- **Changed:** each `/mcp` tool result carried the whole JSON twice, in a text block and in `structuredContent` (a full `board_list` was 6.3 MB against 3.1 MB on the HTTP API). It now carries the text block only; tools declare no `outputSchema`, so the second copy added nothing.
- **Known limitations:** a client that read `structuredContent` must parse the text block instead.

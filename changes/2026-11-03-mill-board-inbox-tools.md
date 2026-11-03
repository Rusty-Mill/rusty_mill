---
category: Added
changelog: Mill Board inbox_list and inbox_act MCP tools over the BBP decision inbox, admin only, with --bbp-bin and --bbp-root (#595).
---
## 2026-11-03 - Mill Board decision inbox: MCP tools

- **Added:** `inbox_list` and `inbox_act`, in the shared operation table, so they are MCP tools and `/api` calls (#595). Both need `admin`, so a model credential is refused.
- **Added:** `inbox_list` reads the open escalations of the BBP tasks linked from the cards the caller may see. It sorts them as BBP does, irreversible first, and attaches each one's card and its reason in plain words.
- **Added:** `inbox_act` takes `approve`, `reject`, `choose` or `refine` on an escalation id at the `rev` the caller read. With `dry_run` it writes nothing. It reads the escalation again before sending anything, so an escalation that has closed or moved on is refused at the board, and BBP fences the rest.
- **Added:** `--bbp-bin` and `--bbp-root` (`MILL_BOARD_BBP_BIN`, `MILL_BOARD_BBP_ROOT`) turn the inbox on. Without them the tools say how to enable it.
- **Known limitations:** no Remediate card yet, because the host does not report whether a claim was acted on. There is no web page yet. The Refine and task-total limits of the previous slice still apply.

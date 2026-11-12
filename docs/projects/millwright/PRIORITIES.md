# Millwright: project priorities

Nano commits work by setting a project's priority (DECISIONS.md backlog-commit). This file
is the current view of those priorities. Agents pull work from it; a card's own priority
field does not commit anything. Change a row here only after Nano's change is recorded
in [DECISIONS.md](DECISIONS.md).

Tiers:

- **High:** the Millwright critical path.
- **Normal:** active; one change in flight at a time.
- **Low:** no new work unless it unblocks a High project.
- **Paused:** takes no new cards; its open cards stay open under Mill Board's paused project status (DECISIONS.md paused-project-status).
- **Closed:** experiments with a conclusion in remind-me.

| Board project | Tier | Next milestone | Source |
| --- | --- | --- | --- |
| let_em_loose | High | L0, L1 | project-priorities |
| bbp_factory | High (with Let Em Loose) | L1 (breaker wiring) | project-priorities |
| mill_board | High | G1, then B1 | project-priorities |
| rusty-remind-me | High | R1 | project-priorities |
| portfolio_intelligence | High | P1 | project-priorities |
| rusty_mux | Normal | none | project-priorities |
| rusty_term | Normal | none | project-priorities |
| pythrust | Normal | none | project-priorities |
| rusty_mill | Normal | G0 (this rule) | project-priorities |
| ai_env_manager | Normal | M0 core and linker (Ai Env Manager's own milestone, not a Millwright one) | ai-env-manager-priority |
| converge_continuum | Low (not paused) | S3 after X4 | project-priorities |
| communications | Low | S2 (one trial, then close) | project-priorities |
| rusty_fclone | Low | L3 verdict compares against it | project-priorities |
| ci | Low | none | project-priorities |
| skill_pack | Low | none | project-priorities |
| expert_agents | Low | none | project-priorities |
| tokio_migration | Low | none | project-priorities |
| agent_cloud | Low | none | project-priorities |
| rusty_tick | Low | archive after B3 | project-priorities, board-store |
| rocket_league | Paused | B0 (paused status) | project-priorities, paused-project-status |
| apprentice_ai | Paused | B0 (paused status) | project-priorities, paused-project-status |
| mediboard | Paused | B0 (paused status) | project-priorities, paused-project-status |
| rusty_meshed | Paused | B0 (paused status) | project-priorities, paused-project-status |
| ledger | Closed | S0, S1 | close-ledger, b0-triage-approved |
| conflux | Closed (paused) | S0, S1 | pause-conflux, b0-triage-approved |
| swarm_os | Closed (archived prototype) | none | archive-swarm-os, b0-triage-approved |

The `default` project and the duplicate spellings `converge-continuum` and
`portfolio-intelligence` hold no work of their own. They follow the rows above.

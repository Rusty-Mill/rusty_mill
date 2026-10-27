---
category: Changed
changelog: "`agentgateway-mcp`: `mcpGuardrails` calls to a processor share one connection per processor instance instead of dialling for every check; a connection that is lost is dialled again on the next call."
---
## 2026-10-27 - agentgateway-mcp: guardrail calls share a connection

- **Why:** every guardrail check built a new `rusty_tonic` channel, so each check paid a TCP connect, an HTTP/2 handshake and, for an `https://` processor, a TLS handshake. Under load the processor saw one connection per call.
- **Changed (`agentgateway-mcp`):** the route's compiled chain holds one channel per processor instance (one per authority in a `service` ring), made when the config is compiled. A channel dials on its first call and again after its connection is lost, and round-robin across instances is unchanged. The connect timeout is still the processor's `timeout`, now paid only when a connection is opened. An endpoint `rusty_tonic` cannot use is now reported when the config is compiled instead of on each call.
- **Tests (written first):** `checks_on_one_processor_share_one_connection` (six checks, both phases, one TCP connection; it saw six before the change), and `a_processor_that_restarts_is_dialled_again` (the processor stops and a new one takes its port; the next check reaches it).

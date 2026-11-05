---
category: Fixed
changelog: Gateway test mock MCP server ends a slow tool call on cancellation, so the backend-timeout test no longer leaks.
---
## 2026-11-05 - Gateway mock server honours cancellation

- **Fixed:** nextest reported `agentgateway::limits::a_backend_timeout_cuts_off_a_slow_tool_call` as leaky. The gateway sends `notifications/cancelled` when the 200 ms backend budget expires, but the test's mock MCP server slept the full `MOCK_DELAY_MS` regardless, so the child stayed alive past the test holding its inherited stderr. The mock's delay now wakes early once the call is cancelled. Test fixture only; no runtime change.

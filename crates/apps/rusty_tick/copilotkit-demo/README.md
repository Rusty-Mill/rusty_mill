# rusty_tick on CopilotKit

A CopilotKit React page in front of `rusty_tick`'s AG-UI assistant
(`POST /api/agent`). It is the check that the open AG-UI endpoint is not only
accepted by the reference client (`@ag-ui/client`, in
`rusty_agui/conformance`) but works with CopilotKit's own runtime and React
SDK. Nothing here is part of `rusty_tick`; it is a client of it.

```
browser (CopilotChat, useFrontendTool)
   │  CopilotKit protocol, same origin through Vite
   ▼
runtime.mjs  (CopilotRuntime + @ag-ui/client HttpAgent, holds the bearer token)
   │  AG-UI over SSE
   ▼
rusty_tick   POST /api/agent
```

## Run

Three processes, from the repository root and then this directory:

```
export RUSTY_TICK_TOKEN=<16+ characters>

# 1. the server
cargo run -p rusty_tick -- --web-dir crates/apps/rusty_tick/web/dist

# 2. the CopilotKit runtime, in front of it (port 4000)
cd crates/apps/rusty_tick/copilotkit-demo
npm install
npm run runtime

# 3. the page (port 5173)
npm run web
```

Open <http://localhost:5173> and type `add buy milk`. The assistant answers,
asks the page to run its `create_task` tool, the task appears in the list, and
the assistant confirms on the follow-up run. That is the whole round trip:
text stream, a frontend tool call, the tool result sent back as a new run.

Settings, all optional: `AGENT_URL` (default `http://127.0.0.1:8787/api/agent`)
and `PORT` (default `4000`) for the runtime.

## Notes

- **The token stays on the server.** The page never sees it; the runtime
  adds it to each run. The runtime binds to loopback and sets no CORS headers,
  because Vite proxies `/api/copilotkit` to it.
- **Nothing calls CopilotKit's servers.** By default the runtime and the
  browser SDK report telemetry, and the dev inspector loads Google Fonts and a
  notice feed from `cdn.copilotkit.ai`. `runtime.mjs` sets
  `COPILOTKIT_TELEMETRY_DISABLED`, the page sets `enableInspector={false}`, and
  `package.json` turns off `@scarf/scarf`'s install-time analytics. Check the
  browser's network panel after upgrading CopilotKit.
- **The assistant is deliberately simple.** `rusty_tick`'s agent is
  deterministic and store-free: it understands `add <title>` and nothing else,
  so no model or API key is involved.
- **Not in CI.** It was verified by hand in a headless browser. It pins
  CopilotKit `1.77.0` and `@ag-ui/client` `1.0.2`, the version
  `rusty_agui`'s conformance project also uses.

# rusty_tick on CopilotKit

A CopilotKit React page in front of `rusty_tick`'s AG-UI assistant
(`POST /api/agent`). It is the check that the open AG-UI endpoint is not only
accepted by the reference client (`@ag-ui/client`, in
`rusty_agui/conformance`) but works with CopilotKit's own runtime and React
SDK. Nothing here is part of `rusty_tick`; it is a client of it.

```
browser (CopilotChat, useFrontendTool)
   │  same origin, through Vite
   ▼
runtime.mjs  holds the bearer token
   │  /api/copilotkit  CopilotRuntime + @ag-ui/client HttpAgent ──▶ POST /api/agent
   │  /api/demo/tasks  list and add Inbox tasks ─────────────────▶ GET /api/v1/snapshot, POST /api/v1/tasks
   ▼
rusty_tick
```

## Run

You need a Rust toolchain and Node 22 (the versions this was checked with).
`rusty_tick`'s own web UI is not needed here, so there is no `--web-dir` and
no web build step. Three processes: the server from the repository root, the
other two from this directory. Use the same token for the first two.

bash:

```
export RUSTY_TICK_TOKEN=<16+ characters>
cargo run -p rusty_tick                      # 1. the server (first build is slow)

cd crates/apps/rusty_tick/copilotkit-demo
npm install
npm run runtime                              # 2. the CopilotKit runtime, port 4000
npm run web                                  # 3. the page, port 5173 (a third terminal)
```

PowerShell (each terminal needs its own `$env:` line, and it lasts only for
that session):

```
$env:RUSTY_TICK_TOKEN = "<16+ characters>"
cargo run -p rusty_tick                      # 1. the server (first build is slow)

$env:RUSTY_TICK_TOKEN = "<same value>"
cd crates/apps/rusty_tick/copilotkit-demo
npm install
npm run runtime                              # 2. the CopilotKit runtime, port 4000

cd crates/apps/rusty_tick/copilotkit-demo
npm run web                                  # 3. the page, port 5173 (a third terminal)
```

Open <http://localhost:5173> and type `add buy milk`. The assistant answers,
asks the page to run its `create_task` tool, the page adds the task to
`rusty_tick`'s Inbox, and the assistant confirms on the follow-up run. That is
the whole round trip: text stream, a frontend tool call, the tool result sent
back as a new run.

The list on the left is the Inbox read from `rusty_tick`, so it survives a
reload and shows tasks added any other way. To see the same tasks from the
server's side:

```
curl -H "Authorization: Bearer $RUSTY_TICK_TOKEN" localhost:8787/api/v1/snapshot
```
```
Invoke-RestMethod -Headers @{ Authorization = "Bearer $env:RUSTY_TICK_TOKEN" } http://127.0.0.1:8787/api/v1/snapshot
```

Settings, all optional: `AGENT_URL` (default `http://127.0.0.1:8787/api/agent`;
its origin is also where the task routes go) and `PORT` (default `4000`) for the
runtime.

## Notes

- **The token stays on the server.** The page never sees it; the runtime adds
  it to every call. The runtime binds to loopback and sets no CORS headers,
  because Vite proxies `/api/copilotkit` and `/api/demo` to it.
- **The task routes check their input.** `POST /api/demo/tasks` takes a JSON
  body of at most 4 KiB with a `title` of 1 to 500 characters, else `400`; only
  `GET` and `POST` are allowed; a `rusty_tick` failure is a `502` with no detail
  beyond that, and the page shows it.
- **Nothing calls CopilotKit's servers.** By default the runtime and the
  browser SDK report telemetry, and the dev inspector loads Google Fonts and a
  notice feed from `cdn.copilotkit.ai`. `runtime.mjs` sets
  `COPILOTKIT_TELEMETRY_DISABLED`, the page sets `enableInspector={false}`, and
  `package.json` turns off `@scarf/scarf`'s install-time analytics. Check the
  browser's network panel after upgrading CopilotKit.
- **The assistant is deliberately simple.** `rusty_tick`'s agent is
  deterministic and store-free: it understands `add <title>` and nothing else,
  so no model or API key is involved. It does not read the task list.
- **Not in CI.** It was verified by hand in a headless browser. It pins
  CopilotKit `1.77.0` and `@ag-ui/client` `1.0.2`, the version
  `rusty_agui`'s conformance project also uses.

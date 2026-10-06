// CopilotKit's runtime in front of rusty_tick's AG-UI endpoint.
//
// The browser talks to this process (CopilotKit's own protocol); this process
// registers rusty_tick's `POST /api/agent` as an AG-UI agent and relays runs to
// it with the bearer token, which therefore never reaches the browser.
import { createServer } from "node:http";
import { HttpAgent } from "@ag-ui/client";

// CopilotKit reports usage to its own servers unless told not to, and tells the
// browser to do the same. Off by default here; set it to "false" to opt back in.
process.env.COPILOTKIT_TELEMETRY_DISABLED ??= "true";
// Imported after the line above: the runtime reads the variable when it loads.
const { CopilotRuntime } = await import("@copilotkit/runtime/v2");
const { createCopilotNodeListener } = await import("@copilotkit/runtime/v2/node");

const token = process.env.RUSTY_TICK_TOKEN;
if (!token) {
  console.error("set RUSTY_TICK_TOKEN to the token rusty_tick was started with");
  process.exit(1);
}
const agentUrl = process.env.AGENT_URL ?? "http://127.0.0.1:8787/api/agent";
const port = Number(process.env.PORT ?? 4000);

const runtime = new CopilotRuntime({
  agents: {
    default: new HttpAgent({ url: agentUrl, headers: { Authorization: `Bearer ${token}` } }),
  },
});

// Loopback only, and no CORS: the Vite dev server proxies /api/copilotkit here.
const listener = createCopilotNodeListener({ runtime, basePath: "/api/copilotkit" });
createServer(listener).listen(port, "127.0.0.1", () => {
  console.log(`CopilotKit runtime on http://127.0.0.1:${port}/api/copilotkit -> ${agentUrl}`);
});

// CopilotKit's runtime in front of rusty_tick's AG-UI endpoint, plus the two
// task routes the page uses.
//
// The browser talks to this process; this process talks to rusty_tick with the
// bearer token, which therefore never reaches the browser:
//   /api/copilotkit    CopilotKit's protocol, relayed to rusty_tick's AG-UI agent
//   /api/demo/tasks    GET the Inbox's open tasks, POST {title} to add one
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
const tickOrigin = new URL(agentUrl).origin;
const port = Number(process.env.PORT ?? 4000);

const MAX_BODY = 4096;
const MAX_TITLE = 500;

/** One call to rusty_tick's REST API; throws with the status on a non-2xx reply. */
async function tick(path, init = {}) {
  const res = await fetch(tickOrigin + path, {
    ...init,
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
  });
  if (!res.ok) throw new Error(`rusty_tick ${init.method ?? "GET"} ${path} answered ${res.status}`);
  return res.json();
}

/** The open, untrashed tasks in the Inbox, and the Inbox's id (rusty_tick always has one). */
async function inbox() {
  const snapshot = await tick("/api/v1/snapshot");
  const tasks = snapshot.tasks
    .filter((t) => t.listId === snapshot.inboxId && t.status === "open" && t.deletedMs == null)
    .map((t) => ({ id: t.id, title: t.title }));
  return { inboxId: snapshot.inboxId, tasks };
}

function send(res, status, body) {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
}

/** The request body as JSON, or null when it is too large or not JSON. */
async function readJson(req) {
  let size = 0;
  const chunks = [];
  for await (const chunk of req) {
    size += chunk.length;
    if (size > MAX_BODY) return null;
    chunks.push(chunk);
  }
  try {
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    return null;
  }
}

async function tasks(req, res) {
  try {
    if (req.method === "GET") return send(res, 200, (await inbox()).tasks);
    if (req.method !== "POST") return send(res, 405, { error: "GET or POST only" });

    const body = await readJson(req);
    const title = typeof body?.title === "string" ? body.title.trim() : "";
    if (!title || title.length > MAX_TITLE) {
      return send(res, 400, { error: `title must be 1 to ${MAX_TITLE} characters` });
    }
    const { inboxId } = await inbox();
    const task = await tick("/api/v1/tasks", {
      method: "POST",
      body: JSON.stringify({ listId: inboxId, title }),
    });
    return send(res, 201, { id: task.id, title: task.title });
  } catch (error) {
    console.error(error.message);
    return send(res, 502, { error: "rusty_tick did not accept the request" });
  }
}

const runtime = new CopilotRuntime({
  agents: {
    default: new HttpAgent({ url: agentUrl, headers: { Authorization: `Bearer ${token}` } }),
  },
});
const copilot = createCopilotNodeListener({ runtime, basePath: "/api/copilotkit" });

// Loopback only, and no CORS: the Vite dev server proxies both prefixes here.
createServer((req, res) =>
  req.url?.split("?")[0] === "/api/demo/tasks" ? tasks(req, res) : copilot(req, res),
).listen(port, "127.0.0.1", () => {
  console.log(`CopilotKit runtime on http://127.0.0.1:${port}/api/copilotkit -> ${agentUrl}`);
});

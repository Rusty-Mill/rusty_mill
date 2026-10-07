import { CopilotChat, CopilotKitProvider, useFrontendTool } from "@copilotkit/react-core/v2";
import "@copilotkit/react-core/v2/styles.css";
import { useEffect, useState } from "react";
import { z } from "zod";

type Task = { id: string; title: string };

async function api(method: "GET" | "POST", body?: unknown): Promise<unknown> {
  const res = await fetch("/api/demo/tasks", {
    method,
    headers: { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`the server answered ${res.status}`);
  return res.json();
}

/** The open tasks in rusty_tick's Inbox, and the tool that adds one. */
function Tasks() {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    api("GET")
      .then((found) => live && setTasks(found as Task[]))
      .catch((e: Error) => live && setError(`Could not load tasks: ${e.message}`));
    return () => {
      live = false;
    };
  }, []);

  // A frontend tool: rusty_tick's agent asks for it by name, CopilotKit runs
  // this handler in the browser and sends the result back as a follow-up run.
  useFrontendTool({
    name: "create_task",
    description: "Create a task with the given title.",
    parameters: z.object({ title: z.string().describe("What the task is") }),
    handler: async ({ title }) => {
      try {
        const task = (await api("POST", { title })) as Task;
        setTasks((current) => [task, ...current]); // newest first, as rusty_tick lists them
        setError(null);
        return `created "${task.title}"`;
      } catch (e) {
        const message = `Could not create "${title}": ${(e as Error).message}`;
        setError(message);
        return message;
      }
    },
  });

  return (
    <section aria-label="Tasks">
      <h2>Inbox</h2>
      {error && <p role="alert" className="error">{error}</p>}
      {tasks.length === 0 ? (
        <p className="empty">None yet. Ask the assistant: add buy milk</p>
      ) : (
        <ul data-testid="tasks">
          {tasks.map((task) => (
            <li key={task.id}>{task.title}</li>
          ))}
        </ul>
      )}
    </section>
  );
}

export function App() {
  // enableInspector is off: the inspector loads fonts and a notice feed from
  // third-party hosts, and this page should call nothing but its own servers.
  return (
    <CopilotKitProvider runtimeUrl="/api/copilotkit" enableInspector={false}>
      <main>
        <Tasks />
        <div className="chat">
          <CopilotChat agentId="default" />
        </div>
      </main>
    </CopilotKitProvider>
  );
}

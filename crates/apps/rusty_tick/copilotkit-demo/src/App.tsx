import { CopilotChat, CopilotKitProvider, useFrontendTool } from "@copilotkit/react-core/v2";
import "@copilotkit/react-core/v2/styles.css";
import { useState } from "react";
import { z } from "zod";

/** The page's own task list: what rusty_tick's `create_task` tool acts on. */
function Tasks() {
  const [titles, setTitles] = useState<string[]>([]);

  // A frontend tool: rusty_tick's agent asks for it by name, CopilotKit runs
  // this handler in the browser and sends the result back as a follow-up run.
  useFrontendTool({
    name: "create_task",
    description: "Create a task with the given title.",
    parameters: z.object({ title: z.string().describe("What the task is") }),
    handler: async ({ title }) => {
      setTitles((current) => [...current, title]);
      return `created "${title}"`;
    },
  });

  return (
    <section aria-label="Tasks">
      <h2>Tasks</h2>
      {titles.length === 0 ? (
        <p className="empty">None yet. Ask the assistant: add buy milk</p>
      ) : (
        <ul data-testid="tasks">
          {titles.map((title, i) => (
            <li key={i}>{title}</li>
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

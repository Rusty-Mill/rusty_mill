// The workspace's own headless core against the same server the reference
// client is run against, so the two clients are held to one echo agent.

import { runAgent, type Event } from "@rusty-mill/agui-core";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { startEchoAgent, type EchoAgent } from "./echo-agent";

let agent: EchoAgent;

beforeAll(async () => {
  agent = await startEchoAgent();
});

afterAll(() => agent?.stop());

describe("rusty_agui AgentHandler, driven by @rusty-mill/agui-core", () => {
  it("runs, verifies and reduces a full run", async () => {
    const seen: Event[] = [];
    const result = await runAgent(
      { url: agent.url },
      { threadId: "thread-1", runId: "run-1", messages: [{ id: "u1", role: "user", content: "hello" }] },
      { onEvent: (event) => { seen.push(event); } },
    );
    expect(seen.map((e) => e.type)).toEqual([
      "RUN_STARTED",
      "STATE_SNAPSHOT",
      "STEP_STARTED",
      "STATE_DELTA",
      "TEXT_MESSAGE_START",
      "TEXT_MESSAGE_CONTENT",
      "TEXT_MESSAGE_END",
      "STEP_FINISHED",
      "RUN_FINISHED",
    ]);
    expect(result.state).toEqual({ turns: 1, echoed: "hello" });
    expect(result.result).toEqual({ echoed: "hello" });
    expect(result.outcome).toEqual({ type: "success" });
    expect(result.messages).toEqual([
      { id: "u1", role: "user", content: "hello" },
      { id: "1", role: "assistant", content: "you said: hello" },
    ]);
  });

  it("reconstructs a frontend tool call and reports RUN_ERROR", async () => {
    const withTool = await runAgent(
      { url: agent.url },
      {
        threadId: "thread-2",
        runId: "run-2",
        messages: [{ id: "u1", role: "user", content: "confirm this" }],
        tools: [{ name: "confirm", description: "Ask", parameters: { type: "object" } }],
      },
    );
    const assistant = withTool.messages.find((m) => m.role === "assistant" && "toolCalls" in m && m.toolCalls?.length);
    expect(assistant).toBeDefined();
    const call = (assistant as { toolCalls: { function: { name: string; arguments: string } }[] }).toolCalls[0]!;
    expect(call.function.name).toBe("confirm");
    expect(JSON.parse(call.function.arguments)).toEqual({ text: "confirm this" });

    const failed = await runAgent({ url: agent.url }, { threadId: "thread-3", runId: "run-3", messages: [{ id: "u1", role: "user", content: "fail" }] });
    expect(failed.events.map((e) => e.type)).toEqual(["RUN_STARTED", "RUN_ERROR"]);
    expect(failed.error).toEqual({ message: "asked to fail" });
  });
});

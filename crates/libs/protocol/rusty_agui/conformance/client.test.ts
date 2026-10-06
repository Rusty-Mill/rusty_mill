// The reference AG-UI client against rusty_agui's server. @ag-ui/client is
// the package CopilotKit's React SDK and OpenBot drive agents with, so a
// run it accepts, verifies and reduces is a run those products accept.

import { HttpAgent, type BaseEvent, type Message, type Tool } from "@ag-ui/client";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { startEchoAgent, type EchoAgent } from "./echo-agent";

let agent: EchoAgent;

beforeAll(async () => {
  agent = await startEchoAgent();
});

afterAll(() => agent?.stop());

async function run(text: string, tools: Tool[]) {
  const http = new HttpAgent({
    url: agent.url,
    threadId: "thread-1",
    initialMessages: [{ id: "u1", role: "user", content: text }],
  });
  const events: BaseEvent[] = [];
  const result = await http.runAgent(
    { runId: "run-1", tools },
    { onEvent: ({ event }) => { events.push(event); } },
  );
  return { http, events, result };
}

describe("rusty_agui AgentHandler, driven by @ag-ui/client", () => {
  it("streams a run the reference client verifies and reduces", async () => {
    const { http, events, result } = await run("hello", []);
    expect(events.map((e) => e.type)).toEqual([
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
    expect(http.state).toEqual({ turns: 1, echoed: "hello" });
    expect(result.result).toEqual({ echoed: "hello" });
    expect(result.newMessages).toEqual([
      expect.objectContaining({ role: "assistant", content: "you said: hello" }),
    ]);
    expect(http.messages.map((m: Message) => m.role)).toEqual(["user", "assistant"]);
  });

  it("delivers a frontend tool call with its arguments", async () => {
    const { http, events } = await run("confirm this", [
      {
        name: "confirm",
        description: "Ask the person to confirm",
        parameters: { type: "object", properties: { text: { type: "string" } } },
      },
    ]);
    const types = events.map((e) => e.type);
    expect(types.slice(0, 6)).toEqual([
      "RUN_STARTED",
      "STATE_SNAPSHOT",
      "STEP_STARTED",
      "TOOL_CALL_START",
      "TOOL_CALL_ARGS",
      "TOOL_CALL_END",
    ]);
    const assistant = http.messages.find((m) => m.role === "assistant" && "toolCalls" in m && m.toolCalls?.length);
    expect(assistant).toBeDefined();
    const call = (assistant as { toolCalls: { function: { name: string; arguments: string } }[] }).toolCalls[0];
    expect(call.function.name).toBe("confirm");
    expect(JSON.parse(call.function.arguments)).toEqual({ text: "confirm this" });
  });

  it("surfaces an agent failure as RUN_ERROR", async () => {
    const http = new HttpAgent({
      url: agent.url,
      threadId: "thread-2",
      initialMessages: [{ id: "u1", role: "user", content: "fail" }],
    });
    const events: BaseEvent[] = [];
    let error: unknown;
    const result = await http.runAgent(
      { runId: "run-2" },
      {
        onEvent: ({ event }) => { events.push(event); },
        onRunErrorEvent: ({ event }) => { error = event; },
      },
    );
    expect(events.map((e) => e.type)).toEqual(["RUN_STARTED", "RUN_ERROR"]);
    expect(error).toMatchObject({ type: "RUN_ERROR", message: "asked to fail" });
    expect(result.newMessages).toEqual([]);
    expect(result.result).toBeUndefined();
  });
});

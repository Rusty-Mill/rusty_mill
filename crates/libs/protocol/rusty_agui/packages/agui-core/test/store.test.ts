import { describe, expect, it } from "vitest";
import { AgentStore, lastAssistantText, parseArguments, type Snapshot } from "../src/index.js";
import { call, fakeAgent, run, say, type FakeAgent } from "../src/testing.js";

let ids = 0;
const newId = () => `id-${(ids += 1)}`;

function store(agent: FakeAgent, extra: Partial<ConstructorParameters<typeof AgentStore>[0]> = {}): AgentStore {
  ids = 0;
  return new AgentStore({ endpoint: { url: "http://agent", fetch: agent.fetch }, threadId: "t", newId, ...extra });
}

describe("AgentStore", () => {
  it("notifies subscribers as the reply streams and reports RUN_ERROR", async () => {
    const agent = fakeAgent((input, i) =>
      i === 0 ? run(input.threadId, input.runId, say("a1", "hi there")) : [{ type: "RUN_STARTED", threadId: "t", runId: input.runId }, { type: "RUN_ERROR", message: "boom" }],
    );
    const s = store(agent);
    const seen: Snapshot[] = [];
    const unsubscribe = s.subscribe(() => seen.push(s.getSnapshot()));
    await s.send("hello");
    expect(seen.some((snap) => snap.running)).toBe(true);
    expect(lastAssistantText(s.getSnapshot().messages)).toBe("hi there");
    expect(s.getSnapshot()).toMatchObject({ running: false, error: undefined });
    expect(agent.inputs[0]).toMatchObject({ threadId: "t", messages: [{ role: "user", content: "hello" }] });

    await s.send("again");
    expect(s.getSnapshot()).toMatchObject({ running: false, error: "boom" });
    unsubscribe();
    await s.send("silent");
    // The errored run added no assistant message; the unsubscribed listener saw nothing after.
    expect(seen.at(-1)?.messages).toHaveLength(3);
    expect(s.getSnapshot().messages).toHaveLength(4);
  });

  it("sends readables and state, and takes state back", async () => {
    const agent = fakeAgent((input) => run(input.threadId, input.runId, [{ type: "STATE_SNAPSHOT", snapshot: { n: 8 } }]));
    const s = store(agent);
    s.setReadable("page", { description: "current page", value: "/home" });
    s.setState({ n: 7 });
    await s.send("go");
    expect(agent.inputs[0]).toMatchObject({ state: { n: 7 }, context: [{ description: "current page", value: "/home" }] });
    expect(s.getSnapshot().state).toEqual({ n: 8 });
    s.removeReadable("page");
    await s.send("again");
    expect(agent.inputs[1]?.context).toEqual([]);
  });

  it("offers actions as tools, runs the handler, and follows up", async () => {
    const agent = fakeAgent((input, i) =>
      i === 0
        ? run(input.threadId, input.runId, [...say("a1", "Checking"), ...call("c1", "a1", "weather", { city: "Oslo" })])
        : run(input.threadId, input.runId, say("a2", "It is cold")),
    );
    const s = store(agent);
    s.setAction({ name: "weather", description: "Look up the weather", handler: async (args) => `cold in ${(args as { city: string }).city}` });
    await s.send("weather?");
    expect(agent.inputs[0]?.tools).toEqual([{ name: "weather", description: "Look up the weather", parameters: { type: "object", properties: {} } }]);
    expect(agent.inputs).toHaveLength(2);
    expect(agent.inputs[1]?.messages?.at(-1)).toMatchObject({ role: "tool", toolCallId: "c1", content: "cold in Oslo" });
    expect(s.toolCalls()).toMatchObject([{ status: "done", result: "cold in Oslo" }]);
    expect(lastAssistantText(s.getSnapshot().messages)).toBe("It is cold");
  });

  it("without a handler the call waits for respond, then continues; render sees it", async () => {
    const agent = fakeAgent((input, i) =>
      i === 0 ? run(input.threadId, input.runId, call("c1", "a1", "weather", { city: "Oslo" })) : run(input.threadId, input.runId, say("a2", "Thanks")),
    );
    const s = store(agent);
    s.setAction({ name: "weather", description: "d", render: ({ args, status, result }) => `${status}:${JSON.stringify(args)}:${String(result)}` });
    await s.send("weather?");
    const [entry] = s.toolCalls();
    expect(entry?.status).toBe("pending");
    expect(s.renderToolCall(entry!.call)).toBe('pending:{"city":"Oslo"}:undefined');
    s.respond("c1", "sunny");
    s.respond("c1", "twice is ignored");
    await new Promise((resolve) => setTimeout(resolve, 0));
    while (s.getSnapshot().running) await new Promise((resolve) => setTimeout(resolve, 0));
    expect(agent.inputs).toHaveLength(2);
    expect(s.renderToolCall(entry!.call)).toBe('done:{"city":"Oslo"}:sunny');
    expect(s.renderToolCall({ id: "x", type: "function", function: { name: "unknown", arguments: "" } })).toBeUndefined();
  });

  it("answers invalid arguments with an error and honours followUp: false", async () => {
    const agent = fakeAgent((input) => run(input.threadId, input.runId, [{ type: "TOOL_CALL_START", toolCallId: "c1", toolCallName: "weather", parentMessageId: "a1" }, { type: "TOOL_CALL_ARGS", toolCallId: "c1", delta: "{not json" }, { type: "TOOL_CALL_END", toolCallId: "c1" }]));
    const s = store(agent, { followUp: false });
    s.setAction({ name: "weather", description: "d", handler: () => "never" });
    await s.send("weather?");
    expect(agent.inputs).toHaveLength(1);
    expect(s.getSnapshot().messages.at(-1)).toMatchObject({ role: "tool", content: JSON.stringify({ error: "invalid arguments: {not json" }) });
  });

  it("parses arguments leniently", () => {
    const f = (args: string) => parseArguments({ id: "c", type: "function", function: { name: "n", arguments: args } });
    expect(f("")).toEqual({});
    expect(f('{"a":1}')).toEqual({ a: 1 });
    expect(f("{")).toBeUndefined();
  });
});

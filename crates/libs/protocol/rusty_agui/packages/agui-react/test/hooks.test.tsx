import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { cleanup } from "@testing-library/react";
import type { ReactNode } from "react";
import { AgentProvider, lastAssistantText, useAction, useAgent, useReadable, useSharedState } from "../src/index.js";
import { call, fakeAgent, run, say, type FakeAgent } from "@rusty-mill/agui-core/testing";

afterEach(cleanup);

let ids = 0;
const newId = () => `id-${(ids += 1)}`;

function mount(agent: FakeAgent, ui: ReactNode, extra: Partial<Parameters<typeof AgentProvider>[0]> = {}) {
  ids = 0;
  return render(
    <AgentProvider endpoint={{ url: "http://agent", fetch: agent.fetch }} threadId="t" newId={newId} {...extra}>
      {ui}
    </AgentProvider>,
  );
}

function Chat() {
  const { messages, running, error, send } = useAgent();
  return (
    <div>
      <button onClick={() => void send("hello")}>send</button>
      <output data-testid="last">{lastAssistantText(messages)}</output>
      <output data-testid="count">{messages.length}</output>
      <output data-testid="running">{String(running)}</output>
      <output data-testid="error">{error ?? ""}</output>
    </div>
  );
}

describe("useAgent", () => {
  it("sends a user message, streams the reply, and reports RUN_ERROR", async () => {
    const agent = fakeAgent((input, i) =>
      i === 0 ? run(input.threadId, input.runId, say("a1", "hi there")) : [{ type: "RUN_STARTED", threadId: "t", runId: input.runId }, { type: "RUN_ERROR", message: "boom" }],
    );
    mount(agent, <Chat />);
    expect(screen.getByTestId("running").textContent).toBe("false");
    await act(async () => {
      screen.getByText("send").click();
    });
    await waitFor(() => expect(screen.getByTestId("last").textContent).toBe("hi there"));
    expect(screen.getByTestId("count").textContent).toBe("2");
    expect(agent.inputs[0]).toMatchObject({ threadId: "t", messages: [{ role: "user", content: "hello" }] });

    await act(async () => {
      screen.getByText("send").click();
    });
    await waitFor(() => expect(screen.getByTestId("error").textContent).toBe("boom"));
    expect(screen.getByTestId("running").textContent).toBe("false");
  });
});

describe("useReadable and useSharedState", () => {
  function Page() {
    useReadable("current page", { route: "/home" });
    const [state, setState] = useSharedState<{ n: number } | null>();
    const { send } = useAgent();
    return (
      <div>
        <button onClick={() => setState({ n: 7 })}>set</button>
        <button onClick={() => void send("go")}>send</button>
        <output data-testid="state">{JSON.stringify(state)}</output>
      </div>
    );
  }

  it("sends context and state, and receives state back", async () => {
    const agent = fakeAgent((input) => run(input.threadId, input.runId, [{ type: "STATE_SNAPSHOT", snapshot: { n: 8 } }]));
    mount(agent, <Page />);
    await act(async () => {
      screen.getByText("set").click();
    });
    expect(screen.getByTestId("state").textContent).toBe('{"n":7}');
    await act(async () => {
      screen.getByText("send").click();
    });
    await waitFor(() => expect(screen.getByTestId("state").textContent).toBe('{"n":8}'));
    expect(agent.inputs[0]).toMatchObject({
      state: { n: 7 },
      context: [{ description: "current page", value: '{"route":"/home"}' }],
    });
  });
});

describe("useAction", () => {
  function WithTool({ handler }: { handler?: (args: { city: string }) => Promise<string> }) {
    useAction<{ city: string }, string>({
      name: "weather",
      description: "Look up the weather",
      parameters: { type: "object", properties: { city: { type: "string" } } },
      ...(handler ? { handler } : {}),
      render: ({ args, status, result, respond }) => (
        <div data-testid="weather">
          {status}:{args.city}:{result ?? ""}
          {status === "pending" && <button onClick={() => respond("sunny")}>answer</button>}
        </div>
      ),
    });
    const { messages, send, toolCalls, renderToolCall } = useAgent();
    return (
      <div>
        <button onClick={() => void send("weather?")}>send</button>
        <output data-testid="count">{messages.length}</output>
        {toolCalls.map(({ call }) => (
          <div key={call.id}>{renderToolCall(call) as ReactNode}</div>
        ))}
      </div>
    );
  }

  it("offers the tool, runs the handler, answers, and follows up", async () => {
    const agent = fakeAgent((input, i) =>
      i === 0
        ? run(input.threadId, input.runId, [...say("a1", "Checking"), ...call("c1", "a1", "weather", { city: "Oslo" })])
        : run(input.threadId, input.runId, say("a2", "It is cold")),
    );
    mount(agent, <WithTool handler={async (args) => `cold in ${args.city}`} />);
    await act(async () => {
      screen.getByText("send").click();
    });
    await waitFor(() => expect(screen.getByTestId("weather").textContent).toBe("done:Oslo:cold in Oslo"));
    expect(agent.inputs[0]?.tools).toEqual([{ name: "weather", description: "Look up the weather", parameters: { type: "object", properties: { city: { type: "string" } } } }]);
    // The follow-up run carried the tool message.
    expect(agent.inputs).toHaveLength(2);
    expect(agent.inputs[1]?.messages?.at(-1)).toMatchObject({ role: "tool", toolCallId: "c1", content: "cold in Oslo" });
    await waitFor(() => expect(screen.getByTestId("count").textContent).toBe("4"));
  });

  it("without a handler the call waits for the person, then continues", async () => {
    const agent = fakeAgent((input, i) =>
      i === 0 ? run(input.threadId, input.runId, call("c1", "a1", "weather", { city: "Oslo" })) : run(input.threadId, input.runId, say("a2", "Thanks")),
    );
    mount(agent, <WithTool />);
    await act(async () => {
      screen.getByText("send").click();
    });
    await waitFor(() => expect(screen.getByTestId("weather").textContent).toContain("pending:Oslo:"));
    expect(screen.getByText("answer")).toBeTruthy();
    expect(agent.inputs).toHaveLength(1);
    await act(async () => {
      screen.getByText("answer").click();
    });
    await waitFor(() => expect(screen.getByTestId("weather").textContent).toBe("done:Oslo:sunny"));
    await waitFor(() => expect(agent.inputs).toHaveLength(2));
    expect(agent.inputs[1]?.messages?.at(-1)).toMatchObject({ role: "tool", toolCallId: "c1", content: "sunny" });
  });
});

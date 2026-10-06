// The packages ship partially compiled; the JIT compiler links them in tests.
import "@angular/compiler";
import { provideZonelessChangeDetection, runInInjectionContext, signal, type ApplicationRef } from "@angular/core";
import { createApplication } from "@angular/platform-browser";
import { call, fakeAgent, run, say, type FakeAgent } from "@rusty-mill/agui-core/testing";
import { afterEach, describe, expect, it } from "vitest";
import { injectAction, injectAgent, injectReadable, injectSharedState, lastAssistantText, provideAgent, type StoreConfig } from "../src/index.js";

let ids = 0;
const newId = () => `id-${(ids += 1)}`;
let app: ApplicationRef | undefined;

/** An application with the agent provided at its root, and `setup` run in its injection context. */
async function context<T>(agent: FakeAgent, setup: () => T, extra: Partial<StoreConfig> = {}): Promise<T> {
  ids = 0;
  const config: StoreConfig = { endpoint: { url: "http://agent", fetch: agent.fetch }, threadId: "t", newId, ...extra };
  app = await createApplication({ providers: [provideZonelessChangeDetection(), ...provideAgent(config)] });
  const value = runInInjectionContext(app.injector, setup);
  app.tick(); // root effects run on the application's tick
  return value;
}

afterEach(() => app?.destroy());

const settle = async (predicate: () => boolean): Promise<void> => {
  for (let i = 0; i < 200 && !predicate(); i += 1) await new Promise((resolve) => setTimeout(resolve, 1));
};

describe("injectAgent", () => {
  it("sends a user message, streams the reply, and reports RUN_ERROR", async () => {
    const agent = fakeAgent((input, i) =>
      i === 0 ? run(input.threadId, input.runId, say("a1", "hi there")) : [{ type: "RUN_STARTED", threadId: "t", runId: input.runId }, { type: "RUN_ERROR", message: "boom" }],
    );
    const { messages, running, error, send } = await context(agent, injectAgent);
    expect(running()).toBe(false);
    const first = send("hello");
    await settle(() => running());
    expect(running()).toBe(true);
    await first;
    expect(lastAssistantText(messages())).toBe("hi there");
    expect(messages()).toHaveLength(2);
    expect(agent.inputs[0]).toMatchObject({ threadId: "t", messages: [{ role: "user", content: "hello" }] });
    await send("again");
    expect(error()).toBe("boom");
    expect(running()).toBe(false);
  });

  it("stops listening when the application is destroyed", async () => {
    const agent = fakeAgent((input) => run(input.threadId, input.runId, say("a1", "late")));
    const { messages, send } = await context(agent, injectAgent);
    const sent = send("hello");
    app?.destroy();
    app = undefined;
    await sent;
    expect(messages()).toHaveLength(1);
  });
});

describe("injectReadable and injectSharedState", () => {
  it("sends context and state, follows a changing readable, and receives state back", async () => {
    const agent = fakeAgent((input) => run(input.threadId, input.runId, [{ type: "STATE_SNAPSHOT", snapshot: { n: 8 } }]));
    const route = signal({ route: "/home" });
    const page = await context(agent, () => {
      injectReadable("current page", route);
      const [shared, setShared] = injectSharedState<{ n: number } | null>();
      return { shared, setShared, ...injectAgent() };
    });
    page.setShared({ n: 7 });
    expect(page.shared()).toEqual({ n: 7 });
    await page.send("go");
    expect(page.shared()).toEqual({ n: 8 });
    expect(page.state()).toEqual({ n: 8 });
    expect(agent.inputs[0]).toMatchObject({ state: { n: 7 }, context: [{ description: "current page", value: '{"route":"/home"}' }] });
    route.set({ route: "/away" });
    app?.tick();
    await page.send("again");
    expect(agent.inputs[1]?.context).toEqual([{ description: "current page", value: '{"route":"/away"}' }]);
  });
});

describe("injectAction", () => {
  let respond: ((result: string) => void) | undefined;
  const weather = (handler?: (args: { city: string }) => Promise<string>) => ({
    name: "weather",
    description: "Look up the weather",
    parameters: { type: "object", properties: { city: { type: "string" } } },
    ...(handler ? { handler } : {}),
    render: (props: { args: { city: string }; status: string; result: string | undefined; respond: (result: string) => void }) => {
      respond = props.respond;
      return `${props.status}:${props.args.city}:${props.result ?? ""}`;
    },
  });

  it("offers the tool, runs the handler, answers, and follows up", async () => {
    const agent = fakeAgent((input, i) =>
      i === 0
        ? run(input.threadId, input.runId, [...say("a1", "Checking"), ...call("c1", "a1", "weather", { city: "Oslo" })])
        : run(input.threadId, input.runId, say("a2", "It is cold")),
    );
    const handle = await context(agent, () => {
      injectAction<{ city: string }, string>(weather(async (args) => `cold in ${args.city}`));
      return injectAgent();
    });
    await handle.send("weather?");
    expect(agent.inputs[0]?.tools).toEqual([{ name: "weather", description: "Look up the weather", parameters: { type: "object", properties: { city: { type: "string" } } } }]);
    expect(agent.inputs).toHaveLength(2);
    expect(agent.inputs[1]?.messages?.at(-1)).toMatchObject({ role: "tool", toolCallId: "c1", content: "cold in Oslo" });
    expect(handle.messages()).toHaveLength(4);
    expect(handle.renderToolCall(handle.toolCalls()[0]!.call)).toBe("done:Oslo:cold in Oslo");
  });

  it("without a handler the call waits for the person, then continues; a changed definition replaces the old", async () => {
    const agent = fakeAgent((input, i) =>
      i === 0 ? run(input.threadId, input.runId, call("c1", "a1", "weather", { city: "Oslo" })) : run(input.threadId, input.runId, say("a2", "Thanks")),
    );
    const action = signal(weather());
    const handle = await context(agent, () => {
      injectAction<{ city: string }, string>(action);
      return injectAgent();
    });
    await handle.send("weather?");
    const [entry] = handle.toolCalls();
    expect(entry?.status).toBe("pending");
    expect(handle.renderToolCall(entry!.call)).toBe("pending:Oslo:");
    expect(agent.inputs).toHaveLength(1);
    respond?.("sunny");
    await settle(() => agent.inputs.length === 2 && !handle.running());
    expect(agent.inputs[1]?.messages?.at(-1)).toMatchObject({ role: "tool", toolCallId: "c1", content: "sunny" });
    expect(handle.renderToolCall(entry!.call)).toBe("done:Oslo:sunny");

    action.set({ ...weather(), name: "forecast" });
    app?.tick();
    await handle.send("more");
    expect(agent.inputs[2]?.tools?.map((t) => t.name)).toEqual(["forecast"]);
  });
});

import { call, fakeAgent, run, say, type FakeAgent } from "@rusty-mill/agui-core/testing";
import { afterEach, describe, expect, it } from "vitest";
import { createApp, defineComponent, h, nextTick, ref, type App, type Component } from "vue";
import { lastAssistantText, provideAgent, useAction, useAgent, useReadable, useSharedState, type StoreConfig } from "../src/index.js";

let ids = 0;
const newId = () => `id-${(ids += 1)}`;
let app: App | undefined;
let root: HTMLElement;

function mount(agent: FakeAgent, child: Component, extra: Partial<StoreConfig> = {}): HTMLElement {
  ids = 0;
  root = document.createElement("div");
  const Provider = defineComponent({
    setup() {
      provideAgent({ endpoint: { url: "http://agent", fetch: agent.fetch }, threadId: "t", newId, ...extra });
      return () => h(child);
    },
  });
  app = createApp(Provider);
  app.mount(root);
  return root;
}

afterEach(() => app?.unmount());

const text = (id: string): string => root.querySelector(`[data-testid="${id}"]`)?.textContent ?? "";
const click = (label: string): void => {
  const button = [...root.querySelectorAll("button")].find((b) => b.textContent === label);
  if (!button) throw new Error(`no button ${label}`);
  button.click();
};
const settle = async (predicate: () => boolean): Promise<void> => {
  for (let i = 0; i < 200 && !predicate(); i += 1) await new Promise((resolve) => setTimeout(resolve, 1));
  await nextTick();
};

const Chat = defineComponent({
  setup() {
    const { messages, running, error, send } = useAgent();
    return () =>
      h("div", [
        h("button", { onClick: () => void send("hello") }, "send"),
        h("output", { "data-testid": "last" }, lastAssistantText(messages.value)),
        h("output", { "data-testid": "count" }, String(messages.value.length)),
        h("output", { "data-testid": "running" }, String(running.value)),
        h("output", { "data-testid": "error" }, error.value ?? ""),
      ]);
  },
});

describe("useAgent", () => {
  it("sends a user message, streams the reply, and reports RUN_ERROR", async () => {
    const agent = fakeAgent((input, i) =>
      i === 0 ? run(input.threadId, input.runId, say("a1", "hi there")) : [{ type: "RUN_STARTED", threadId: "t", runId: input.runId }, { type: "RUN_ERROR", message: "boom" }],
    );
    mount(agent, Chat);
    expect(text("running")).toBe("false");
    click("send");
    await settle(() => text("last") === "hi there");
    expect(text("count")).toBe("2");
    expect(agent.inputs[0]).toMatchObject({ threadId: "t", messages: [{ role: "user", content: "hello" }] });
    click("send");
    await settle(() => text("error") === "boom");
    expect(text("running")).toBe("false");
  });

  it("stops listening when unmounted", async () => {
    const agent = fakeAgent((input) => run(input.threadId, input.runId, say("a1", "late")));
    mount(agent, Chat);
    click("send");
    app?.unmount();
    app = undefined;
    await settle(() => agent.inputs.length === 1);
    expect(text("last")).toBe("");
  });
});

describe("useReadable and useSharedState", () => {
  const Page = defineComponent({
    setup() {
      const route = ref("/home");
      useReadable("current page", () => ({ route: route.value }));
      const state = useSharedState<{ n: number } | null>();
      const { send } = useAgent();
      return () =>
        h("div", [
          h("button", { onClick: () => (state.value = { n: 7 }) }, "set"),
          h("button", { onClick: () => (route.value = "/away") }, "move"),
          h("button", { onClick: () => void send("go") }, "send"),
          h("output", { "data-testid": "state" }, JSON.stringify(state.value)),
        ]);
    },
  });

  it("sends context and state, follows a changing readable, and receives state back", async () => {
    const agent = fakeAgent((input) => run(input.threadId, input.runId, [{ type: "STATE_SNAPSHOT", snapshot: { n: 8 } }]));
    mount(agent, Page);
    click("set");
    await nextTick();
    expect(text("state")).toBe('{"n":7}');
    click("send");
    await settle(() => text("state") === '{"n":8}');
    expect(agent.inputs[0]).toMatchObject({ state: { n: 7 }, context: [{ description: "current page", value: '{"route":"/home"}' }] });
    click("move");
    await nextTick();
    click("send");
    await settle(() => agent.inputs.length === 2);
    expect(agent.inputs[1]?.context).toEqual([{ description: "current page", value: '{"route":"/away"}' }]);
  });
});

describe("useAction", () => {
  const withTool = (handler?: (args: { city: string }) => Promise<string>) =>
    defineComponent({
      setup() {
        useAction<{ city: string }, string>({
          name: "weather",
          description: "Look up the weather",
          parameters: { type: "object", properties: { city: { type: "string" } } },
          ...(handler ? { handler } : {}),
          render: ({ args, status, result, respond }) =>
            h("div", { "data-testid": "weather" }, [`${status}:${args.city}:${result ?? ""}`, status === "pending" ? h("button", { onClick: () => respond("sunny") }, "answer") : null]),
        });
        const { messages, send, toolCalls, renderToolCall } = useAgent();
        return () =>
          h("div", [
            h("button", { onClick: () => void send("weather?") }, "send"),
            h("output", { "data-testid": "count" }, String(messages.value.length)),
            ...toolCalls.value.map(({ call }) => h("div", { key: call.id }, [renderToolCall(call) as ReturnType<typeof h>])),
          ]);
      },
    });

  it("offers the tool, runs the handler, answers, and follows up", async () => {
    const agent = fakeAgent((input, i) =>
      i === 0
        ? run(input.threadId, input.runId, [...say("a1", "Checking"), ...call("c1", "a1", "weather", { city: "Oslo" })])
        : run(input.threadId, input.runId, say("a2", "It is cold")),
    );
    mount(agent, withTool(async (args) => `cold in ${args.city}`));
    click("send");
    await settle(() => text("weather") === "done:Oslo:cold in Oslo");
    expect(agent.inputs[0]?.tools).toEqual([{ name: "weather", description: "Look up the weather", parameters: { type: "object", properties: { city: { type: "string" } } } }]);
    expect(agent.inputs).toHaveLength(2);
    expect(agent.inputs[1]?.messages?.at(-1)).toMatchObject({ role: "tool", toolCallId: "c1", content: "cold in Oslo" });
    await settle(() => text("count") === "4");
  });

  it("without a handler the call waits for the person, then continues", async () => {
    const agent = fakeAgent((input, i) =>
      i === 0 ? run(input.threadId, input.runId, call("c1", "a1", "weather", { city: "Oslo" })) : run(input.threadId, input.runId, say("a2", "Thanks")),
    );
    mount(agent, withTool());
    click("send");
    await settle(() => text("weather") === "pending:Oslo:");
    expect(agent.inputs).toHaveLength(1);
    click("answer");
    await settle(() => text("weather") === "done:Oslo:sunny");
    expect(agent.inputs).toHaveLength(2);
    expect(agent.inputs[1]?.messages?.at(-1)).toMatchObject({ role: "tool", toolCallId: "c1", content: "sunny" });
  });
});

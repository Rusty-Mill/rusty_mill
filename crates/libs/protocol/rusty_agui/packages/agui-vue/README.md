# @rusty-mill/agui-vue

Vue composables over `@rusty-mill/agui-core`. Headless: a provider, four
composables and render functions. No components, no styles; a chat UI is
a few lines over `useAgent`. The store behind them is the core's
`AgentStore`, the same one the React and Angular bindings wrap.

| Composable | What it does |
|---|---|
| `provideAgent({ endpoint, threadId?, initialMessages?, initialState? })` | In a `setup`: holds one thread with one agent for the tree below. Or `app.provide(AGENT, new AgentStore(config))` for the whole app. |
| `useAgent()` | Computed refs `messages`, `state`, `running`, `error`, `toolCalls`; `send(text)`, `run()`, `stop()`, `renderToolCall(call)` |
| `useReadable(description, value)` | Exposes application context to the agent while the scope lives (`RunAgentInput.context`); a ref or getter is followed |
| `useAction({ name, description, parameters?, handler?, render? })` | Registers a frontend tool while the scope lives (`RunAgentInput.tools`); a ref or getter is followed |
| `useSharedState()` | A writable computed ref: what you set, the next run sends, and the agent's snapshots and deltas update it |

```ts
// Chat.vue, <script setup>
const { messages, running, send, toolCalls, renderToolCall } = useAgent();
useReadable("current page", () => ({ route: route.path }));
useAction<{ city: string }, string>({
  name: "weather",
  description: "Look up the weather",
  parameters: { type: "object", properties: { city: { type: "string" } } },
  handler: async ({ city }) => fetchWeather(city),            // answered, run continues
  render: ({ args, status, result }) => h(WeatherCard, { city: args.city, status, result }),
});
```

```vue
<Bubble v-for="m in messages" :key="m.id" :message="m" />
<component :is="() => renderToolCall(call)" v-for="{ call } in toolCalls" :key="call.id" />
<Composer :disabled="running" @submit="send" />
```

**Frontend tools, generative UI, human in the loop and errors** work as
in [the React binding](../agui-react/README.md): an action with a
`handler` is answered and the run follows up; one with only a `render`
stays `pending` until the rendered UI calls `respond(result)`;
`render` receives the parsed `args`, the call's `status` and the
`result`; a failed run ends with `error` set and `running` false.

Tests mount components with `createApp` under jsdom against a scripted
fake agent from `@rusty-mill/agui-core/testing`.

```
npm ci && npm run typecheck && npm test && npm run build
```

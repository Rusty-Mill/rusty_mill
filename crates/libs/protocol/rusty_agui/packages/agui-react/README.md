# @rusty-mill/agui-react

React hooks over `@rusty-mill/agui-core`. Headless: a provider, four hooks
and render functions. No components, no styles; a chat UI is a few lines
over `useAgent`. The store behind them is the core's `AgentStore`, the
same one the Vue and Angular bindings wrap.

| Hook | What it does |
|---|---|
| `<AgentProvider endpoint threadId? initialMessages? initialState?>` | Holds one thread with one agent. Every hook reads from it. |
| `useAgent()` | `messages`, `state`, `running`, `error`; `send(text)`, `run()`, `stop()`; `toolCalls` and `renderToolCall(call)` |
| `useReadable(description, value)` | Exposes application context to the agent while the component lives (`RunAgentInput.context`) |
| `useAction({ name, description, parameters?, handler?, render? })` | Registers a frontend tool while the component lives (`RunAgentInput.tools`) |
| `useSharedState()` | `[state, setState]`; the next run sends what you set, and the agent's snapshots and deltas update it |

```tsx
function Chat() {
  const { messages, running, send, toolCalls, renderToolCall } = useAgent();
  useReadable("current page", { route: location.pathname });
  useAction<{ city: string }, string>({
    name: "weather",
    description: "Look up the weather",
    parameters: { type: "object", properties: { city: { type: "string" } } },
    handler: async ({ city }) => fetchWeather(city),            // answered, run continues
    render: ({ args, status, result }) => <WeatherCard city={args.city} status={status} result={result} />,
  });
  return (
    <>
      {messages.map((m) => <Bubble key={m.id} message={m} />)}
      {toolCalls.map(({ call }) => <div key={call.id}>{renderToolCall(call)}</div>)}
      <Composer disabled={running} onSubmit={send} />
    </>
  );
}

<AgentProvider endpoint={{ url: "/api/agent" }}><Chat /></AgentProvider>
```

**Frontend tools.** Each registered action is offered to the agent as a
tool. When the agent calls one, the run ends on the agent's side and the
store answers it: an action with a `handler` runs it and appends the tool
message; an action with only a `render` stays `pending` until the rendered
UI calls `respond(result)`, which is the human-in-the-loop pattern. Either
way a follow-up run starts automatically (`followUp={false}` on the
provider to stop that) so the agent sees the answer.

**Generative UI** is `render` on an action: it receives the parsed `args`,
the call's `status` (`running`, `pending`, `done`) and the `result`.

**Errors.** A `RUN_ERROR`, a transport fault, or a verifier rejection ends
the run with `error` set and `running` false; what arrived stays.

Tests run the hooks under jsdom against a scripted fake agent from
`@rusty-mill/agui-core/testing`.

```
npm ci && npm run typecheck && npm test && npm run build
```

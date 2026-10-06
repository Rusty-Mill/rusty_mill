# @rusty-mill/agui-angular

Angular signals over `@rusty-mill/agui-core`. Headless: a provider, four
inject functions and render functions. No components, no styles; a chat
UI is a few lines over `injectAgent`. The store behind them is the
core's `AgentStore`, the same one the React and Vue bindings wrap.

| Function | What it does |
|---|---|
| `provideAgent({ endpoint, threadId?, initialMessages?, initialState? })` | Providers for one thread with one agent: a component's `providers`, a route's, or the application's |
| `injectAgent()` | Signals `messages`, `state`, `running`, `error`, `toolCalls`; `send(text)`, `run()`, `stop()`, `renderToolCall(call)` |
| `injectReadable(description, value)` | Exposes application context to the agent until the injection context is destroyed (`RunAgentInput.context`); a signal is followed |
| `injectAction({ name, description, parameters?, handler?, render? })` | Registers a frontend tool until the injection context is destroyed (`RunAgentInput.tools`); a signal is followed |
| `injectSharedState()` | `[state, setState]`: a signal, and a setter whose value the next run sends; the agent's snapshots and deltas update it |

Each is called in an injection context: a constructor or a field
initializer.

```ts
@Component({
  selector: "app-chat",
  providers: provideAgent({ endpoint: { url: "/api/agent" } }),
  template: `
    @for (m of agent.messages(); track m.id) { <app-bubble [message]="m" /> }
    @for (entry of agent.toolCalls(); track entry.call.id) { <app-weather-card [props]="agent.renderToolCall(entry.call)" /> }
    <app-composer [disabled]="agent.running()" (submit)="agent.send($event)" />
  `,
})
export class ChatComponent {
  readonly agent = injectAgent();
  constructor() {
    injectReadable("current page", inject(Router).url);
    injectAction<{ city: string }, string>({
      name: "weather",
      description: "Look up the weather",
      parameters: { type: "object", properties: { city: { type: "string" } } },
      handler: async ({ city }) => fetchWeather(city),            // answered, run continues
      render: (props) => props,                                   // what the template hands the card
    });
  }
}
```

`render` returns whatever the template wants: a props object for a child
component, a string, a `TemplateRef`. The binding does not render.

**Frontend tools, generative UI, human in the loop and errors** work as
in [the React binding](../agui-react/README.md): an action with a
`handler` is answered and the run follows up; one with only a `render`
stays `pending` until the rendered UI calls `respond(result)`; a failed
run ends with `error` set and `running` false.

Tests build an application with `createApplication` (zoneless) against a
scripted fake agent from `@rusty-mill/agui-core/testing`; root effects
run on `tick()`. `@angular/compiler` is a dev dependency because the
packages ship partially compiled and the tests link them just in time.

```
npm ci && npm run typecheck && npm test && npm run build
```

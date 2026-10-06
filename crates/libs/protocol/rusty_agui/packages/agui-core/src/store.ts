// The agent store behind every framework binding: one thread's messages
// and state, the registered readables and actions, and the run loop. The
// shape is an external store (subscribe + getSnapshot), which React's
// `useSyncExternalStore`, a Vue `shallowRef` and an Angular `signal` each
// wrap in a few lines, so the bindings stay thin and the loop is tested
// once, here.

import { Reducer } from "./reduce.js";
import { streamAgent, type AgentEndpoint } from "./run.js";
import {
  contentText,
  type Context,
  type Event,
  type Json,
  type Message,
  type RunAgentInput,
  type Tool,
  type ToolCall,
  type ToolMessage,
  type UserMessage,
} from "./types.js";

export interface Snapshot {
  messages: Message[];
  state: Json;
  running: boolean;
  /** The last run's failure: a RUN_ERROR, a transport fault, or a thrown handler. */
  error: string | undefined;
}

/** How a frontend tool call stands, for its render function. */
export type ToolCallStatus = "running" | "pending" | "done";

/** One tool call in the thread with how it stands. */
export interface ToolCallEntry {
  call: ToolCall;
  message: Message;
  status: ToolCallStatus;
  result: Json | undefined;
}

export interface ActionRenderProps<Args = Json, Result = Json> {
  args: Args;
  status: ToolCallStatus;
  /** The tool message's content once answered. */
  result: Result | undefined;
  /** Answers a pending call (an action without a handler) and continues the run. */
  respond: (result: Result) => void;
}

export interface ActionDefinition<Args = Json, Result = Json> {
  name: string;
  description: string;
  /** JSON Schema for the arguments. Defaults to an open object. */
  parameters?: Json;
  /**
   * Runs when the agent calls the tool. Its result becomes the tool
   * message and the run continues. Without a handler the call stays
   * `pending` until `respond` is called from `render`: human in the loop.
   */
  handler?: (args: Args) => Result | Promise<Result>;
  /** Generative UI: what to show for this call. */
  render?: (props: ActionRenderProps<Args, Result>) => unknown;
}

export interface StoreConfig {
  endpoint: AgentEndpoint;
  threadId?: string;
  initialMessages?: Message[];
  initialState?: Json;
  /** Start a follow-up run after frontend tools answered. Default true. */
  followUp?: boolean;
  /** Id generator, for tests. Defaults to `crypto.randomUUID`. */
  newId?: () => string;
}

export interface SendOptions {
  /** Extra forwarded props for this run. */
  forwardedProps?: Json;
}

const defaultId = (): string => crypto.randomUUID();

export class AgentStore {
  readonly threadId: string;
  private snapshot: Snapshot;
  private readonly listeners = new Set<() => void>();
  private readonly readables = new Map<string, Context>();
  private readonly actions = new Map<string, ActionDefinition<Json, Json>>();
  private readonly results = new Map<string, Json>();
  private abort: AbortController | undefined;
  private readonly newId: () => string;

  constructor(private readonly config: StoreConfig) {
    this.threadId = config.threadId ?? (config.newId ?? defaultId)();
    this.newId = config.newId ?? defaultId;
    this.snapshot = {
      messages: structuredClone(config.initialMessages ?? []),
      state: structuredClone(config.initialState ?? null),
      running: false,
      error: undefined,
    };
  }

  // ------------------------------------------------------------- store

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): Snapshot => this.snapshot;

  private set(patch: Partial<Snapshot>): void {
    this.snapshot = { ...this.snapshot, ...patch };
    for (const listener of this.listeners) listener();
  }

  // --------------------------------------------------------- registries

  setReadable(id: string, context: Context): void {
    this.readables.set(id, context);
  }

  removeReadable(id: string): void {
    this.readables.delete(id);
  }

  setAction(action: ActionDefinition<Json, Json>): void {
    this.actions.set(action.name, action);
  }

  removeAction(name: string): void {
    this.actions.delete(name);
  }

  /** The tools the next run offers: one per registered action. */
  tools(): Tool[] {
    return [...this.actions.values()].map((a) => ({
      name: a.name,
      description: a.description,
      parameters: a.parameters ?? { type: "object", properties: {} },
    }));
  }

  context(): Context[] {
    return [...this.readables.values()];
  }

  // --------------------------------------------------------------- state

  /** Sets the shared state locally; the next run sends it to the agent. */
  setState(state: Json | ((previous: Json) => Json)): void {
    const next = typeof state === "function" ? state(this.snapshot.state) : state;
    this.set({ state: structuredClone(next) });
  }

  // --------------------------------------------------------------- calls

  /** Every tool call in the thread, with how it stands. */
  toolCalls(): ToolCallEntry[] {
    const answered = new Set<string>();
    for (const m of this.snapshot.messages) if (m.role === "tool") answered.add(m.toolCallId);
    const out: ToolCallEntry[] = [];
    for (const message of this.snapshot.messages) {
      if (message.role !== "assistant") continue;
      for (const call of message.toolCalls ?? []) {
        const done = answered.has(call.id);
        const status: ToolCallStatus = done ? "done" : this.snapshot.running ? "running" : "pending";
        out.push({ call, message, status, result: done ? this.results.get(call.id) : undefined });
      }
    }
    return out;
  }

  action(name: string): ActionDefinition<Json, Json> | undefined {
    return this.actions.get(name);
  }

  /** What the registered action's `render` shows for this call, if any. */
  renderToolCall(call: ToolCall): unknown {
    const action = this.actions.get(call.function.name);
    if (!action?.render) return undefined;
    const entry = this.toolCalls().find((c) => c.call.id === call.id);
    const props: ActionRenderProps<Json, Json> = {
      args: parseArguments(call) ?? {},
      status: entry?.status ?? "pending",
      result: entry?.result,
      respond: (result) => this.respond(call.id, result),
    };
    return action.render(props);
  }

  /** Answers a pending call and, by default, continues the run. */
  respond(toolCallId: string, result: Json): void {
    if (this.snapshot.messages.some((m) => m.role === "tool" && m.toolCallId === toolCallId)) return;
    this.answer(toolCallId, result);
    if (this.config.followUp !== false && !this.snapshot.running) void this.run();
  }

  private answer(toolCallId: string, result: Json): void {
    this.results.set(toolCallId, result);
    const message: ToolMessage = {
      id: this.newId(),
      role: "tool",
      content: typeof result === "string" ? result : JSON.stringify(result),
      toolCallId,
    };
    this.set({ messages: [...this.snapshot.messages, message] });
  }

  // ---------------------------------------------------------------- runs

  /** Appends a user message and runs the agent. */
  async send(text: string, options: SendOptions = {}): Promise<void> {
    const message: UserMessage = { id: this.newId(), role: "user", content: text };
    this.set({ messages: [...this.snapshot.messages, message], error: undefined });
    await this.run(options);
  }

  /** Runs the agent on the thread as it stands. Concurrent calls wait. */
  async run(options: SendOptions = {}): Promise<void> {
    if (this.snapshot.running) return;
    const input: RunAgentInput = {
      threadId: this.threadId,
      runId: this.newId(),
      state: this.snapshot.state,
      messages: this.snapshot.messages,
      tools: this.tools(),
      context: this.context(),
    };
    if (options.forwardedProps !== undefined) input.forwardedProps = options.forwardedProps;
    const reducer = Reducer.fromInput(input);
    this.abort = new AbortController();
    this.set({ running: true, error: undefined });
    let failure: string | undefined;
    try {
      for await (const event of streamAgent(this.config.endpoint, input, { signal: this.abort.signal })) {
        reducer.apply(event);
        if (event.type === "RUN_ERROR") failure = event.message;
        this.publish(reducer, event);
      }
    } catch (error) {
      failure = error instanceof Error ? error.message : String(error);
    } finally {
      this.abort = undefined;
      this.set({ running: false, error: failure });
    }
    if (failure === undefined) await this.answerToolCalls();
  }

  /** Stops the current run; what arrived stays. */
  stop(): void {
    this.abort?.abort();
  }

  private publish(reducer: Reducer, _event: Event): void {
    this.set({ messages: [...reducer.messages], state: reducer.state });
  }

  /** Runs handlers for unanswered calls and continues the run if any ran. */
  private async answerToolCalls(): Promise<void> {
    let answered = 0;
    for (const { call, status } of this.toolCalls()) {
      if (status === "done") continue;
      const action = this.actions.get(call.function.name);
      if (!action?.handler) continue;
      const args = parseArguments(call);
      if (args === undefined) {
        this.answer(call.id, { error: `invalid arguments: ${call.function.arguments}` });
        answered += 1;
        continue;
      }
      try {
        const result = await action.handler(args);
        this.answer(call.id, result ?? null);
      } catch (error) {
        this.answer(call.id, { error: error instanceof Error ? error.message : String(error) });
      }
      answered += 1;
    }
    if (answered > 0 && this.config.followUp !== false) await this.run();
  }
}

/** The call's arguments parsed, `{}` when empty, `undefined` when not JSON. */
export function parseArguments(call: ToolCall): Json | undefined {
  if (call.function.arguments === "") return {};
  try {
    return JSON.parse(call.function.arguments) as Json;
  } catch {
    return undefined;
  }
}

/** The text of the last assistant message, a convenience for simple UIs. */
export function lastAssistantText(messages: Message[]): string {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const m = messages[i]!;
    if (m.role === "assistant") return contentText(m.content);
  }
  return "";
}

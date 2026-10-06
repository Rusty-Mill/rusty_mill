// @rusty-mill/agui-angular: Angular signals over @rusty-mill/agui-core.
// Headless: a provider, four inject functions and render functions; no
// components, no styles. The store and its types are the core's,
// re-exported here.

import {
  AgentStore,
  type ActionDefinition,
  type Json,
  type Message,
  type SendOptions,
  type Snapshot,
  type StoreConfig,
  type ToolCall,
  type ToolCallEntry,
} from "@rusty-mill/agui-core";
import { computed, DestroyRef, effect, inject, InjectionToken, isSignal, signal, type Provider, type Signal } from "@angular/core";

export { AgentStore, lastAssistantText } from "@rusty-mill/agui-core";
export type { ActionDefinition, ActionRenderProps, SendOptions, Snapshot, StoreConfig, ToolCallEntry, ToolCallStatus } from "@rusty-mill/agui-core";

/** The token the store is provided under. */
export const AGENT = new InjectionToken<AgentStore>("agui");

/** Holds one thread with one agent for the injector's subtree: a component's `providers`, a route, or the application. */
export function provideAgent(config: StoreConfig): Provider[] {
  return [{ provide: AGENT, useFactory: () => new AgentStore(config) }];
}

/** The store itself, for the rare case the signals are not enough. */
export function injectAgentStore(): AgentStore {
  return inject(AGENT);
}

/** The store's snapshot as a signal, kept current until the injection context is destroyed. */
function injectSnapshot(store: AgentStore): Signal<Snapshot> {
  const snapshot = signal(store.getSnapshot());
  inject(DestroyRef).onDestroy(store.subscribe(() => snapshot.set(store.getSnapshot())));
  return snapshot.asReadonly();
}

export interface AgentHandle {
  threadId: string;
  messages: Signal<Message[]>;
  state: Signal<Json>;
  running: Signal<boolean>;
  /** The last run's failure: a RUN_ERROR, a transport fault, or a thrown handler. */
  error: Signal<string | undefined>;
  /** Every tool call in the thread with how it stands. */
  toolCalls: Signal<ToolCallEntry[]>;
  /** Appends a user message and runs the agent. */
  send: (text: string, options?: SendOptions) => Promise<void>;
  /** Runs the agent on the thread as it stands. */
  run: (options?: SendOptions) => Promise<void>;
  /** Stops the current run. */
  stop: () => void;
  /** What a registered action's `render` shows for this call, if any. */
  renderToolCall: (call: ToolCall) => unknown;
}

/** The thread: messages, state, running, error, and the verbs. Call in an injection context (a constructor or field initializer). */
export function injectAgent(): AgentHandle {
  const store = injectAgentStore();
  const snapshot = injectSnapshot(store);
  return {
    threadId: store.threadId,
    messages: computed(() => snapshot().messages),
    state: computed(() => snapshot().state),
    running: computed(() => snapshot().running),
    error: computed(() => snapshot().error),
    toolCalls: computed(() => (snapshot(), store.toolCalls())),
    send: (text, options) => store.send(text, options),
    run: (options) => store.run(options),
    stop: () => store.stop(),
    renderToolCall: (call) => store.renderToolCall(call),
  };
}

const read = <T>(value: T | Signal<T>): T => (isSignal(value) ? value() : value);

let readables = 0;

/** Exposes application context to the agent until the injection context is destroyed. A signal is followed. */
export function injectReadable(description: string, value: unknown | Signal<unknown>): void {
  const store = injectAgentStore();
  const id = `readable-${(readables += 1)}`;
  effect(() => {
    const current = read(value);
    store.setReadable(id, { description, value: typeof current === "string" ? current : JSON.stringify(current) });
  });
  inject(DestroyRef).onDestroy(() => store.removeReadable(id));
}

/**
 * Registers a frontend tool until the injection context is destroyed. With
 * a `handler`, the agent's call is answered automatically and the run
 * continues. With only a `render`, the call stays pending until the
 * rendered UI calls `respond`: human in the loop. A signal is followed:
 * the previous registration goes when the definition changes.
 */
export function injectAction<Args = Json, Result = Json>(action: ActionDefinition<Args, Result> | Signal<ActionDefinition<Args, Result>>): void {
  const store = injectAgentStore();
  effect((onCleanup) => {
    const current = read(action);
    store.setAction(current as unknown as ActionDefinition<Json, Json>);
    onCleanup(() => store.removeAction(current.name));
  });
}

/** The shared state as a signal, and a setter whose value the next run sends to the agent. */
export function injectSharedState<T extends Json = Json>(): [Signal<T>, (state: T | ((previous: T) => T)) => void] {
  const store = injectAgentStore();
  const snapshot = injectSnapshot(store);
  return [computed(() => snapshot().state as T), (state) => store.setState(state as Json | ((previous: Json) => Json))];
}

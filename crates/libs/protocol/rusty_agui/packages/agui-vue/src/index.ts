// @rusty-mill/agui-vue: Vue composables over @rusty-mill/agui-core. Headless:
// a provider, four composables and render functions; no components, no
// styles. The store and its types are the core's, re-exported here.

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
import {
  computed,
  inject,
  onScopeDispose,
  provide,
  shallowRef,
  toValue,
  watch,
  type ComputedRef,
  type InjectionKey,
  type MaybeRefOrGetter,
  type ShallowRef,
  type WritableComputedRef,
} from "vue";

export { AgentStore, lastAssistantText } from "@rusty-mill/agui-core";
export type { ActionDefinition, ActionRenderProps, SendOptions, Snapshot, StoreConfig, ToolCallEntry, ToolCallStatus } from "@rusty-mill/agui-core";

/** The injection key: `app.provide(AGENT, new AgentStore(config))` at app level. */
export const AGENT: InjectionKey<AgentStore> = Symbol("agui");

/** Holds one thread with one agent for the component tree below; every composable reads from it. */
export function provideAgent(config: StoreConfig): AgentStore {
  const store = new AgentStore(config);
  provide(AGENT, store);
  return store;
}

/** The store itself, for the rare case the composables are not enough. */
export function useAgentStore(): AgentStore {
  const store = inject(AGENT);
  if (!store) throw new Error("useAgent* composables need provideAgent() above them");
  return store;
}

/** The store's snapshot as a ref, kept current for as long as the scope lives. */
function useSnapshot(store: AgentStore): ShallowRef<Snapshot> {
  const snapshot = shallowRef(store.getSnapshot());
  onScopeDispose(store.subscribe(() => (snapshot.value = store.getSnapshot())));
  return snapshot;
}

export interface AgentHandle {
  threadId: string;
  messages: ComputedRef<Message[]>;
  state: ComputedRef<Json>;
  running: ComputedRef<boolean>;
  /** The last run's failure: a RUN_ERROR, a transport fault, or a thrown handler. */
  error: ComputedRef<string | undefined>;
  /** Every tool call in the thread with how it stands. */
  toolCalls: ComputedRef<ToolCallEntry[]>;
  /** Appends a user message and runs the agent. */
  send: (text: string, options?: SendOptions) => Promise<void>;
  /** Runs the agent on the thread as it stands. */
  run: (options?: SendOptions) => Promise<void>;
  /** Stops the current run. */
  stop: () => void;
  /** What a registered action's `render` shows for this call, if any. */
  renderToolCall: (call: ToolCall) => unknown;
}

/** The thread: messages, state, running, error, and the verbs. */
export function useAgent(): AgentHandle {
  const store = useAgentStore();
  const snapshot = useSnapshot(store);
  return {
    threadId: store.threadId,
    messages: computed(() => snapshot.value.messages),
    state: computed(() => snapshot.value.state),
    running: computed(() => snapshot.value.running),
    error: computed(() => snapshot.value.error),
    toolCalls: computed(() => (snapshot.value, store.toolCalls())),
    send: (text, options) => store.send(text, options),
    run: (options) => store.run(options),
    stop: () => store.stop(),
    renderToolCall: (call) => store.renderToolCall(call),
  };
}

let readables = 0;

/** Exposes application context to the agent for as long as the scope lives. A ref or getter is followed. */
export function useReadable(description: string, value: MaybeRefOrGetter<unknown>): void {
  const store = useAgentStore();
  const id = `readable-${(readables += 1)}`;
  watch(
    () => toValue(value),
    (current) => store.setReadable(id, { description, value: typeof current === "string" ? current : JSON.stringify(current) }),
    { immediate: true, deep: true },
  );
  onScopeDispose(() => store.removeReadable(id));
}

/**
 * Registers a frontend tool for as long as the scope lives. With a
 * `handler`, the agent's call is answered automatically and the run
 * continues. With only a `render`, the call stays pending until the
 * rendered UI calls `respond`: human in the loop. A ref or getter is
 * followed: the previous registration goes when the definition changes.
 */
export function useAction<Args = Json, Result = Json>(action: MaybeRefOrGetter<ActionDefinition<Args, Result>>): void {
  const store = useAgentStore();
  watch(
    () => toValue(action),
    (current, _previous, onCleanup) => {
      store.setAction(current as unknown as ActionDefinition<Json, Json>);
      onCleanup(() => store.removeAction(current.name));
    },
    { immediate: true },
  );
}

/** The shared state as a writable ref; what you set, the next run sends to the agent. */
export function useSharedState<T extends Json = Json>(): WritableComputedRef<T> {
  const store = useAgentStore();
  const snapshot = useSnapshot(store);
  return computed({
    get: () => snapshot.value.state as T,
    set: (state: T) => store.setState(state),
  });
}

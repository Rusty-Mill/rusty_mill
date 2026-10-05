// The hooks. Each is a thin view over one AgentStore held in context.

import type { Json, Message, ToolCall } from "@rusty-mill/agui-core";
import { createContext, useContext, useEffect, useId, useMemo, useSyncExternalStore, type ReactNode } from "react";
import { AgentStore, type ActionDefinition, type ActionRenderProps, type SendOptions, type Snapshot, type StoreConfig, type ToolCallStatus } from "./store.js";

const StoreContext = createContext<AgentStore | undefined>(undefined);

export interface AgentProviderProps extends StoreConfig {
  children?: ReactNode;
}

/** Holds one thread with one agent; every hook below reads from it. */
export function AgentProvider({ children, ...config }: AgentProviderProps): ReactNode {
  // The store is created once; later prop changes do not re-create a
  // thread mid-conversation.
  const store = useMemo(() => new AgentStore(config), []); // eslint-disable-line react-hooks/exhaustive-deps
  return <StoreContext.Provider value={store}>{children}</StoreContext.Provider>;
}

/** The store itself, for the rare case the hooks are not enough. */
export function useAgentStore(): AgentStore {
  const store = useContext(StoreContext);
  if (!store) throw new Error("useAgent* hooks need an <AgentProvider> above them");
  return store;
}

export interface AgentHandle extends Snapshot {
  threadId: string;
  /** Appends a user message and runs the agent. */
  send: (text: string, options?: SendOptions) => Promise<void>;
  /** Runs the agent on the thread as it stands. */
  run: (options?: SendOptions) => Promise<void>;
  /** Stops the current run. */
  stop: () => void;
  /** What a registered action's `render` shows for this call, if any. */
  renderToolCall: (call: ToolCall) => unknown;
  /** Every tool call in the thread with how it stands. */
  toolCalls: { call: ToolCall; message: Message; status: ToolCallStatus; result: Json | undefined }[];
}

/** The thread: messages, state, running, error, and the verbs. */
export function useAgent(): AgentHandle {
  const store = useAgentStore();
  const snapshot = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  return useMemo(
    () => ({
      ...snapshot,
      threadId: store.threadId,
      send: (text, options) => store.send(text, options),
      run: (options) => store.run(options),
      stop: () => store.stop(),
      toolCalls: store.toolCalls(),
      renderToolCall: (call) => {
        const action = store.action(call.function.name);
        if (!action?.render) return undefined;
        const entry = store.toolCalls().find((c) => c.call.id === call.id);
        let args: Json = {};
        try {
          args = call.function.arguments === "" ? {} : (JSON.parse(call.function.arguments) as Json);
        } catch {
          args = {};
        }
        const props: ActionRenderProps<Json, Json> = {
          args,
          status: entry?.status ?? "pending",
          result: entry?.result,
          respond: (result) => store.respond(call.id, result),
        };
        return action.render(props);
      },
    }),
    [snapshot, store],
  );
}

/** Exposes application context to the agent for as long as the component lives. */
export function useReadable(description: string, value: unknown): void {
  const store = useAgentStore();
  const id = useId();
  const text = typeof value === "string" ? value : JSON.stringify(value);
  useEffect(() => {
    store.setReadable(id, { description, value: text });
    return () => store.removeReadable(id);
  }, [store, id, description, text]);
}

/**
 * Registers a frontend tool for as long as the component lives. With a
 * `handler`, the agent's call is answered automatically and the run
 * continues. With only a `render`, the call stays pending until the
 * rendered UI calls `respond`: human in the loop. Both together show the
 * handler's result.
 */
export function useAction<Args = Json, Result = Json>(action: ActionDefinition<Args, Result>): void {
  const store = useAgentStore();
  // Re-register when the identity-bearing parts change; handler and render
  // are read at call time from the latest registration.
  useEffect(() => {
    store.setAction(action as unknown as ActionDefinition<Json, Json>);
    return () => store.removeAction(action.name);
  }, [store, action]);
}

/** The shared state, and a setter whose value the next run sends to the agent. */
export function useSharedState<T extends Json = Json>(): [T, (state: T | ((previous: T) => T)) => void] {
  const store = useAgentStore();
  const snapshot = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  return [snapshot.state as T, (state) => store.setState(state as Json | ((previous: Json) => Json))];
}

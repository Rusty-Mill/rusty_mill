// @rusty-mill/agui-react: React hooks over @rusty-mill/agui-core. Headless:
// a provider, four hooks and render functions; no components, no styles.
// The store and its types are the core's, re-exported for convenience.

export { AgentProvider, useAction, useAgent, useAgentStore, useReadable, useSharedState } from "./hooks.js";
export type { AgentHandle, AgentProviderProps } from "./hooks.js";
export { AgentStore, lastAssistantText } from "@rusty-mill/agui-core";
export type { ActionDefinition, ActionRenderProps, SendOptions, Snapshot, StoreConfig, ToolCallEntry, ToolCallStatus } from "@rusty-mill/agui-core";

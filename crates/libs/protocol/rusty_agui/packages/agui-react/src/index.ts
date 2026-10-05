// @rusty-mill/agui-react: React hooks over @rusty-mill/agui-core. Headless:
// a provider, four hooks and render functions; no components, no styles.

export { AgentProvider, useAction, useAgent, useAgentStore, useReadable, useSharedState } from "./hooks.js";
export type { AgentHandle, AgentProviderProps } from "./hooks.js";
export { AgentStore, lastAssistantText } from "./store.js";
export type { ActionDefinition, ActionRenderProps, SendOptions, Snapshot, StoreConfig, ToolCallStatus } from "./store.js";

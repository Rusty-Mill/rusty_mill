// @rusty-mill/agui-core: the headless AG-UI core. Types and a validating
// parser, SSE framing, the verifier with chunk expansion, the reducer, and
// `runAgent`/`streamAgent` over fetch. No runtime dependencies; the
// TypeScript mirror of the rusty_agui crate, sharing its fixtures.

export * from "./types.js";
export { applyPatch, mergePatch, parsePointer, resolvePointer, PatchError } from "./jsonPatch.js";
export { CONTENT_TYPE, Decoder, encode } from "./sse.js";
export { SequenceError, Verifier } from "./verify.js";
export { Reducer } from "./reduce.js";
export { HttpError, TransportError, runAgent, streamAgent } from "./run.js";
export type { AgentEndpoint, RunOptions, RunResult, StreamOptions } from "./run.js";
export { AgentStore, lastAssistantText, parseArguments } from "./store.js";
export type { ActionDefinition, ActionRenderProps, SendOptions, Snapshot, StoreConfig, ToolCallEntry, ToolCallStatus } from "./store.js";

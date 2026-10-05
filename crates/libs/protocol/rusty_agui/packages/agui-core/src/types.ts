// The wire types, exactly as AG-UI's TypeScript SDK names them. The
// Rust crate's `types` and `event` modules are the other half of this
// contract; `fixtures/` at the crate root is shared between the two.

export type Role = "developer" | "system" | "assistant" | "user" | "tool" | "activity" | "reasoning";

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

export interface TextPart {
  type: "text";
  text: string;
}

export interface BinaryPart {
  type: "binary";
  mimeType: string;
  data?: string;
  url?: string;
  filename?: string;
}

export type ContentPart = TextPart | BinaryPart;
export type Content = string | ContentPart[];

export interface FunctionCall {
  name: string;
  arguments: string;
}

export interface ToolCall {
  id: string;
  type: "function";
  function: FunctionCall;
}

export interface UserMessage {
  id: string;
  role: "user";
  content: Content;
  name?: string;
}

export interface AssistantMessage {
  id: string;
  role: "assistant";
  content?: string;
  name?: string;
  toolCalls?: ToolCall[];
}

export interface SystemMessage {
  id: string;
  role: "system";
  content: string;
  name?: string;
}

export interface DeveloperMessage {
  id: string;
  role: "developer";
  content: string;
  name?: string;
}

export interface ToolMessage {
  id: string;
  role: "tool";
  content: Content;
  toolCallId: string;
  error?: string;
}

export interface ActivityMessage {
  id: string;
  role: "activity";
  activityType: string;
  content: Json;
}

export interface ReasoningMessage {
  id: string;
  role: "reasoning";
  content: string;
  encryptedValue?: string;
}

export type Message =
  | UserMessage
  | AssistantMessage
  | SystemMessage
  | DeveloperMessage
  | ToolMessage
  | ActivityMessage
  | ReasoningMessage;

export interface Tool {
  name: string;
  description: string;
  parameters: Json;
}

export interface Context {
  description: string;
  value: string;
}

export interface RunAgentInput {
  threadId: string;
  runId: string;
  parentRunId?: string;
  state?: Json;
  messages?: Message[];
  tools?: Tool[];
  context?: Context[];
  forwardedProps?: Json;
}

/** The text of a message's content: the string, or its text parts joined. */
export function contentText(content: Content | undefined): string {
  if (content === undefined) return "";
  if (typeof content === "string") return content;
  return content.map((part) => (part.type === "text" ? part.text : "")).join("");
}

// ------------------------------------------------------------------ events

export interface EventMeta {
  timestamp?: number;
  subagentRunId?: string;
  metadata?: Json;
  rawEvent?: Json;
}

export type RunOutcome =
  | { type: "success" }
  | { type: "interrupt"; interrupts: Json[] };

export type Event = EventMeta &
  (
    | { type: "RUN_STARTED"; threadId: string; runId: string; parentRunId?: string; input?: RunAgentInput }
    | { type: "RUN_FINISHED"; threadId: string; runId: string; result?: Json; outcome?: RunOutcome }
    | { type: "RUN_ERROR"; message: string; code?: string }
    | { type: "STEP_STARTED"; stepName: string }
    | { type: "STEP_FINISHED"; stepName: string }
    | { type: "TEXT_MESSAGE_START"; messageId: string; role: Role }
    | { type: "TEXT_MESSAGE_CONTENT"; messageId: string; delta: string }
    | { type: "TEXT_MESSAGE_END"; messageId: string }
    | { type: "TEXT_MESSAGE_CHUNK"; messageId?: string; role?: Role; delta?: string }
    | { type: "TOOL_CALL_START"; toolCallId: string; toolCallName: string; parentMessageId?: string }
    | { type: "TOOL_CALL_ARGS"; toolCallId: string; delta: string }
    | { type: "TOOL_CALL_END"; toolCallId: string }
    | { type: "TOOL_CALL_RESULT"; messageId: string; toolCallId: string; content: string; role?: Role }
    | { type: "TOOL_CALL_CHUNK"; toolCallId?: string; toolCallName?: string; parentMessageId?: string; delta?: string }
    | { type: "STATE_SNAPSHOT"; snapshot: Json }
    | { type: "STATE_DELTA"; delta: Json[] }
    | { type: "MESSAGES_SNAPSHOT"; messages: Message[] }
    | { type: "ACTIVITY_SNAPSHOT"; messageId: string; activityType: string; content: Json; replace?: boolean }
    | { type: "ACTIVITY_DELTA"; messageId: string; activityType: string; patch: Json[] }
    | { type: "REASONING_START"; messageId: string }
    | { type: "REASONING_MESSAGE_START"; messageId: string; role?: "reasoning" }
    | { type: "REASONING_MESSAGE_CONTENT"; messageId: string; delta: string }
    | { type: "REASONING_MESSAGE_END"; messageId: string }
    | { type: "REASONING_MESSAGE_CHUNK"; messageId?: string; delta?: string }
    | { type: "REASONING_END"; messageId: string }
    | { type: "REASONING_ENCRYPTED_VALUE"; subtype: string; entityId: string; encryptedValue: string }
    | { type: "SUBAGENT_STARTED"; subagentRunId: string; name: string; description?: string; parentSubagentRunId?: string; parentToolCallId?: string; parentMessageId?: string }
    | { type: "SUBAGENT_FINISHED"; subagentRunId: string; result?: Json; outcome?: Json }
    | { type: "SUBAGENT_ERROR"; subagentRunId: string; message: string; code?: string }
    | { type: "RAW"; event: Json; source?: string }
    | { type: "CUSTOM"; name: string; value: Json }
  );

export type EventType = Event["type"];

/** Required fields per event type: a string member that must be present. */
const REQUIRED: Record<EventType, readonly string[]> = {
  RUN_STARTED: ["threadId", "runId"],
  RUN_FINISHED: ["threadId", "runId"],
  RUN_ERROR: ["message"],
  STEP_STARTED: ["stepName"],
  STEP_FINISHED: ["stepName"],
  TEXT_MESSAGE_START: ["messageId", "role"],
  TEXT_MESSAGE_CONTENT: ["messageId", "delta"],
  TEXT_MESSAGE_END: ["messageId"],
  TEXT_MESSAGE_CHUNK: [],
  TOOL_CALL_START: ["toolCallId", "toolCallName"],
  TOOL_CALL_ARGS: ["toolCallId", "delta"],
  TOOL_CALL_END: ["toolCallId"],
  TOOL_CALL_RESULT: ["messageId", "toolCallId", "content"],
  TOOL_CALL_CHUNK: [],
  STATE_SNAPSHOT: [],
  STATE_DELTA: [],
  MESSAGES_SNAPSHOT: [],
  ACTIVITY_SNAPSHOT: ["messageId", "activityType"],
  ACTIVITY_DELTA: ["messageId", "activityType"],
  REASONING_START: ["messageId"],
  REASONING_MESSAGE_START: ["messageId"],
  REASONING_MESSAGE_CONTENT: ["messageId", "delta"],
  REASONING_MESSAGE_END: ["messageId"],
  REASONING_MESSAGE_CHUNK: [],
  REASONING_END: ["messageId"],
  REASONING_ENCRYPTED_VALUE: ["subtype", "entityId", "encryptedValue"],
  SUBAGENT_STARTED: ["subagentRunId", "name"],
  SUBAGENT_FINISHED: ["subagentRunId"],
  SUBAGENT_ERROR: ["subagentRunId", "message"],
  RAW: [],
  CUSTOM: ["name"],
};

/** Array members that must be present. */
const REQUIRED_ARRAYS: Partial<Record<EventType, readonly string[]>> = {
  STATE_DELTA: ["delta"],
  MESSAGES_SNAPSHOT: ["messages"],
  ACTIVITY_DELTA: ["patch"],
};

export class DecodeError extends Error {
  override name = "DecodeError";
}

/**
 * Checks that `raw` is an AG-UI event: a known `type` and its required
 * members. Unknown members are kept, as the Rust codec ignores them, so a
 * newer peer still parses. Deltas may be empty here; the verifier rejects
 * those, as it rejects every other ordering fault.
 */
export function parseEvent(raw: unknown): Event {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    throw new DecodeError("event is not an object");
  }
  const obj = raw as Record<string, unknown>;
  const type = obj["type"];
  if (typeof type !== "string" || !(type in REQUIRED)) {
    throw new DecodeError(`unknown event type ${JSON.stringify(type)}`);
  }
  for (const field of REQUIRED[type as EventType]) {
    if (typeof obj[field] !== "string") {
      throw new DecodeError(`${type}: missing ${JSON.stringify(field)}`);
    }
  }
  for (const field of REQUIRED_ARRAYS[type as EventType] ?? []) {
    if (!Array.isArray(obj[field])) {
      throw new DecodeError(`${type}: ${JSON.stringify(field)} is not an array`);
    }
  }
  return obj as unknown as Event;
}

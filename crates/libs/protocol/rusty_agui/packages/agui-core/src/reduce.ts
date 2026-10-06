// Folds canonical events into what a client shows: messages and state.
// The mirror of the Rust `Reducer`, checked against `fixtures/runs.json`.

import { applyPatch, mergePatch } from "./jsonPatch.js";
import { SequenceError } from "./verify.js";
import type { AssistantMessage, Event, Json, Message, Role, RunAgentInput, ToolCall } from "./types.js";

function newMessage(id: string, role: Role): Message {
  switch (role) {
    case "assistant":
      return { id, role, content: "" };
    case "user":
      return { id, role, content: "" };
    case "system":
      return { id, role, content: "" };
    case "developer":
      return { id, role, content: "" };
    case "tool":
      return { id, role, content: "", toolCallId: "" };
    case "reasoning":
      return { id, role, content: "" };
    case "activity":
      return { id, role, activityType: "", content: {} };
  }
}

/** The client-side view of a thread. Feed it canonical (verified) events. */
export class Reducer {
  messages: Message[];
  state: Json;

  constructor(messages: Message[] = [], state: Json = null) {
    this.messages = structuredClone(messages);
    this.state = structuredClone(state);
  }

  /** A reducer seeded from a run input, the way a client resumes a thread. */
  static fromInput(input: RunAgentInput): Reducer {
    return new Reducer(input.messages ?? [], input.state ?? null);
  }

  find(id: string): Message | undefined {
    for (let i = this.messages.length - 1; i >= 0; i -= 1) {
      if (this.messages[i]!.id === id) return this.messages[i];
    }
    return undefined;
  }

  apply(event: Event): void {
    switch (event.type) {
      case "TEXT_MESSAGE_START":
        this.messages.push(newMessage(event.messageId, event.role));
        break;
      case "TEXT_MESSAGE_CONTENT":
      case "REASONING_MESSAGE_CONTENT":
        this.appendText(event.messageId, event.delta);
        break;
      case "TOOL_CALL_START":
        this.startToolCall(event.toolCallId, event.toolCallName, event.parentMessageId);
        break;
      case "TOOL_CALL_ARGS":
        this.appendArgs(event.toolCallId, event.delta);
        break;
      case "TOOL_CALL_RESULT":
        this.messages.push({ id: event.messageId, role: "tool", content: event.content, toolCallId: event.toolCallId });
        break;
      case "STATE_SNAPSHOT":
        this.state = structuredClone(event.snapshot);
        break;
      case "STATE_DELTA":
        this.state = applyPatch(this.state, event.delta);
        break;
      case "MESSAGES_SNAPSHOT":
        this.messages = structuredClone(event.messages);
        break;
      case "ACTIVITY_SNAPSHOT":
        this.activitySnapshot(event.messageId, event.activityType, event.content, event.replace ?? true);
        break;
      case "ACTIVITY_DELTA": {
        const message = this.find(event.messageId);
        if (!message || message.role !== "activity") {
          throw new SequenceError(`ACTIVITY_DELTA for unknown activity ${JSON.stringify(event.messageId)}`);
        }
        message.content = applyPatch(message.content, event.patch);
        break;
      }
      case "REASONING_MESSAGE_START":
        this.messages.push({ id: event.messageId, role: "reasoning", content: "" });
        break;
      case "REASONING_ENCRYPTED_VALUE": {
        const message = this.find(event.entityId);
        if (message?.role === "reasoning") message.encryptedValue = event.encryptedValue;
        break;
      }
      default:
        break;
    }
  }

  private appendText(id: string, delta: string): void {
    const message = this.find(id);
    if (!message) throw new SequenceError(`content for unknown message ${JSON.stringify(id)}`);
    switch (message.role) {
      case "assistant":
        message.content = (message.content ?? "") + delta;
        break;
      case "system":
      case "developer":
      case "reasoning":
        message.content += delta;
        break;
      case "user":
      case "tool":
        if (typeof message.content === "string") message.content += delta;
        else message.content.push({ type: "text", text: delta });
        break;
      case "activity":
        throw new SequenceError(`text content for activity message ${JSON.stringify(id)}`);
    }
  }

  private startToolCall(id: string, name: string, parent: string | undefined): void {
    const call: ToolCall = { id, type: "function", function: { name, arguments: "" } };
    let target: Message | undefined;
    if (parent !== undefined) target = this.find(parent);
    else {
      for (let i = this.messages.length - 1; i >= 0; i -= 1) {
        if (this.messages[i]!.role === "assistant") {
          target = this.messages[i];
          break;
        }
      }
    }
    if (target?.role === "assistant") {
      (target.toolCalls ??= []).push(call);
      return;
    }
    const message: AssistantMessage = { id: parent ?? id, role: "assistant", toolCalls: [call] };
    this.messages.push(message);
  }

  private appendArgs(id: string, delta: string): void {
    for (let i = this.messages.length - 1; i >= 0; i -= 1) {
      const message = this.messages[i]!;
      if (message.role !== "assistant") continue;
      const call = message.toolCalls?.find((c) => c.id === id);
      if (call) {
        call.function.arguments += delta;
        return;
      }
    }
    throw new SequenceError(`args for unknown tool call ${JSON.stringify(id)}`);
  }

  private activitySnapshot(id: string, activityType: string, content: Json, replace: boolean): void {
    const existing = this.find(id);
    if (existing?.role === "activity") {
      existing.activityType = activityType;
      existing.content = replace ? structuredClone(content) : mergePatch(existing.content, content);
      return;
    }
    this.messages.push({ id, role: "activity", activityType, content: structuredClone(content) });
  }
}

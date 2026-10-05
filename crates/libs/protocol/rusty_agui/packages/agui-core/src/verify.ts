// The ordering rules and chunk expansion. The mirror of the Rust
// `Verifier`: the same rules, the same expansion, checked against the
// same `fixtures/chunks.json`.

import type { Event, Role } from "./types.js";

export class SequenceError extends Error {
  override name = "SequenceError";
}

type Open = { kind: "text" | "tool" | "reasoning"; id: string };

function describe(open: Open): string {
  const noun = open.kind === "text" ? "text message" : open.kind === "tool" ? "tool call" : "reasoning message";
  return `${noun} ${JSON.stringify(open.id)}`;
}

/** A stateful checker for one run's event sequence. */
export class Verifier {
  private phase: "notStarted" | "running" | "finished" = "notStarted";
  private open: Open | undefined;
  private chunk: Open | undefined;
  private steps: string[] = [];

  /** True once RUN_FINISHED or RUN_ERROR has been accepted. */
  get finished(): boolean {
    return this.phase === "finished";
  }

  /** Accepts one event; returns the canonical events it stands for. */
  push(event: Event): Event[] {
    const out: Event[] = [];
    const expanded = this.expandChunk(event, out);
    if (expanded !== undefined) {
      for (const e of expanded) {
        this.check(e);
        out.push(e);
      }
      return out;
    }
    this.closeChunk(out);
    this.check(event);
    out.push(event);
    return out;
  }

  private meta(event: Event): Partial<Event> {
    const meta: Record<string, unknown> = {};
    for (const key of ["timestamp", "subagentRunId", "metadata", "rawEvent"] as const) {
      if (event[key] !== undefined) meta[key] = event[key];
    }
    return meta as Partial<Event>;
  }

  private chunkId(given: string | undefined, what: string): string {
    if (given !== undefined) return given;
    if (this.chunk) return this.chunk.id;
    throw new SequenceError(`${what} without an id and no chunk stream open`);
  }

  private expandChunk(event: Event, out: Event[]): Event[] | undefined {
    const meta = this.meta(event);
    const events: Event[] = [];
    switch (event.type) {
      case "TEXT_MESSAGE_CHUNK": {
        const id = this.chunkId(event.messageId, "TEXT_MESSAGE_CHUNK");
        if (!(this.chunk?.kind === "text" && this.chunk.id === id)) {
          this.closeChunk(out);
          this.chunk = { kind: "text", id };
          events.push({ ...meta, type: "TEXT_MESSAGE_START", messageId: id, role: event.role ?? ("assistant" as Role) });
        }
        if (event.delta) events.push({ ...meta, type: "TEXT_MESSAGE_CONTENT", messageId: id, delta: event.delta });
        return events;
      }
      case "TOOL_CALL_CHUNK": {
        const id = this.chunkId(event.toolCallId, "TOOL_CALL_CHUNK");
        if (!(this.chunk?.kind === "tool" && this.chunk.id === id)) {
          if (event.toolCallName === undefined) {
            throw new SequenceError("the first TOOL_CALL_CHUNK of a call needs toolCallName");
          }
          this.closeChunk(out);
          this.chunk = { kind: "tool", id };
          const start: Event = { ...meta, type: "TOOL_CALL_START", toolCallId: id, toolCallName: event.toolCallName };
          if (event.parentMessageId !== undefined) start.parentMessageId = event.parentMessageId;
          events.push(start);
        }
        if (event.delta) events.push({ ...meta, type: "TOOL_CALL_ARGS", toolCallId: id, delta: event.delta });
        return events;
      }
      case "REASONING_MESSAGE_CHUNK": {
        const id = this.chunkId(event.messageId, "REASONING_MESSAGE_CHUNK");
        if (!(this.chunk?.kind === "reasoning" && this.chunk.id === id)) {
          this.closeChunk(out);
          this.chunk = { kind: "reasoning", id };
          events.push({ ...meta, type: "REASONING_MESSAGE_START", messageId: id, role: "reasoning" });
        }
        if (event.delta === "") {
          this.chunk = undefined;
          events.push({ ...meta, type: "REASONING_MESSAGE_END", messageId: id });
        } else if (event.delta !== undefined) {
          events.push({ ...meta, type: "REASONING_MESSAGE_CONTENT", messageId: id, delta: event.delta });
        }
        return events;
      }
      default:
        return undefined;
    }
  }

  private closeChunk(out: Event[]): void {
    const chunk = this.chunk;
    if (!chunk) return;
    this.chunk = undefined;
    const end: Event =
      chunk.kind === "text"
        ? { type: "TEXT_MESSAGE_END", messageId: chunk.id }
        : chunk.kind === "tool"
          ? { type: "TOOL_CALL_END", toolCallId: chunk.id }
          : { type: "REASONING_MESSAGE_END", messageId: chunk.id };
    this.check(end);
    out.push(end);
  }

  private openNew(open: Open): void {
    if (this.open) throw new SequenceError(`cannot open ${describe(open)} while ${describe(this.open)} is open`);
    this.open = open;
  }

  private expectOpen(wanted: Open, event: string): void {
    if (!this.open) throw new SequenceError(`${event} for ${describe(wanted)} that is not open`);
    if (this.open.kind !== wanted.kind || this.open.id !== wanted.id) {
      throw new SequenceError(`${event} for ${describe(wanted)} but ${describe(this.open)} is open`);
    }
  }

  private check(event: Event): void {
    if (this.phase === "notStarted" && event.type !== "RUN_STARTED") {
      throw new SequenceError(`${event.type} before RUN_STARTED`);
    }
    if (this.phase === "finished") throw new SequenceError(`${event.type} after the run finished`);
    switch (event.type) {
      case "RUN_STARTED":
        if (this.phase === "running") throw new SequenceError("RUN_STARTED twice");
        this.phase = "running";
        break;
      case "RUN_FINISHED":
        if (this.open) throw new SequenceError(`RUN_FINISHED with ${describe(this.open)} open`);
        if (this.steps.length) throw new SequenceError(`RUN_FINISHED with step ${JSON.stringify(this.steps.at(-1))} open`);
        this.phase = "finished";
        break;
      case "RUN_ERROR":
        this.phase = "finished";
        break;
      case "STEP_STARTED":
        this.steps.push(event.stepName);
        break;
      case "STEP_FINISHED": {
        const i = this.steps.lastIndexOf(event.stepName);
        if (i < 0) throw new SequenceError(`STEP_FINISHED for step ${JSON.stringify(event.stepName)} that is not open`);
        this.steps.splice(i, 1);
        break;
      }
      case "TEXT_MESSAGE_START":
        this.openNew({ kind: "text", id: event.messageId });
        break;
      case "TEXT_MESSAGE_CONTENT":
        if (event.delta === "") throw new SequenceError("TEXT_MESSAGE_CONTENT with empty delta");
        this.expectOpen({ kind: "text", id: event.messageId }, "TEXT_MESSAGE_CONTENT");
        break;
      case "TEXT_MESSAGE_END":
        this.expectOpen({ kind: "text", id: event.messageId }, "TEXT_MESSAGE_END");
        this.open = undefined;
        break;
      case "TOOL_CALL_START":
        this.openNew({ kind: "tool", id: event.toolCallId });
        break;
      case "TOOL_CALL_ARGS":
        if (event.delta === "") throw new SequenceError("TOOL_CALL_ARGS with empty delta");
        this.expectOpen({ kind: "tool", id: event.toolCallId }, "TOOL_CALL_ARGS");
        break;
      case "TOOL_CALL_END":
        this.expectOpen({ kind: "tool", id: event.toolCallId }, "TOOL_CALL_END");
        this.open = undefined;
        break;
      case "REASONING_MESSAGE_START":
        this.openNew({ kind: "reasoning", id: event.messageId });
        break;
      case "REASONING_MESSAGE_CONTENT":
        this.expectOpen({ kind: "reasoning", id: event.messageId }, "REASONING_MESSAGE_CONTENT");
        break;
      case "REASONING_MESSAGE_END":
        this.expectOpen({ kind: "reasoning", id: event.messageId }, "REASONING_MESSAGE_END");
        this.open = undefined;
        break;
      case "TEXT_MESSAGE_CHUNK":
      case "TOOL_CALL_CHUNK":
      case "REASONING_MESSAGE_CHUNK":
        throw new Error("chunks are expanded before checking");
      default:
        if (this.open) throw new SequenceError(`${event.type} while ${describe(this.open)} is open`);
    }
  }
}

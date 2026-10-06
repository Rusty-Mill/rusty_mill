// A scripted AG-UI agent behind a fake fetch, for the bindings' tests:
// each run answers from a script that sees the RunAgentInput it was sent.
// Exported as `@rusty-mill/agui-core/testing`; not part of the API.

import { encode } from "./sse.js";
import type { Event, RunAgentInput } from "./types.js";

export type Script = (input: RunAgentInput, runIndex: number) => Event[];

export interface FakeAgent {
  fetch: typeof fetch;
  inputs: RunAgentInput[];
}

export function fakeAgent(script: Script): FakeAgent {
  const inputs: RunAgentInput[] = [];
  const fake: typeof fetch = async (_url, init) => {
    const input = JSON.parse(String(init?.body)) as RunAgentInput;
    inputs.push(input);
    const events = script(input, inputs.length - 1);
    const body = events.map(encode).join("");
    return new Response(new TextEncoder().encode(body), { status: 200, headers: { "content-type": "text/event-stream" } });
  };
  return { fetch: fake, inputs };
}

export const run = (threadId: string, runId: string, inner: Event[], result?: unknown): Event[] => [
  { type: "RUN_STARTED", threadId, runId },
  ...inner,
  { type: "RUN_FINISHED", threadId, runId, ...(result === undefined ? {} : { result: result as never }) },
];

export const say = (id: string, text: string): Event[] => [
  { type: "TEXT_MESSAGE_START", messageId: id, role: "assistant" },
  { type: "TEXT_MESSAGE_CONTENT", messageId: id, delta: text },
  { type: "TEXT_MESSAGE_END", messageId: id },
];

export const call = (id: string, parent: string, name: string, args: unknown): Event[] => [
  { type: "TOOL_CALL_START", toolCallId: id, toolCallName: name, parentMessageId: parent },
  { type: "TOOL_CALL_ARGS", toolCallId: id, delta: JSON.stringify(args) },
  { type: "TOOL_CALL_END", toolCallId: id },
];

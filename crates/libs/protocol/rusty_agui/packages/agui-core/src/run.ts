// Run an agent over fetch: POST the input, stream the events back through
// the decoder, the verifier and the reducer.

import { Reducer } from "./reduce.js";
import { CONTENT_TYPE, Decoder } from "./sse.js";
import type { Event, Json, Message, RunAgentInput, RunOutcome } from "./types.js";
import { Verifier } from "./verify.js";

export interface AgentEndpoint {
  url: string;
  headers?: Record<string, string>;
  /** Defaults to the global `fetch`. */
  fetch?: typeof fetch;
}

export interface StreamOptions {
  signal?: AbortSignal;
}

export class HttpError extends Error {
  override name = "HttpError";
  constructor(
    readonly status: number,
    readonly body: string,
  ) {
    super(`server answered ${status}: ${body}`);
  }
}

export class TransportError extends Error {
  override name = "TransportError";
}

/**
 * Starts a run and yields its events as they arrive, verified and in
 * order. Ends after RUN_FINISHED or RUN_ERROR; throws when the stream
 * closes before either, breaks the ordering rules, or is not AG-UI JSON.
 */
export async function* streamAgent(
  endpoint: AgentEndpoint,
  input: RunAgentInput,
  options: StreamOptions = {},
): AsyncGenerator<Event, void, undefined> {
  const doFetch = endpoint.fetch ?? fetch;
  const init: RequestInit = {
    method: "POST",
    headers: { "Content-Type": "application/json", Accept: CONTENT_TYPE, ...endpoint.headers },
    body: JSON.stringify(input),
  };
  if (options.signal) init.signal = options.signal;
  const response = await doFetch(endpoint.url, init);
  if (response.status !== 200) throw new HttpError(response.status, await response.text());
  const contentType = response.headers.get("content-type") ?? "";
  if (!contentType.startsWith(CONTENT_TYPE)) throw new TransportError(`expected ${CONTENT_TYPE}, got ${JSON.stringify(contentType)}`);
  if (!response.body) throw new TransportError("response has no body");

  const reader = response.body.getReader();
  const text = new TextDecoder();
  const decoder = new Decoder();
  const verifier = new Verifier();
  try {
    for (;;) {
      const { value, done } = await reader.read();
      const raw = done ? [] : decoder.feed(text.decode(value, { stream: true }));
      if (done) {
        const last = decoder.finish();
        if (last) raw.push(last);
      }
      for (const event of raw) {
        for (const canonical of verifier.push(event)) yield canonical;
      }
      if (verifier.finished) return;
      if (done) throw new TransportError("stream closed before RUN_FINISHED or RUN_ERROR");
    }
  } finally {
    reader.releaseLock();
  }
}

export interface RunResult {
  events: Event[];
  messages: Message[];
  state: Json;
  /** The RUN_FINISHED result, when the run finished. */
  result?: Json;
  outcome?: RunOutcome;
  /** The RUN_ERROR, when the run failed. */
  error?: { message: string; code?: string };
}

export interface RunOptions extends StreamOptions {
  /** Called for every canonical event, after the reducer has applied it. */
  onEvent?: (event: Event, view: Reducer) => void;
}

/** Runs an agent to completion and returns the events and the reduced view. */
export async function runAgent(endpoint: AgentEndpoint, input: RunAgentInput, options: RunOptions = {}): Promise<RunResult> {
  const view = Reducer.fromInput(input);
  const events: Event[] = [];
  const result: RunResult = { events, messages: view.messages, state: view.state };
  for await (const event of streamAgent(endpoint, input, options)) {
    view.apply(event);
    events.push(event);
    options.onEvent?.(event, view);
    if (event.type === "RUN_FINISHED") {
      if (event.result !== undefined) result.result = event.result;
      if (event.outcome !== undefined) result.outcome = event.outcome;
    } else if (event.type === "RUN_ERROR") {
      result.error = event.code === undefined ? { message: event.message } : { message: event.message, code: event.code };
    }
  }
  result.messages = view.messages;
  result.state = view.state;
  return result;
}

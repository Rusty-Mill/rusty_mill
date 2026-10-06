import { describe, expect, it } from "vitest";
import {
  applyPatch,
  Decoder,
  DecodeError,
  encode,
  mergePatch,
  parseEvent,
  PatchError,
  Reducer,
  runAgent,
  SequenceError,
  streamAgent,
  Verifier,
  type Event,
} from "../src/index.js";

const step = (stepName: string): Event => ({ type: "STEP_STARTED", stepName });
const started: Event = { type: "RUN_STARTED", threadId: "t", runId: "r" };
const finished: Event = { type: "RUN_FINISHED", threadId: "t", runId: "r" };

describe("parseEvent", () => {
  it("names the missing field and refuses unknown types", () => {
    expect(() => parseEvent({ type: "TEXT_MESSAGE_CONTENT", messageId: "m" })).toThrow(DecodeError);
    expect(() => parseEvent({ type: "TEXT_MESSAGE_CONTENT", messageId: "m" })).toThrow(/missing "delta"/);
    expect(() => parseEvent({ type: "NOPE" })).toThrow(/unknown event type/);
    expect(() => parseEvent({ type: "STATE_DELTA", delta: {} })).toThrow(/not an array/);
    expect(parseEvent({ type: "TEXT_MESSAGE_END", messageId: "m", future: 1 })).toMatchObject({ future: 1 });
  });
});

describe("SSE", () => {
  it("encodes one data frame", () => {
    expect(encode(step("a"))).toBe('data: {"type":"STEP_STARTED","stepName":"a"}\n\n');
  });

  it("decodes split frames, comments, multi-line data and every line ending", () => {
    const decoder = new Decoder();
    const frame = encode(step("a"));
    expect(decoder.feed(frame.slice(0, 10))).toEqual([]);
    expect(decoder.feed(frame.slice(10))).toEqual([step("a")]);
    const mixed = ': keep-alive\r\nid: 7\r\nevent: message\r\ndata: {"type":"STEP_STARTED",\r\ndata: "stepName":"b"}\r\n\r\n';
    expect(decoder.feed(mixed)).toEqual([step("b")]);
    expect(decoder.feed('data: {"type":"STEP_STARTED","stepName":"c"}\r\r')).toEqual([step("c")]);
    expect(decoder.feed(encode(step("d")) + encode(step("e")))).toEqual([step("d"), step("e")]);
    decoder.feed('data: {"type":"STEP_FINISHED","stepName":"z"}');
    expect(decoder.finish()).toEqual({ type: "STEP_FINISHED", stepName: "z" });
    expect(decoder.finish()).toBeUndefined();
  });

  it("a bad frame throws without poisoning the decoder", () => {
    const decoder = new Decoder();
    expect(() => decoder.feed("data: nope\n\n")).toThrow();
    expect(decoder.feed(encode(step("ok")))).toEqual([step("ok")]);
  });
});

describe("JSON patch", () => {
  it("applies RFC 6902 Appendix A cases and is atomic", () => {
    expect(applyPatch({ foo: "bar" }, [{ op: "add", path: "/baz", value: "qux" }])).toEqual({ baz: "qux", foo: "bar" });
    expect(applyPatch({ foo: ["bar", "baz"] }, [{ op: "add", path: "/foo/1", value: "qux" }])).toEqual({ foo: ["bar", "qux", "baz"] });
    expect(applyPatch({ foo: ["all", "grass", "cows", "eat"] }, [{ op: "move", from: "/foo/1", path: "/foo/3" }])).toEqual({
      foo: ["all", "cows", "eat", "grass"],
    });
    expect(applyPatch({ foo: ["bar"] }, [{ op: "add", path: "/foo/-", value: ["abc", "def"] }])).toEqual({ foo: ["bar", ["abc", "def"]] });
    expect(applyPatch({ "/": 9, "~1": 10 }, [{ op: "test", path: "/~01", value: 10 }])).toEqual({ "/": 9, "~1": 10 });
    expect(() => applyPatch({ baz: "qux" }, [{ op: "test", path: "/baz", value: "bar" }])).toThrow(PatchError);
    expect(() => applyPatch({ foo: "bar" }, [{ op: "add", path: "/baz/bat", value: "qux" }])).toThrow(/path not found/);
    expect(() => applyPatch({ foo: ["bar", "baz"] }, [{ op: "add", path: "/foo/5", value: 1 }])).toThrow(/invalid array index/);
    const original = { a: 1, b: [1, 2] };
    expect(() =>
      applyPatch(original, [
        { op: "replace", path: "/a", value: 2 },
        { op: "remove", path: "/missing" },
      ]),
    ).toThrow();
    expect(original).toEqual({ a: 1, b: [1, 2] });
    expect(applyPatch({ a: 1 }, [{ op: "replace", path: "", value: [1] }])).toEqual([1]);
  });

  it("merges per RFC 7386", () => {
    expect(mergePatch({ a: { b: "c" } }, { a: { b: "d", c: null } })).toEqual({ a: { b: "d" } });
    expect(mergePatch({ a: "foo" }, null)).toBeNull();
    expect(mergePatch([1, 2], { a: "b", c: null })).toEqual({ a: "b" });
  });
});

describe("Verifier", () => {
  const run = (events: Event[]) => {
    const v = new Verifier();
    return events.flatMap((e) => v.push(e));
  };

  it("names the rule it enforces", () => {
    const text = (id: string, delta: string): Event => ({ type: "TEXT_MESSAGE_CONTENT", messageId: id, delta });
    const open: Event = { type: "TEXT_MESSAGE_START", messageId: "m", role: "assistant" };
    const cases: [Event[], RegExp][] = [
      [[text("m", "x")], /before RUN_STARTED/],
      [[started, started], /twice/],
      [[started, finished, finished], /after the run finished/],
      [[started, text("m", "x")], /not open/],
      [[started, open, text("m", "")], /empty delta/],
      [[started, open, text("other", "x")], /but text message "m" is open/],
      [[started, open, { type: "TOOL_CALL_START", toolCallId: "c", toolCallName: "f" }], /cannot open tool call/],
      [[started, open, finished], /RUN_FINISHED with text message/],
      [[started, step("s"), finished], /step "s" open/],
      [[started, { type: "STEP_FINISHED", stepName: "s" }], /not open/],
      [[started, open, { type: "STATE_SNAPSHOT", snapshot: null }], /STATE_SNAPSHOT while text message/],
      [[started, { type: "TEXT_MESSAGE_CHUNK", delta: "x" }], /no chunk stream open/],
      [[started, { type: "TOOL_CALL_CHUNK", toolCallId: "c" }], /needs toolCallName/],
    ];
    for (const [events, pattern] of cases) {
      expect(() => run(events), JSON.stringify(events)).toThrow(SequenceError);
      expect(() => run(events)).toThrow(pattern);
    }
  });

  it("RUN_ERROR closes the run from anywhere and keeps meta on expanded chunks", () => {
    const out = run([
      started,
      { type: "TEXT_MESSAGE_CHUNK", messageId: "a", delta: "x", subagentRunId: "s" },
      { type: "RUN_ERROR", message: "boom" },
    ]);
    expect(out.map((e) => e.type)).toEqual(["RUN_STARTED", "TEXT_MESSAGE_START", "TEXT_MESSAGE_CONTENT", "TEXT_MESSAGE_END", "RUN_ERROR"]);
    expect(out[1]).toMatchObject({ subagentRunId: "s" });
  });
});

describe("Reducer", () => {
  it("fails a bad delta and leaves state alone", () => {
    const reducer = new Reducer([], { a: 1 });
    expect(() => reducer.apply({ type: "STATE_DELTA", delta: [{ op: "remove", path: "/missing" }] })).toThrow(PatchError);
    expect(reducer.state).toEqual({ a: 1 });
  });

  it("gives an orphan tool call its own assistant message", () => {
    const reducer = new Reducer();
    reducer.apply({ type: "TOOL_CALL_START", toolCallId: "c", toolCallName: "f" });
    expect(reducer.messages).toEqual([{ id: "c", role: "assistant", toolCalls: [{ id: "c", type: "function", function: { name: "f", arguments: "" } }] }]);
  });
});

describe("runAgent over a fake fetch", () => {
  const sse = (events: Event[]) => events.map(encode).join("");
  const respond = (body: string, init: ResponseInit = {}) =>
    new Response(new TextEncoder().encode(body), { status: 200, headers: { "content-type": "text/event-stream" }, ...init });

  it("verifies, reduces and reports the result", async () => {
    const events: Event[] = [
      started,
      { type: "STATE_SNAPSHOT", snapshot: { n: 1 } },
      { type: "TEXT_MESSAGE_CHUNK", messageId: "m", delta: "hi" },
      { type: "RUN_FINISHED", threadId: "t", runId: "r", result: "done", outcome: { type: "success" } },
    ];
    let request: { url: string; body: string } | undefined;
    const fakeFetch: typeof fetch = async (url, init) => {
      request = { url: String(url), body: String(init?.body) };
      return respond(sse(events));
    };
    const result = await runAgent({ url: "http://agent/run", fetch: fakeFetch }, { threadId: "t", runId: "r", messages: [{ id: "u", role: "user", content: "hello" }] });
    expect(request?.url).toBe("http://agent/run");
    expect(JSON.parse(request!.body)).toMatchObject({ threadId: "t", runId: "r" });
    expect(result.events.map((e) => e.type)).toEqual(["RUN_STARTED", "STATE_SNAPSHOT", "TEXT_MESSAGE_START", "TEXT_MESSAGE_CONTENT", "TEXT_MESSAGE_END", "RUN_FINISHED"]);
    expect(result.state).toEqual({ n: 1 });
    expect(result.messages).toEqual([
      { id: "u", role: "user", content: "hello" },
      { id: "m", role: "assistant", content: "hi" },
    ]);
    expect(result.result).toBe("done");
    expect(result.outcome).toEqual({ type: "success" });
    expect(result.error).toBeUndefined();
  });

  it("reports RUN_ERROR as a result, and transport faults as errors", async () => {
    const failing = await runAgent({ url: "x", fetch: async () => respond(sse([started, { type: "RUN_ERROR", message: "boom", code: "E" }])) }, { threadId: "t", runId: "r" });
    expect(failing.error).toEqual({ message: "boom", code: "E" });

    await expect(runAgent({ url: "x", fetch: async () => respond("nope", { status: 404 }) }, { threadId: "t", runId: "r" })).rejects.toThrow(/404/);
    await expect(runAgent({ url: "x", fetch: async () => respond("{}", { headers: { "content-type": "application/json" } }) }, { threadId: "t", runId: "r" })).rejects.toThrow(/expected text\/event-stream/);
    await expect(runAgent({ url: "x", fetch: async () => respond(sse([started])) }, { threadId: "t", runId: "r" })).rejects.toThrow(/closed before/);
    const events: Event[] = [];
    await expect(
      (async () => {
        for await (const e of streamAgent({ url: "x", fetch: async () => respond(sse([started, { type: "TEXT_MESSAGE_CONTENT", messageId: "ghost", delta: "x" }])) }, { threadId: "t", runId: "r" })) events.push(e);
      })(),
    ).rejects.toThrow(SequenceError);
    expect(events.map((e) => e.type)).toEqual(["RUN_STARTED"]);
  });
});

// The fixtures shared with the Rust crate: every event type parses and
// re-encodes unchanged, chunk sequences expand identically, and whole
// runs reduce to the same messages and state.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { parseEvent, Reducer, Verifier, type Event, type RunAgentInput } from "../src/index.js";

const FIXTURES = resolve(import.meta.dirname, "../../../fixtures");
const load = <T>(name: string): T => JSON.parse(readFileSync(resolve(FIXTURES, name), "utf8")) as T;

describe("fixtures/events.json", () => {
  it("every sample parses and survives a JSON round trip unchanged", () => {
    const samples = load<unknown[]>("events.json");
    expect(samples.length).toBeGreaterThanOrEqual(35);
    for (const sample of samples) {
      const event = parseEvent(sample);
      expect(JSON.parse(JSON.stringify(event))).toEqual(sample);
    }
  });
});

describe("fixtures/chunks.json", () => {
  const cases = load<{ name: string; input: unknown[]; expected: unknown[] }[]>("chunks.json");
  for (const c of cases) {
    it(c.name, () => {
      const verifier = new Verifier();
      const out: Event[] = [];
      for (const raw of c.input) out.push(...verifier.push(parseEvent(raw)));
      expect(out).toEqual(c.expected);
    });
  }
});

describe("fixtures/runs.json", () => {
  const cases = load<{ name: string; input: RunAgentInput; events: unknown[]; messages: unknown[]; state: unknown }[]>("runs.json");
  for (const c of cases) {
    it(c.name, () => {
      const reducer = Reducer.fromInput(c.input);
      const verifier = new Verifier();
      for (const raw of c.events) {
        for (const event of verifier.push(parseEvent(raw))) reducer.apply(event);
      }
      expect(reducer.messages).toEqual(c.messages);
      expect(reducer.state).toEqual(c.state);
    });
  }
});

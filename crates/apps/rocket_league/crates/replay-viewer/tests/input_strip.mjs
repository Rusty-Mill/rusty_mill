#!/usr/bin/env node
// Unit test for the viewer's input-strip helpers (ADR-0085 b6).
//
// Extracts the pure block between INPUT_STRIP_BEGIN and INPUT_STRIP_END from
// the page template in src/render.rs and checks that gaps in a car's input
// (a candidate frame that did not match, or a frame without input) stay gaps:
// analog lines break and button bars stop at the next frame.
//
//   node crates/apps/rocket_league/crates/replay-viewer/tests/input_strip.mjs

import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';

const src = readFileSync(new URL('../src/render.rs', import.meta.url), 'utf8');
const block = src.match(/\/\/ INPUT_STRIP_BEGIN[\s\S]*?\/\/ INPUT_STRIP_END/);
assert.ok(block, 'INPUT_STRIP block not found in render.rs');
const { inputSamples, axisRuns, buttonSpans } =
  new Function(block[0] + '\nreturn { inputSamples, axisRuns, buttonSpans };')();

const JUMP = 1;
const inp = (throttle, buttons, pitch = 0) => ({ axes: [throttle, 0, pitch, 0, 0], buttons });
const car = (pri, input) => (input === undefined ? { pri } : { pri, input });
// A mixed-rate candidate (pri -1): matched at t = 0, missing at .1, matched at
// .2; at .3 it is present without input; at .4 it is back.
const frames = [
  { t: 0.0, cars: [car(0, inp(1, JUMP)), car(-1, inp(1, JUMP))] },
  { t: 0.1, cars: [car(0, inp(1, JUMP))] },
  { t: 0.2, cars: [car(0, inp(0, 0)), car(-1, inp(0, JUMP))] },
  { t: 0.3, cars: [car(0, inp(0, 0)), car(-1)] },
  { t: 0.4, cars: [car(0, inp(-1, 0, null)), car(-1, inp(-1, 0))] },
];

const sim = inputSamples(frames, -1);
assert.deepEqual(sim.map(([t, i]) => [t, i !== null]), [[0, true], [0.1, false], [0.2, true], [0.3, false], [0.4, true]]);

// Analog: every gap ends the line, so no segment bridges a missing frame.
assert.deepEqual(axisRuns(sim, 0), [[[0, 1]], [[0.2, 0]], [[0.4, -1]]]);
// The recording has no gaps: one line, except where an axis is unrecoverable.
const rec = inputSamples(frames, 0);
assert.deepEqual(axisRuns(rec, 0).map(r => r.length), [5]);
assert.deepEqual(axisRuns(rec, 2).map(r => r.length), [4]);

// Buttons: a press lasts to the next frame, never to the next surviving sample.
assert.deepEqual(buttonSpans(sim, JUMP), [[0, 0.1], [0.2, 0.3]]);
assert.deepEqual(buttonSpans(rec, JUMP), [[0, 0.1], [0.1, 0.2]]);
// A press on the last frame is a zero-length span, drawn 1 px wide.
assert.deepEqual(buttonSpans([[0.5, inp(0, JUMP)]], JUMP), [[0.5, 0.5]]);
// A car that never appears has no runs and no spans.
const none = inputSamples(frames, 7);
assert.deepEqual([axisRuns(none, 0), buttonSpans(none, JUMP)], [[], []]);

console.log('OK: input strip keeps gaps');

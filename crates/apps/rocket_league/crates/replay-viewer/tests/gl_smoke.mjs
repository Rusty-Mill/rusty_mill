#!/usr/bin/env node
// Headless-GL smoke test for the offline replay viewer.
//
// Loads a self-contained viewer HTML (generate with `replay-viewer --offline`),
// renders a few frames under software WebGL (swiftshader), and fails if any page
// error occurred or the render loop never ticked. This exercises the JS + three.js
// path that the Rust tests cannot.
//
//   node viewer/tests/gl_smoke.mjs <viewer.html>
//
// Requires puppeteer (`npm i puppeteer`). The viewer increments
// `window.__rendered` each animation frame; we assert it advanced with no errors.

import puppeteer from 'puppeteer';

const file = process.argv[2];
if (!file) {
  console.error('usage: gl_smoke.mjs <viewer.html>');
  process.exit(2);
}

const browser = await puppeteer.launch({
  headless: true,
  args: [
    '--no-sandbox',
    '--enable-unsafe-swiftshader',
    '--use-gl=angle',
    '--use-angle=swiftshader',
    '--ignore-gpu-blocklist',
  ],
});

let ok = false;
try {
  const page = await browser.newPage();
  await page.setViewport({ width: 1024, height: 640 });
  const errors = [];
  page.on('pageerror', (e) => errors.push('pageerror: ' + e.message));
  page.on('console', (m) => { if (m.type() === 'error') errors.push('console: ' + m.text()); });
  page.on('requestfailed', (r) => errors.push('requestfailed: ' + r.url().slice(0, 80)));

  await page.goto('file://' + file, { waitUntil: 'load', timeout: 60000 });
  // Wait until the render loop has ticked several times (three.js loaded + WebGL ok).
  await page.waitForFunction('window.__rendered > 3', { timeout: 20000 });
  const rendered = await page.evaluate(() => window.__rendered);

  if (errors.length) throw new Error('page errors:\n  ' + errors.join('\n  '));
  console.log(`OK: rendered ${rendered} frames, no page errors`);
  ok = true;
} catch (e) {
  console.error('FAIL: ' + e.message);
} finally {
  await browser.close();
}
process.exit(ok ? 0 : 1);

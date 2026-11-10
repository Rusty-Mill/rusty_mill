// Browser check of the board over a real rusty_tick: changes made in rusty_tick reach an open page
// through the incremental poll (S2, #1431), and a comment made there is an event.
//
//   RUSTY_TICK_TOKEN=<16+ chars> rusty_tick --data-dir "$(mktemp -d)" --addr 127.0.0.1:8791 &
//   RUSTY_TICK_TOKEN=... rusty_mill_board --http 127.0.0.1:8799 --tick-url http://127.0.0.1:8791 &
//   RUSTY_TICK_TOKEN=... node tests/ui/live_tick.mjs "$(npm root -g)/playwright" http://127.0.0.1:8799 http://127.0.0.1:8791
//
// Needs Playwright and a Chromium (set CHROMIUM=/path if it is not at /opt/pw-browsers/chromium).
import { createRequire } from "node:module";
import { randomBytes } from "node:crypto";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const BOARD = process.argv[3] ?? "http://127.0.0.1:8799";
const TICK = process.argv[4] ?? "http://127.0.0.1:8791";
const TOKEN = process.env.RUSTY_TICK_TOKEN;

const api = async (op, args = {}) => {
  const r = await fetch(`${BOARD}/api/${op}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "nano", role: "human", ...args }) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const tick = async (method, path, body) => {
  const r = await fetch(`${TICK}/api/v1${path}`, { method, headers: { "content-type": "application/json", authorization: `Bearer ${TOKEN}` }, body: body && JSON.stringify(body) });
  if (!r.ok) throw new Error(`${method} ${path}: ${r.status} ${await r.text()}`);
  return r.json();
};
/** A UUIDv7, as rusty_tick's own web UI mints comment ids. */
const v7 = () => {
  const b = randomBytes(16);
  const ms = BigInt(Date.now());
  for (let i = 0; i < 6; i++) b[i] = Number((ms >> BigInt(8 * (5 - i))) & 0xffn);
  b[6] = (b[6] & 0x0f) | 0x70;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = b.toString("hex");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const failures = [];
const run = randomBytes(3).toString("hex"); // titles unique per run, so a rerun on the same tick still checks one card
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 700 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));

const card = (await api("card_add", { title: `Live card ${run}` })).card;
await page.goto(BOARD);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");
check(await page.locator(".card", { hasText: `Live card ${run}` }).count() === 1, "the card is on the page");

// 1. The owner renames the task in rusty_tick: the open page shows it within two polls.
await tick("PATCH", `/tasks/${card.id}`, { title: `Renamed in tick ${run}` });
await sleep(3500);
check(await page.locator(".card", { hasText: `Renamed in tick ${run}` }).count() === 1, "a rename made in rusty_tick reaches the open page");

// 2. A task tagged `agent` in rusty_tick joins the board on the open page.
const { listId } = await tick("GET", `/tasks/${card.id}`);
await tick("POST", "/tasks", { listId, title: `Made in tick ${run}`, tags: ["agent"] });
await sleep(3500);
check(await page.locator(".card", { hasText: `Made in tick ${run}` }).count() === 1, "a task tagged agent in rusty_tick appears");

// 3. A comment added in rusty_tick is an event for pollers.
const { cursor } = await api("board_events", { tail: true });
await tick("PUT", `/docs/comment/${v7()}`, { v: 1, taskId: card.id, text: "from the phone", createdMs: Date.now() });
const polled = await api("board_events", { after: cursor });
check(polled.events.some((e) => e.kind === "commented" && e.card === card.id), "a comment made in rusty_tick is a commented event");
check(polled.resync === false, "a cursor from this board needs no resync");

check(errors.length === 0, `no page errors (${errors.join("; ")})`);
await browser.close();
if (failures.length) { console.log(`${failures.length} failed`); process.exit(1); }
console.log("all passed");

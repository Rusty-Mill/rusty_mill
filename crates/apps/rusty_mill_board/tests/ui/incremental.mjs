// Browser check of the incremental refresh (S2b, #1431): after the first read, the open page reads
// only the cards that changed, one card_get each, whether another actor or this page changed them,
// and reads the board again only when many cards changed at once.
//
//   rusty_mill_board --http 127.0.0.1:8797 &           # open board, loopback
//   node tests/ui/incremental.mjs "$(npm root -g)/playwright" http://127.0.0.1:8797
//
// Needs Playwright and a Chromium (set CHROMIUM=/path if it is not at /opt/pw-browsers/chromium).
import { createRequire } from "node:module";
import { randomBytes } from "node:crypto";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const BASE = process.argv[3] ?? "http://127.0.0.1:8797";

const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "other", role: "human", ...args }) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };
const run = randomBytes(3).toString("hex"); // titles unique per run, so a rerun on the same board still checks one card

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 700 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
const ops = [];
page.on("request", (r) => { const m = r.url().match(/\/api\/(\w+)$/); if (m) ops.push(m[1]); });
const since = (n) => ops.slice(n);
const shown = (title) => page.locator(".card", { hasText: title }).count();

const first = (await api("card_add", { title: `First ${run}` })).card;
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");
check(await shown(`First ${run}`) === 1, "the card is on the page");
check(ops.filter((o) => o === "board_list").length === 1, "the page reads the board once on load");

// 1. Another actor edits a card: the page reads that card only.
let mark = ops.length;
await api("card_update", { id: first.id, title: `Edited ${run}` });
await sleep(2500);
check(await shown(`Edited ${run}`) === 1, "an edit by another actor reaches the page");
check(!since(mark).includes("board_list") && since(mark).filter((o) => o === "card_get").length === 1, `an edit costs one card_get, no board_list (${since(mark).join(",")})`);

// 2. Another actor adds a card: it joins the page the same way.
mark = ops.length;
await api("card_add", { title: `Added ${run}` });
await sleep(2500);
check(await shown(`Added ${run}`) === 1, "a card added by another actor appears");
check(!since(mark).includes("board_list"), "a new card needs no board_list");

// 3. This page adds a card: its own write is read back the same way.
mark = ops.length;
await page.fill("#title", `Mine ${run}`);
await page.click("#add");
await page.locator(".card", { hasText: `Mine ${run}` }).waitFor({ timeout: 5000 });
check(!since(mark).includes("board_list"), `this page's own write needs no board_list (${since(mark).join(",")})`);

// 4. Many cards change between two polls: one board_list instead of a card_get each. The page's
// polls fail while the cards are added (as on a network blip), so the next one sees all 60.
await page.route("**/api/board_events", (r) => r.abort());
for (let i = 0; i < 60; i++) await api("card_add", { title: `Bulk ${run} ${i}` });
await sleep(1600);
mark = ops.length;
await page.unroute("**/api/board_events");
await sleep(2500);
check(await page.locator(".card", { hasText: `Bulk ${run}` }).count() === 60, "60 new cards all appear");
check(since(mark).filter((o) => o === "board_list").length === 1 && !since(mark).includes("card_get"), "many changes read the board once");

check(errors.length === 0, `no page errors (${errors.join("; ")})`);
await browser.close();
if (failures.length) { console.log(`${failures.length} failed`); process.exit(1); }
console.log("all passed");

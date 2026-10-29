// Browser check of card priority: the badge, the column order, and setting it from the dialog.
//
//   rusty_mill_board --http 127.0.0.1:8798 &           # open board, loopback
//   node tests/ui/priority.mjs "$(npm root -g)/playwright" http://127.0.0.1:8798
//
// Needs Playwright and a Chromium (set CHROMIUM=/path if it is not at /opt/pw-browsers/chromium).
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const BASE = process.argv[3] ?? "http://127.0.0.1:8798";

const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "other", role: "human", ...args }) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 700 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));

await api("card_add", { title: "Plain card" });
const urgent = (await api("card_add", { title: "Urgent card" })).card;
await api("card_plan", { id: urgent.id, priority: 5 });
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");
const backlog = page.locator('.col[data-col="backlog"] .card b');

// 1. A prioritised card shows its badge and sorts first, though it was added last.
check((await backlog.allInnerTexts())[0] === "Urgent card", "the high-priority card is first in its column");
check(await page.locator(".card", { hasText: "Urgent card" }).locator(".tag.p5", { hasText: "High" }).count() === 1, "it shows a High badge");
check(await page.locator(".card", { hasText: "Plain card" }).locator(".tag.p5, .tag.p3, .tag.p1").count() === 0, "a card with no priority shows no badge");

// 2. The dialog sets it, and the board follows.
await page.locator(".card", { hasText: "Plain card" }).click();
const pick = page.locator('#dlg select[aria-label="priority"]');
check((await pick.inputValue()) === "0", "the dialog shows the current priority");
await pick.selectOption("3");
await sleep(500);
const plain = (await api("board_list")).cards.find((c) => c.title === "Plain card");
check(plain.priority === 3, "choosing Medium in the dialog sets priority 3");
check(await page.locator(".card", { hasText: "Plain card" }).locator(".tag.p3", { hasText: "Medium" }).count() === 1, "the board shows the Medium badge");

// 3. A change made elsewhere reaches the open dialog.
await page.locator("#dlg").locator("textarea").focus();
await api("card_plan", { id: plain.id, priority: 1 });
await sleep(3500);   // > two polls
check((await pick.inputValue()) === "1", "a remote priority change updates the open dialog");

check(errors.length === 0, `no page errors (${errors.join("; ")})`);
await browser.close();
if (failures.length) { console.error(`${failures.length} failed`); process.exit(1); }
console.log("all passed");

// Browser check of the archive: Done cards leave the board when archived, the Archive view lists
// and searches them, and Restore and the dialog's Archive button move them back and forth.
//
//   rusty_mill_board --http 127.0.0.1:8798 &           # open board, loopback
//   node tests/ui/archive.mjs "$(npm root -g)/playwright" http://127.0.0.1:8798
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
page.on("dialog", (d) => d.accept());

const old = (await api("card_add", { title: "Old shipped", done: true })).card;
await api("card_add", { title: "Fresh shipped", done: true });
await api("card_archive", { id: old.id });
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");
const done = page.locator('.col[data-col="done"]');

// 1. An archived card is off the Done column, which links to the Archive.
check(await done.locator(".card", { hasText: "Old shipped" }).count() === 0, "an archived card is not in Done");
check(await done.locator(".card", { hasText: "Fresh shipped" }).count() === 1, "an unarchived Done card is");
const link = done.locator(".colbar button", { hasText: "1 archived" });
check(await link.count() === 1, "Done's heading says one card is archived");

// 2. The Archive view lists and searches it, and Restore brings it back.
await link.click();
const arch = page.locator("main.archive");
check(await arch.locator(".card", { hasText: "Old shipped" }).count() === 1, "the Archive view lists it");
await arch.locator("input").fill("fresh");
check(await arch.locator(".card").count() === 0, "a search with no match lists nothing");
await arch.locator("input").fill("#" + old.number);
check(await arch.locator(".card").count() === 1, "searching by number finds it");
await arch.locator(".card button", { hasText: "Restore" }).click();
await sleep(500);
check((await api("card_get", { id: old.id })).card.archived_ms == null, "Restore clears it");

// 3. Back on the board, the dialog archives a Done card.
await page.click("#archive");
await done.locator(".card", { hasText: "Fresh shipped" }).click();
await page.locator("#dlg button", { hasText: "Archive" }).click();
await sleep(500);
check(await done.locator(".card", { hasText: "Fresh shipped" }).count() === 0, "the dialog's Archive takes it off the board");
check(await page.locator("#dlg button", { hasText: "Restore" }).count() === 1, "and the button turns into Restore");
await page.locator("#dlg button", { hasText: "Close" }).click();

// 4. Archive all empties the column.
await done.locator(".colbar button", { hasText: "Archive all" }).click();
await sleep(500);
check(await done.locator(".card").count() === 0, "Archive all leaves Done empty");
check((await api("board_list", { archived: "only" })).cards.length === 2, "both cards are in the archive");

check(errors.length === 0, `no page errors (${errors.join("; ")})`);
await browser.close();
if (failures.length) { console.error(`${failures.length} failed`); process.exit(1); }
console.log("all passed");

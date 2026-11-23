// Browser check of #1576: "Move to" in the card dialog moves a card with card_move, no drag needed.
//
//   rusty_mill_board --http 127.0.0.1:8796 &           # open board, loopback
//   node tests/ui/move_to.mjs "$(npm root -g)/playwright" http://127.0.0.1:8796
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const BASE = process.argv[3] ?? "http://127.0.0.1:8796";

const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "other", role: "human", ...args }) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await (await browser.newContext({ viewport: { width: 1300, height: 800 } })).newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("dialog", (d) => d.accept());

const card = (await api("card_add", { title: "Move test" })).card;
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");
await page.locator(".card", { hasText: "Move test" }).click();
const mv = page.locator("#dlg").getByLabel("move to");
const column = async () => (await api("card_get", { id: card.id })).card.column;

const options = await mv.locator("option").allTextContents();
check(options[0] === "Move to…" && options.some((o) => o.startsWith("Ready")), "a Backlog card offers the columns it may move to: " + options.join(" | "));
await mv.selectOption({ label: options.find((o) => o.startsWith("Ready")) });
await page.waitForFunction(async (id) => (await (await fetch("/api/card_get", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "o", role: "human", id }) })).json()).card.column === "ready", card.id);
check((await column()) === "ready", "choosing Ready moves the card through card_move");
check((await mv.inputValue()) === "", "the select returns to its prompt");
await page.waitForTimeout(500);
check((await mv.locator("option").allTextContents()).join() !== options.join(), "its options follow the card's new column");
check(errors.length === 0, "no page errors: " + errors.join("; "));
await browser.close();
process.exit(failures.length ? 1 : 0);

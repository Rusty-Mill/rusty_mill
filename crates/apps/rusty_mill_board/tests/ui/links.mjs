// Browser check of #1575: the card dialog sets and clears waits-on, related and superseded-by through
// card_plan, shown by card number.
//
//   rusty_mill_board --http 127.0.0.1:8796 &           # open board, loopback
//   node tests/ui/links.mjs "$(npm root -g)/playwright" http://127.0.0.1:8796
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

const a = (await api("card_add", { title: "Links alpha" })).card;
const b = (await api("card_add", { title: "Links beta" })).card;
const c = (await api("card_add", { title: "Links gamma" })).card;
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");
await page.locator(".card", { hasText: "Links alpha" }).click();
const dialog = page.locator("#dlg");
const ids = async (field) => (await api("card_get", { id: a.id })).card[field] ?? [];

await dialog.getByLabel("waits on").fill(`#${b.number}, ${c.number}`);
await dialog.getByLabel("waits on").blur();
await page.waitForFunction(async (n) => (await (await fetch("/api/card_get", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "o", role: "human", id: "#" + n }) })).json()).card.deps?.length === 2, a.number);
check((await ids("deps")).sort().join() === [b.id, c.id].sort().join(), "waits on takes numbers and stores both cards");
check((await dialog.getByLabel("waits on").inputValue()).split(", ").sort().join() === [`#${b.number}`, `#${c.number}`].sort().join(), "the field shows card numbers");

await dialog.getByLabel("related").fill(`#${c.number}`);
await dialog.getByLabel("related").blur();
await page.waitForFunction(async (n) => (await (await fetch("/api/card_get", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "o", role: "human", id: "#" + n }) })).json()).card.related?.length === 1, a.number);
check((await ids("related"))[0] === c.id, "related is stored");

await dialog.getByLabel("superseded by").fill(`#${b.number}`);
await dialog.getByLabel("superseded by").blur();
await page.waitForFunction(async (n) => (await (await fetch("/api/card_get", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "o", role: "human", id: "#" + n }) })).json()).card.superseded_by?.length === 1, a.number);
check((await ids("superseded_by"))[0] === b.id, "superseded by is stored");

await dialog.getByLabel("waits on").fill("");
await dialog.getByLabel("waits on").blur();
await page.waitForFunction(async (n) => ((await (await fetch("/api/card_get", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "o", role: "human", id: "#" + n }) })).json()).card.deps ?? []).length === 0, a.number);
check((await ids("deps")).length === 0, "an empty field clears the list");

await dialog.getByLabel("related").fill("#99999");
await dialog.getByLabel("related").blur();
await page.waitForSelector("#toast", { state: "visible" });
check((await ids("related"))[0] === c.id, "an unknown card number is refused and the list is kept");
check(errors.length === 0, "no page errors: " + errors.join("; "));
await browser.close();
process.exit(failures.length ? 1 : 0);

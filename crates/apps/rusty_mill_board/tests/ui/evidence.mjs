// Browser check of #1574: the card dialog adds evidence through card_evidence, lists it, and the
// card shows an evidence count.
//
//   rusty_mill_board --http 127.0.0.1:8796 &           # open board, loopback
//   node tests/ui/evidence.mjs "$(npm root -g)/playwright" http://127.0.0.1:8796
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
const page = await (await browser.newContext({ viewport: { width: 1300, height: 700 } })).newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));

const card = (await api("card_add", { title: "Evidence test" })).card;
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");
await page.locator(".card", { hasText: "Evidence test" }).click();
const dialog = page.locator("#dlg");
const pr = "https://github.com/rusty-mill/rusty_mill/pull/1";

await dialog.getByLabel("evidence link").fill("not a link");
await dialog.getByRole("button", { name: "Add evidence" }).click();
await page.waitForSelector("#toast", { state: "visible" });
check(((await api("card_get", { id: card.id })).card.evidence ?? []).length === 0, "a malformed link is refused with the op's error");

await dialog.getByLabel("evidence link").fill(pr);
await dialog.getByRole("button", { name: "Add evidence" }).click();
await dialog.locator(".evidence a", { hasText: pr }).waitFor();
check((await api("card_get", { id: card.id })).card.evidence[0] === pr, "the link is stored through card_evidence");
check((await dialog.getByLabel("evidence link").inputValue()) === "", "the box clears after adding");
// A slow board: a link typed while the first is sent is kept, and a double click adds it once.
await page.route("**/api/card_evidence", async (route) => { await new Promise((r) => setTimeout(r, 600)); await route.continue(); });
const sent = [];
page.on("request", (r) => r.url().endsWith("/api/card_evidence") && sent.push(r));
const second = "https://github.com/rusty-mill/rusty_mill/pull/2";
await dialog.getByLabel("evidence link").fill(second);
await dialog.getByRole("button", { name: "Add evidence" }).click();
await dialog.getByRole("button", { name: "Add evidence" }).click({ force: true });
await dialog.getByLabel("evidence link").fill("https://github.com/rusty-mill/rusty_mill/pull/3");
await dialog.locator(".evidence a", { hasText: second }).waitFor({ timeout: 5000 });
await page.waitForTimeout(300);
check((await dialog.getByLabel("evidence link").inputValue()).endsWith("/pull/3"), "a link typed while the request ran is kept");
check(sent.length === 1, "a double click sends one card_evidence request: " + sent.length);
await page.unroute("**/api/card_evidence");
await dialog.getByRole("button", { name: "Close" }).click();
check(await page.locator(".card", { hasText: "Evidence test" }).getByText("evidence 2").count() === 1, "the card face shows its evidence count");
check(errors.length === 0, "no page errors: " + errors.join("; "));
await browser.close();
process.exit(failures.length ? 1 : 0);

// Browser check of the typed card fields (#1581): summary on the card face, acceptance, scope and why in
// the dialog (why collapsed), editing with a byte counter, an over-cap refusal, and empty fields left out.
//
//   rusty_mill_board --http 127.0.0.1:8796 &           # open board, loopback
//   node tests/ui/typed_fields.mjs "$(npm root -g)/playwright" http://127.0.0.1:8796
//
// Needs Playwright and a Chromium (set CHROMIUM=/path if it is not at /opt/pw-browsers/chromium).
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

const full = (await api("card_add", { title: "Typed full", summary: "One line.", acceptance: "It works.", scope: "Only this.", why: "Because." })).card;
await api("card_add", { title: "Typed bare" });
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");

const face = page.locator(".card", { hasText: "Typed full" });
check((await face.locator(".sum").textContent()) === "One line.", "the card face shows the summary");
check((await page.locator(".card", { hasText: "Typed bare" }).locator(".sum").count()) === 0, "a card with no summary shows no summary line");

await face.click();
const dlg = page.locator("#dlg");
check((await dlg.getByLabel("acceptance", { exact: true }).inputValue()) === "It works.", "the dialog shows acceptance");
check((await dlg.getByLabel("scope", { exact: true }).inputValue()) === "Only this.", "the dialog shows scope");
check(!(await dlg.getByLabel("why", { exact: true }).isVisible()), "why is collapsed by default");
await dlg.locator("summary", { hasText: "Why" }).click();
check((await dlg.getByLabel("why", { exact: true }).inputValue()) === "Because.", "why opens on request");
check((await dlg.getByText("9 / 320 B").count()) === 1, "the summary counter counts bytes");

await dlg.getByLabel("acceptance", { exact: true }).fill("é".repeat(601));   // 1202 bytes, 601 characters
check((await dlg.getByText("1202 / 1200 B").count()) === 1, "the counter counts bytes, not characters");
const saves = [];
page.on("request", (r) => r.url().endsWith("/api/card_update") && saves.push(r));
await dlg.getByRole("button", { name: "Save" }).click();
check((await dlg.getByText("Acceptance is over 1200 bytes").count()) === 1, "an over-cap save is refused with a message");
check(saves.length === 0, "an over-cap save sends nothing");

await dlg.getByLabel("acceptance", { exact: true }).fill("Changed.");
await dlg.getByLabel("scope", { exact: true }).fill("");
const saved = page.waitForResponse("**/api/card_update");
await dlg.getByRole("button", { name: "Save" }).click();
await saved;
const got = (await api("card_get", { id: full.id })).card;
check(got.acceptance === "Changed." && !got.scope && got.why === "Because." && got.summary === "One line.", "Save stores edited fields, clears an emptied one, keeps the rest");
check(errors.length === 0, "no page errors: " + errors.join("; "));
await browser.close();
process.exit(failures.length ? 1 : 0);

// Browser check of the card prompt (#745): its own box in the card dialog, saved apart from the
// body, and a button that copies it.
//
//   rusty_mill_board --http 127.0.0.1:8796 &           # open board, loopback
//   node tests/ui/prompt.mjs "$(npm root -g)/playwright" http://127.0.0.1:8796
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
const context = await browser.newContext({ viewport: { width: 1300, height: 700 } });
await context.grantPermissions(["clipboard-read", "clipboard-write"], { origin: BASE });
const page = await context.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));

const card = (await api("card_add", { title: "Prompt test", body: "the description" })).card;
await api("card_update", { id: card.id, prompt: "Read the spec, then build it." });
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");
await page.locator(".card", { hasText: "Prompt test" }).click();
const dialog = page.locator("#dlg");
const prompt = dialog.getByLabel("prompt", { exact: true });

check((await prompt.inputValue()) === "Read the spec, then build it.", "the dialog shows the prompt in its own box");
check((await dialog.locator("textarea").nth(0).inputValue()) === "the description", "the body stays the description");

await dialog.getByRole("button", { name: "Copy prompt" }).click();
const copied = await page.evaluate(() => navigator.clipboard.readText());
check(copied === "Read the spec, then build it.", "Copy prompt puts the prompt on the clipboard");

await prompt.fill("A new prompt.");
const savedResponse = page.waitForResponse("**/api/card_update");
await dialog.getByRole("button", { name: "Save" }).click();
await savedResponse;
const saved = (await api("card_get", { id: card.id })).card;
check(saved.prompt === "A new prompt." && saved.body === "the description", "Save stores the prompt apart from the body");
await dialog.getByText(`rev ${saved.rev}`).waitFor();   // the dialog has taken the saved card as its base

// A save that stores title, body and tags but not the prompt (rusty_tick lost the second write):
// the route stores the request without its prompt and answers as the board does.
await page.route("**/api/card_update", async (route) => {
  const sent = JSON.parse(route.request().postData());
  delete sent.prompt;
  const fetched = await route.fetch({ postData: JSON.stringify(sent) });
  const stored = await fetched.json();
  if (!fetched.ok()) throw new Error(`card_update: ${stored.error}`);
  await route.fulfill({ status: 409, contentType: "application/json", body: JSON.stringify({ error: "only part of this change was saved", saved_rev: stored.card.rev }) });
});
await dialog.locator("input").nth(0).fill("Prompt test, renamed");
await prompt.fill("Partial prompt.");
const partial = page.waitForResponse("**/api/card_update");
await dialog.getByRole("button", { name: "Save" }).click();
await partial;
const banner = dialog.locator(".banner");
await banner.filter({ hasText: "the prompt was not" }).waitFor({ timeout: 5000 }).catch(() => {});
check((await banner.textContent()).includes("the prompt was not"), "a partial save says the prompt was not saved, not that someone else edited");
check((await prompt.inputValue()) === "Partial prompt.", "the unsaved prompt stays in its box");
const half = (await api("card_get", { id: card.id })).card;
check(half.title === "Prompt test, renamed" && half.prompt === "A new prompt.", "only the title reached the board");
await page.waitForTimeout(2500);   // a poll or two: the note stays until the next Save
check((await banner.textContent()).includes("the prompt was not"), "the note outlives the next refresh");

await page.unroute("**/api/card_update");
const finished = page.waitForResponse("**/api/card_update");
await dialog.getByRole("button", { name: "Save" }).click();
check((await finished).ok(), "Save again is accepted on the saved revision");
const whole = (await api("card_get", { id: card.id })).card;
check(whole.prompt === "Partial prompt." && whole.title === "Prompt test, renamed", "and stores the prompt");
await page.waitForFunction(() => document.querySelector("#dlg .banner").textContent === "");
check(true, "the note clears once everything is saved");

check(errors.length === 0, `no page errors (${errors.join("; ")})`);
await browser.close();
if (failures.length) { console.error(`${failures.length} failed`); process.exit(1); }

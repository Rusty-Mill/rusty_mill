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

check(errors.length === 0, `no page errors (${errors.join("; ")})`);
await browser.close();
if (failures.length) { console.error(`${failures.length} failed`); process.exit(1); }

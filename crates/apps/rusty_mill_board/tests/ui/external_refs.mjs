// Browser check of external refs (ADR-0006): the card dialog lists each observed ref, linking
// its url, and follows a new observation without being reopened.
//
//   rusty_mill_board --http 127.0.0.1:8796 &           # open board, loopback
//   node tests/ui/external_refs.mjs "$(npm root -g)/playwright" http://127.0.0.1:8796
//
// Needs Playwright and a Chromium (set CHROMIUM=/path if it is not at /opt/pw-browsers/chromium).
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const BASE = process.argv[3] ?? "http://127.0.0.1:8796";

const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "codex", role: "builder", ...args }) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 700 } });
page.on("pageerror", (e) => failures.push(String(e)));

const title = `Refs test ${Math.random().toString(36).slice(2, 8)}`;
const card = (await api("card_add", { title })).card;
const url = "https://example.test/thread-1";
await api("card_execution_observe", { id: card.id, provider: "codex-cloud", ref: "thread-1", state: "running", url, observed_at_ms: Date.now() - 1000 });
await page.goto(BASE);
await page.waitForSelector(".card");
await page.locator(".card", { hasText: title }).click();
const refs = page.locator("#dlg .refs");
const link = refs.locator("a", { hasText: "codex-cloud thread-1" });
check(await link.count() === 1, "the ref is listed by provider and id");
check(await link.getAttribute("href") === url && await link.getAttribute("rel") === "noopener noreferrer", "it links its url, opener-safe");
check((await refs.textContent()).includes("running") && (await refs.textContent()).includes("by codex"), "it shows the state and who observed it");

await api("card_execution_observe", { id: card.id, provider: "github-pr", ref: "o/r#7", state: "succeeded", observed_at_ms: Date.now() });
await page.waitForFunction(() => document.querySelector("#dlg .refs")?.textContent.includes("o/r#7"), null, { timeout: 15000 }).catch(() => {});
const text = await refs.textContent();
check(text.includes("github-pr o/r#7") && text.includes("succeeded"), "a new observation shows up in the open dialog");
await browser.close();
if (failures.length) { console.error(failures.join("\n")); process.exit(1); }

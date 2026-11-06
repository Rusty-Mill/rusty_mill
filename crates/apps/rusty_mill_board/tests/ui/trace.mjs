// Browser check of traceability (#766) and milestone links (#771): the parent chain and the
// children's progress in the card dialog, the pieces count on a parent's ticket, setting the
// parent and milestone from the dialog, and a milestone's progress on the roadmap.
//
//   rusty_mill_board --http 127.0.0.1:8798 &           # open board, loopback
//   node tests/ui/trace.mjs "$(npm root -g)/playwright" http://127.0.0.1:8798 [screenshot-dir]
//
// Needs Playwright and a Chromium (set CHROMIUM=/path if it is not at /opt/pw-browsers/chromium).
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const BASE = process.argv[3] ?? "http://127.0.0.1:8798";
const SHOTS = process.argv[4];

const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "other", role: "human", ...args }) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };

const arc = (await api("card_add", { title: "Arc 2: the mux GUI" })).card;
const s2 = (await api("card_add", { title: "S2: panes", parent: arc.id })).card;
const s2a = (await api("card_add", { title: "S2a: split", parent: s2.id })).card;
await api("card_add", { title: "S2b: resize", parent: s2.id, done: true });
const loose = (await api("card_add", { title: "S2c: focus" })).card;
const beta = (await api("milestone_add", { project: "default", title: "Beta", target_ms: Date.now() + 7 * 864e5 })).milestone;

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 800 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");

// 1. A parent's ticket counts its pieces.
const tag = page.locator(".card", { hasText: "S2: panes" }).locator(".tag", { hasText: "pieces" });
check((await tag.innerText()) === "1/2 pieces", "S2's ticket shows 1/2 pieces");

// 2. The dialog shows the chain up, linked, and the children's progress.
await page.locator(".card", { hasText: "S2a: split" }).click();
const trace = page.locator("#dlg .trace");
check((await trace.innerText()).includes("Arc 2: the mux GUI") && (await trace.innerText()).includes("S2: panes"), "S2a's dialog shows arc 2 › S2");
await trace.locator("button", { hasText: "S2: panes" }).click();
await sleep(200);
check((await trace.innerText()).includes("Split into 1 of 2 done"), "S2's dialog shows its children's progress");
if (SHOTS) await page.screenshot({ path: `${SHOTS}/trace-dialog.png` });
await page.keyboard.press("Escape");

// 3. The parent is set from the dialog by number, and the milestone from the picker.
await page.locator(".card", { hasText: "S2c: focus" }).click();
await page.fill('#dlg input[aria-label="parent card"]', `#${s2.number}`);
await page.locator('#dlg input[aria-label="parent card"]').press("Tab");
await sleep(500);
check((await api("card_get", { id: loose.id })).card.parent === s2.id, "the parent set in the dialog is stored");
await page.locator('#dlg select[aria-label="milestone"]').selectOption(beta.id);
await sleep(500);
check((await api("card_get", { id: loose.id })).card.milestone === beta.id, "the milestone picked in the dialog is stored");
check((await trace.innerText()).includes("S2: panes"), "the chain follows the new parent");
await page.keyboard.press("Escape");
check((await tag.innerText()) === "1/3 pieces", "S2's count follows");

// 4. All pieces done: the parent is flagged, not closed.
await api("card_plan", { id: s2a.id, parent: null });
await api("card_plan", { id: loose.id, parent: null });
await sleep(3500);
check((await tag.innerText()).includes("ready to close"), "S2 is flagged ready to close");
check((await api("card_get", { id: s2.id })).card.column !== "done", "and stays open");

// 5. The roadmap shows the milestone's progress.
await page.click("#view");
await sleep(1500);
check((await page.getByText("Beta 0/1", { exact: true }).count()) === 1, "the roadmap shows Beta 0/1");
if (SHOTS) await page.screenshot({ path: `${SHOTS}/trace-roadmap.png` });

check(!errors.length, "no page errors " + errors.join("; "));
await browser.close();
if (failures.length) { console.log(`${failures.length} failed`); process.exit(1); }

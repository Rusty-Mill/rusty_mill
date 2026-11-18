// Browser check of the card dialog's close controls: Dismiss only hides the dialog, Close card runs
// card_close (reason, one confirmation, card lands in Done as dropped), and a started card says why
// it cannot be closed instead.
//
//   rusty_mill_board --http 127.0.0.1:8798 &           # open board, loopback
//   node tests/ui/close_card.mjs "$(npm root -g)/playwright" http://127.0.0.1:8798
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
const page = await browser.newPage({ viewport: { width: 1300, height: 800 } });
const errors = [], confirms = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("dialog", (d) => { confirms.push(d.message()); d.accept(); });

const idle = (await api("card_add", { title: "Idle card" })).card;
const busy = (await api("card_add", { title: "Busy card" })).card;
await api("card_move", { id: busy.id, to: "ready" });
await api("card_take", { id: busy.id });   // Ready -> Building, claimed
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");
const open = (t) => page.locator(".card", { hasText: t }).first().click();
const dlg = page.locator("#dlg");
const column = async (c) => (await api("card_get", { id: c.id })).card.column;

// 1. Dismiss hides the dialog and leaves the card as it was.
await open("Idle card");
check(await dlg.locator("button", { hasText: /^Close$/ }).count() === 0, "no button is labelled just Close");
await dlg.locator("button", { hasText: "Dismiss" }).click();
check(!(await dlg.evaluate((d) => d.open)), "Dismiss hides the dialog");
check(await column(idle) === "backlog" && confirms.length === 0, "and the card is unchanged, with nothing asked");

// 2. Close card asks once, runs card_close, and the card is Done as dropped work.
await open("Idle card");
await dlg.locator("select[aria-label='close reason']").selectOption("duplicate");
await dlg.locator("button", { hasText: "Close card" }).click();
await sleep(700);
const closed = (await api("card_get", { id: idle.id })).card;
check(confirms.length === 1, "it asks for confirmation once");
check(closed.column === "done" && closed.done_ms == null, "the card is Done with no ship time, as card_close leaves it");
check(closed.closed?.reason === "duplicate", "with the chosen reason");
check(await page.locator('.col[data-col="done"] .card', { hasText: "Idle card" }).count() === 1, "and the board shows it in Done");

// 3. A started card keeps no Close card button and says why.
await dlg.evaluate((d) => d.open && d.close());
await open("Busy card");
check(await dlg.locator("button", { hasText: "Close card" }).isHidden(), "a started card has no Close card button");
check((await dlg.innerText()).includes("Cannot be closed"), "it explains why instead");
check(await column(busy) === "building", "and the card is unchanged");

// 4. If board_roles fails, capabilities are unknown: the dialog still renders, without Close card.
const blind = await browser.newPage({ viewport: { width: 1300, height: 800 } });
const blindErrors = [];
blind.on("pageerror", (e) => blindErrors.push(String(e)));
await blind.route("**/api/board_roles", (r) => r.abort());
await blind.goto(BASE);
await blind.waitForSelector(".card");
await blind.locator(".card", { hasText: "Busy card" }).first().click();
check(await blind.locator("#dlg button", { hasText: "Dismiss" }).isVisible(), "with board_roles down the dialog still renders");
const spare = (await api("card_add", { title: "Spare card" })).card;
await blind.locator("#dlg button", { hasText: "Dismiss" }).click();
await blind.locator(".card", { hasText: "Spare card" }).first().click();
check(await blind.locator("#dlg button", { hasText: "Close card" }).isHidden(), "and an unstarted card shows no Close card (fails closed)");
check(blindErrors.length === 0, "no page errors without roles" + (blindErrors.length ? ": " + blindErrors[0] : ""));

check(errors.length === 0, "no page errors" + (errors.length ? ": " + errors[0] : ""));
await browser.close();
if (failures.length) { console.log(`\n${failures.length} failed`); process.exit(1); }

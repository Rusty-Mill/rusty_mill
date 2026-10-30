// Browser check of the metrics view: the toggle, the scope it follows, and a live update.
//
//   cat > /tmp/p.json <<'J'
//   {"repos": {"games": {"rl": ["crates/apps/rocket_league"]}, "tools": {"orch": ["crates/apps/rusty_orch"]}}}
//   J
//   rusty_mill_board --projects /tmp/p.json --http 127.0.0.1:8797 &
//   node tests/ui/metrics.mjs "$(npm root -g)/playwright" http://127.0.0.1:8797
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const BASE = process.argv[3] ?? "http://127.0.0.1:8797";

const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "other", role: "human", ...args }) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };

const ready = (await api("card_add", { title: "Ready card", project: "rl" })).card;
await api("card_plan", { id: ready.id, priority: 5 });
await api("card_move", { id: ready.id, to: "ready" });
await api("card_add", { title: "Idea", project: "rl" });
await api("card_add", { title: "Orch card", project: "orch" });

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 800 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(BASE);
await page.waitForSelector(".card");
await page.click("#metrics");
await page.waitForSelector(".metrics h3");
const kpi = async (name) => page.locator(".kpi").filter({ hasText: name }).locator("b").innerText();
const panel = (name) => page.locator(".metrics .col").filter({ has: page.locator(`h2:text-is("${name}")`) });
const bar = async (name, row) => panel(name).locator(".bar").filter({ hasText: row }).locator("span").last().innerText();
check((await page.locator(".metrics h3").innerText()).endsWith("all projects"), "no selection covers the whole board");
check((await kpi("open")) === "3", "three open cards");
check((await bar("open by project", "rl")) === "2", "per-project counts");
check((await bar("open by priority", "High")) === "1", "per-priority counts");
check((await panel("idle longest").locator(".card b").allInnerTexts()).join() === "Ready card", "idle lists cards past backlog");

await page.locator("#nav1 .item").filter({ hasText: /^games\d/ }).first().click();
await page.waitForFunction(() => document.querySelector(".metrics h3")?.textContent.endsWith("games"));
check((await kpi("open")) === "2", "a chosen repository narrows the metrics");
check((await panel("open by project").count()) === 1, "per-project panel shown for a repository");

await api("card_move", { id: ready.id, to: "blocked" });
await sleep(2500);
check((await kpi("blocked")) === "1", "metrics follow board changes");

await page.reload();
await page.waitForSelector(".metrics h3");
check(true, "the metrics view is remembered across reloads");
await page.click("#view");
await page.waitForSelector(".roadmap h3");
check((await page.locator("#metrics").getAttribute("aria-pressed")) === "false", "roadmap replaces metrics");
await page.click("#metrics");
await page.waitForSelector(".metrics h3");
await page.click("#metrics");
await page.waitForSelector("[data-col=backlog]");
check((await page.locator(".metrics").count()) === 0, "toggling back shows the board");

// A metrics request that fails after the user went back to the board must not replace it.
await page.route("**/api/board_metrics", async (route) => { await sleep(800); await route.fulfill({ status: 500, contentType: "application/json", body: JSON.stringify({ error: "late failure" }) }); });
await page.click("#metrics");
await page.click("#metrics");
await sleep(1500);
check((await page.locator("[data-col=backlog]").count()) === 1 && !(await page.locator("#board").innerText()).includes("late failure"), "a late metrics failure leaves the board alone");
await page.unroute("**/api/board_metrics");

await page.setViewportSize({ width: 390, height: 800 });
await page.click("#metrics");
await page.waitForSelector(".metrics h3");
check(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), "no sideways scroll at phone width");
await page.screenshot({ path: process.env.SHOT ?? "/dev/null", fullPage: true }).catch(() => {});
check(errors.length === 0, `no page errors (${errors.join("; ")})`);
await browser.close();
if (failures.length) { console.error(`${failures.length} failed`); process.exit(1); }
console.log("all browser checks passed");

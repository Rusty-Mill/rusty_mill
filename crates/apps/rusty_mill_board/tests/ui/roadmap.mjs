// Browser check of the roadmap view: the toggle, the chosen project's lanes, and a live update.
//
//   cat > /tmp/p.json <<'J'
//   {"rl": ["crates/apps/rocket_league"], "orch": ["crates/apps/rusty_orch"]}
//   J
//   rusty_mill_board --projects /tmp/p.json --http 127.0.0.1:8797 &
//   node tests/ui/roadmap.mjs "$(npm root -g)/playwright" http://127.0.0.1:8797
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

const later = (await api("card_add", { title: "Later idea", project: "rl" })).card;
const next = (await api("card_add", { title: "Next up", project: "rl" })).card;
await api("card_move", { id: next.id, to: "ready" });
await api("card_add", { title: "Orch card", project: "orch" });

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 700 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(BASE);
await page.waitForSelector(".card");
await page.locator("#nav1 .item").filter({ hasText: /^rl\d/ }).click();
await page.click("#view");
await page.waitForSelector(".roadmap h3");
const lane = async (name) => page.locator(`[data-lane=${name}] .card b`).allInnerTexts();
const heads = (await page.locator(".roadmap .col h2").allInnerTexts()).map((t) => t.toLowerCase());
check(heads.join("|") === "now · 0|next · 1|later · 1|blocked · 0|shipped · 0", `five lanes with counts (${heads.join("|")})`);
check((await lane("next")).join() === "Next up", "ready card is next");
check((await lane("later")).join() === "Later idea", "backlog card is later");
check(!(await page.locator(".roadmap").innerText()).includes("Orch card"), "other projects are left out");
const rowsOf = async () => (await page.locator(".tl .tl-row:not(.tl-head) .tl-label b").allInnerTexts());
check((await rowsOf()).some((t) => t.endsWith("Next up")) && !(await rowsOf()).some((t) => t.endsWith("Later idea")), `the timeline draws cards from Ready on, not the backlog (${await rowsOf()})`);
check((await page.locator(".ins .tile", { hasText: "Up next" }).locator("b").innerText()) === "1", "the progress panel counts what is up next");
check((await page.locator(".act .act-row").count()) === 2, "recent activity lists the project's cards");

await api("card_move", { id: later.id, to: "blocked" });
await sleep(2500);
check((await lane("blocked")).join() === "Later idea", "the roadmap follows board changes");

await page.reload();
await page.waitForSelector(".roadmap h3");
check(true, "the roadmap view is remembered across reloads");
await page.locator("#nav1 .item.on").filter({ hasText: /^rl\d/ }).click();
await page.waitForFunction(() => document.querySelector(".roadmap h3")?.textContent !== "Roadmap · rl");
check((await rowsOf()).join() === "rl", `with no project chosen, the timeline has a row per project with planned work (${await rowsOf()})`);
check((await page.locator("[data-lane]").count()) === 0, "and no lanes, which belong to one project");
await page.locator(".tl-bar").first().click();
await page.waitForFunction(() => document.querySelector(".roadmap h3")?.textContent === "Roadmap · rl");
check(true, "clicking a project's bar chooses it");

// Plans ahead: a card's target date carries its bar past today, and a milestone sits on its own row.
await api("card_plan", { id: next.id, target_ms: Date.now() + 9 * 864e5 });
await page.waitForFunction(() => [...document.querySelectorAll(".tl-bar .tl-text")].some((t) => t.textContent.startsWith("due ")));
check((await page.locator(".tl-bar .tl-ahead").count()) === 1, "a card due later is drawn on past today");
await page.fill("#actor", "tester"); // an open board has no credential, so the UI needs a name
await page.click(".rm-head button");
await page.fill('#dlg input[aria-label="milestone title"]', "Beta");
const ahead = new Date(Date.now() + 14 * 864e5);
await page.fill('#dlg input[aria-label="milestone date"]', `${ahead.getFullYear()}-${String(ahead.getMonth() + 1).padStart(2, "0")}-${String(ahead.getDate()).padStart(2, "0")}`);
await page.locator("#dlg button", { hasText: "Add milestone" }).click();
await page.waitForSelector(".tl-mark.stone");
const stones = (await api("board_milestones", { project: "rl" })).milestones;
check(stones.length === 1 && stones[0].title === "Beta", "a milestone added from the roadmap is stored in its project");
await page.click(".tl-mark.stone");
await page.locator("#dlg button", { hasText: "Mark reached" }).click();
await page.waitForSelector(".tl-mark.stone.done");
check((await api("board_milestones", { project: "rl" })).milestones[0].done_ms != null, "and can be marked reached");
await api("milestone_add", { project: "rl", title: "Added elsewhere", target_ms: Date.now() + 20 * 864e5 });
await page.waitForFunction(() => document.querySelectorAll(".tl-mark.stone").length === 2, null, { timeout: 15000 });
check(true, "a milestone added by another session appears without a reload");
await page.click("#view");
await page.waitForSelector("[data-col=backlog]");
check((await page.locator(".roadmap").count()) === 0, "toggling back shows the board");

// A roadmap request that fails after the user went back to the board must not replace it.
await page.route("**/api/board_roadmap", async (route) => { await sleep(800); await route.fulfill({ status: 500, contentType: "application/json", body: JSON.stringify({ error: "late failure" }) }); });
await page.click("#view");
await page.click("#view");
await sleep(1500);
check((await page.locator("[data-col=backlog]").count()) === 1 && !(await page.locator("#board").innerText()).includes("late failure"), "a late roadmap failure leaves the board alone");
await page.unroute("**/api/board_roadmap");
check(errors.length === 0, `no page errors (${errors.join("; ")})`);
await browser.close();
if (failures.length) { console.error(`${failures.length} failed`); process.exit(1); }
console.log("all browser checks passed");

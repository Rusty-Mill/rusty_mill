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

await api("card_move", { id: later.id, to: "blocked" });
await sleep(2500);
check((await lane("blocked")).join() === "Later idea", "the roadmap follows board changes");

await page.reload();
await page.waitForSelector(".roadmap h3");
check(true, "the roadmap view is remembered across reloads");
await page.click("#view");
await page.waitForSelector("[data-col=backlog]");
check((await page.locator(".roadmap").count()) === 0, "toggling back shows the board");
check(errors.length === 0, `no page errors (${errors.join("; ")})`);
await browser.close();
if (failures.length) { console.error(`${failures.length} failed`); process.exit(1); }

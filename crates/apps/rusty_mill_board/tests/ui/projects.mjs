// Browser check of projects: the filter, the chip on each card, adding into the selected project
// and moving a card between projects from its dialog.
//
//   echo '{"rl":["crates/apps/rocket_league"],"orch":["crates/apps/rusty_orch"]}' > /tmp/p.json
//   rusty_mill_board --projects /tmp/p.json --http 127.0.0.1:8797 &
//   node tests/ui/projects.mjs "$(npm root -g)/playwright" http://127.0.0.1:8797
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

await api("card_add", { title: "RL card", project: "rl" });
await api("card_add", { title: "Orch card", project: "orch" });
await api("card_add", { title: "Loose card" });

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 700 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");
const titles = async () => (await page.locator(".card b").allInnerTexts()).sort();

check((await titles()).length === 3, "all projects show every card");
check((await page.locator(".card .tag.proj").allInnerTexts()).sort().join() === "default,orch,rl", "each card names its project");

await page.selectOption("#proj", "rl");
check((await titles()).join() === "RL card", "the project filter hides the others");
check((await page.locator(".card .tag.proj").count()) === 0, "no chip when one project is selected");

await page.fill("#title", "Added under rl");
await page.click("#add");
await page.waitForFunction(() => [...document.querySelectorAll(".card b")].some((b) => b.textContent === "Added under rl"));
const added = (await api("board_list", { project: "rl" })).cards.map((c) => c.title).sort();
check(added.join() === "Added under rl,RL card", "a card added while rl is selected belongs to rl");

await page.locator(".card", { hasText: "Added under rl" }).click();
await page.locator("dialog select").selectOption("orch");
await sleep(600);
const moved = (await api("board_list", { project: "orch" })).cards.map((c) => c.title);
check(moved.includes("Added under rl"), "the dialog moves a card to another project");
await page.keyboard.press("Escape");
check(!(await titles()).includes("Added under rl"), "it leaves the filtered view");

await page.reload();
await page.waitForSelector(".card");
check((await page.inputValue("#proj")) === "rl", "the chosen project is remembered");

check(errors.length === 0, `no page errors ${errors.join("; ")}`);
await browser.close();
if (failures.length) { console.error(`\n${failures.length} check(s) failed`); process.exit(1); }
console.log("\nall browser checks passed");

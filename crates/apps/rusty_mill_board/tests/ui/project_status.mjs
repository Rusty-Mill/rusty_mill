// Browser check of project statuses: the board shows active projects by default, the status picker
// shows the others, and Settings > Repositories sets a project's status at once.
//
//   D=$(mktemp -d); printf '%s' '{"rl": ["crates/apps/rocket_league"], "orch": ["crates/apps/rusty_orch"], "lab": ["lab"]}' > $D/projects.json
//   rusty_mill_board --settings-dir $D --http 127.0.0.1:8799 2>$D/log &
//   node tests/ui/project_status.mjs "$(npm root -g)/playwright" http://127.0.0.1:8799 "$(cat $D/first-admin.token)"
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const [BASE, TOKEN] = [process.argv[3], process.argv[4]];

const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };
const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json", authorization: `Bearer ${TOKEN}` }, body: JSON.stringify(args) });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};

for (const project of ["rl", "orch", "lab"]) await api("card_add", { title: `${project} card`, project });
await api("settings_project_status_set", { project: "orch", status: "paused" });

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 900 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(`${BASE}/#token=${TOKEN}`);
await page.waitForSelector(".card");
const titles = async () => (await page.locator(".card b").allInnerTexts()).sort();
const chips = async () => (await page.locator("#nav1 .item .name").allInnerTexts()).map((t) => t.trim());

check((await titles()).join() === "lab card,rl card", `active projects only by default (${await titles()})`);
check(!(await chips()).includes("orch"), "a paused project has no chip by default");
await page.selectOption("#pstatus", "paused");
check((await titles()).join() === "orch card", `the picker shows paused projects (${await titles()})`);
await page.selectOption("#pstatus", "");
check((await titles()).length === 3, "All projects shows every card");

// Settings > Repositories: make `lab` an experiment; it leaves the active view at once.
await page.selectOption("#pstatus", "active");
await page.click("#gear");
await page.waitForSelector("#settings[open]");
await page.locator("#settings button", { hasText: /^Repositories/ }).first().click();
await page.selectOption('#settings select[aria-label="lab status"]', "experiment");
await page.waitForFunction(() => !document.querySelector("#settings .err")?.textContent);
check((await api("board_projects")).status.lab === "experiment", "the status is saved");
await page.keyboard.press("Escape");
await page.waitForFunction(() => [...document.querySelectorAll(".card b")].every((b) => b.textContent !== "lab card"));
check((await titles()).join() === "rl card", `an experiment leaves the default view (${await titles()})`);

check(errors.length === 0, `no page errors (${errors.join("; ")})`);
await browser.close();
if (failures.length) { console.error(`${failures.length} failed`); process.exit(1); }

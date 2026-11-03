// Browser check that a credential limited to projects other than `default` gets no Inbox row.
//
//   D=$(mktemp -d); echo '{"rl": ["crates/apps/rocket_league"], "orch": ["crates/apps/rusty_orch"]}' > $D/p.json
//   echo '[{"token": "scoped-token-0123456789", "actor": "dev", "role": "builder", "projects": ["rl", "orch"]}]' > $D/ids.json
//   chmod 600 $D/ids.json
//   rusty_mill_board --projects $D/p.json --identities $D/ids.json --http 127.0.0.1:8794 &
//   node tests/ui/inbox_scoped.mjs "$(npm root -g)/playwright" http://127.0.0.1:8794 scoped-token-0123456789
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const [BASE, TOKEN] = [process.argv[3], process.argv[4]];

const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };
const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json", authorization: `Bearer ${TOKEN}` }, body: JSON.stringify(args) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
await api("card_add", { title: "RL card", project: "rl" });
await api("card_add", { title: "Orch card", project: "orch" });

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 700 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(`${BASE}/#token=${TOKEN}`);
await page.waitForSelector(".card");
const titles = async () => (await page.locator(".card b").allInnerTexts()).sort();
const rows = (await page.locator("#nav1 .item").allInnerTexts()).map((t) => t.replace(/\s+/g, " ").trim());

check(!rows.some((r) => r.startsWith("Inbox")), `no Inbox row for a credential without project default (${rows.join("|")})`);
check(rows.some((r) => r === "rl 1") && rows.some((r) => r === "orch 1"), "its permitted projects are listed");
check((await page.locator("#dlg option[value=default]").count()) === 0, "and the dialog does not offer it");

await page.locator("#nav1 .item", { hasText: /^rl/ }).click();
check((await titles()).join() === "RL card", "choosing a permitted project still filters");
await page.fill("#title", "Added under rl");
await page.click("#add");
await page.waitForFunction(() => [...document.querySelectorAll(".card b")].some((b) => b.textContent === "Added under rl"));
check((await api("board_list", { project: "rl" })).cards.some((c) => c.title === "Added under rl"), "and adding there keeps the project");

check(errors.length === 0, `no page errors (${errors.join("; ")})`);
await browser.close();
if (failures.length) { console.error(`${failures.length} failed`); process.exit(1); }
console.log("all browser checks passed");

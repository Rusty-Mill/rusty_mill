// Browser check of the roadmap for a credential limited to one project, starting with no saved choice.
//
//   D=$(mktemp -d); echo '{"rl": ["crates/apps/rocket_league"], "orch": ["crates/apps/rusty_orch"]}' > $D/p.json
//   echo '[{"token": "scoped-token-0123456789", "actor": "dev", "role": "builder", "projects": ["rl"]}]' > $D/ids.json
//   chmod 600 $D/ids.json
//   rusty_mill_board --projects $D/p.json --identities $D/ids.json --http 127.0.0.1:8796 &
//   node tests/ui/roadmap_scoped.mjs "$(npm root -g)/playwright" http://127.0.0.1:8796 scoped-token-0123456789
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
await api("card_add", { title: "Scoped idea", project: "rl" });

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 700 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(`${BASE}/#token=${TOKEN}`);
await page.waitForSelector(".card");
await page.click("#view");
await page.waitForSelector(".roadmap h3");
check((await page.locator(".roadmap h3").innerText()) === "Roadmap · rl", "with nothing chosen, the roadmap shows the one project this credential may see");
check((await page.locator("[data-lane=later] .card b").allInnerTexts()).join() === "Scoped idea", "its cards are in the lanes");

await page.reload();
await page.waitForSelector(".roadmap h3");
check((await page.locator(".roadmap h3").innerText()) === "Roadmap · rl", "the roadmap is restored after a reload");
check(errors.length === 0, `no page errors (${errors.join("; ")})`);
await browser.close();
if (failures.length) { console.error(`${failures.length} failed`); process.exit(1); }
console.log("all browser checks passed");

// Browser check of the Decisions view (#595) against tests/ui/fake_bbp.sh: the button shows the
// count, /inbox opens the view, Choose previews BBP's events before acting, and a decided item leaves.
//
// The inbox needs a credential-bound admin, so the board runs with one token for the owner:
//
//   export FAKE_BBP_DIR=$(mktemp -d) ID=$(mktemp)
//   echo '[{"token":"decisions-check-owner-token-0001","actor":"nano","role":"human"}]' > $ID && chmod 600 $ID
//   rusty_mill_board --http 127.0.0.1:8796 --identities $ID --bbp-bin tests/ui/fake_bbp.sh --bbp-root /srv/bbp &
//   node tests/ui/decisions.mjs "$(npm root -g)/playwright" http://127.0.0.1:8796
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const BASE = process.argv[3] ?? "http://127.0.0.1:8796";
const TOKEN = "decisions-check-owner-token-0001";

const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json", authorization: `Bearer ${TOKEN}` }, body: JSON.stringify(args) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };

const { card } = await api("card_add", { title: "Pick the cache" });
await api("card_execution_link", { id: card.id, execution: "x1", goal: "g", bbp_task: "T1", attempt: "a", assignment: "orch", generation: 1 });

const browser = await chromium.launch();
const page = await browser.newPage();
page.on("pageerror", e => failures.push(e.message));
await page.goto(`${BASE}/inbox#token=${TOKEN}`);
await page.waitForTimeout(800);
check((await page.textContent("#decide")) === "Decisions · 1", "the button counts one decision");
check((await page.textContent("#board")).includes("Use redis"), "/inbox shows the candidates side by side");
let asked = "";
page.on("dialog", d => { asked = d.message(); d.accept(); });
await page.click("text=Use redis >> xpath=.. >> button");
await page.waitForTimeout(800);
check(asked.startsWith("choose msg:1 msg:6") && asked.includes("ClaimAccepted"), "Choose shows BBP's dry run first");
check((await page.textContent("#board")).includes("Nothing needs you."), "a decided item leaves the inbox");
await browser.close();
if (failures.length) { console.error(failures.join("\n")); process.exit(1); }

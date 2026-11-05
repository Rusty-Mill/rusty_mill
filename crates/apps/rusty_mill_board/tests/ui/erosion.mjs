// Browser check of the #704 erosion detail (D91) against tests/ui/fake_bbp.sh: a claim erosion
// fails shows its shares, limits and worst function; one it passes report-only shows only its
// unmeasured code; both show unmeasured code (D45).
//
//   export FAKE_BBP_DIR=$(mktemp -d) ID=$(mktemp) && touch $FAKE_BBP_DIR/erosion
//   echo '[{"token":"decisions-check-owner-token-0001","actor":"nano","role":"human"}]' > $ID && chmod 600 $ID
//   rusty_mill_board --http 127.0.0.1:8797 --identities $ID --bbp-bin tests/ui/fake_bbp.sh --bbp-root /srv/bbp &
//   node tests/ui/erosion.mjs "$(npm root -g)/playwright" http://127.0.0.1:8797
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const BASE = process.argv[3] ?? "http://127.0.0.1:8797";
const TOKEN = "decisions-check-owner-token-0001";

const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json", authorization: `Bearer ${TOKEN}` }, body: JSON.stringify(args) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };

const { card } = await api("card_add", { title: "Eroding build" });
await api("card_execution_link", { id: card.id, execution: "x1", goal: "g", bbp_task: "T1", attempt: "a", assignment: "orch", generation: 1 });

const browser = await chromium.launch();
const page = await browser.newPage();
page.on("pageerror", e => failures.push(e.message));
await page.goto(`${BASE}/inbox#token=${TOKEN}`);
await page.waitForTimeout(800);
const [fails, passes] = await page.$$eval("#board section", s => s.map(x => x.textContent));
check(fails?.includes("Erosion fails (limits 0.5% and 0.5%): concentration 0% → 0% (0), duplication 0% → 48% (+48)"), "a failing claim shows its shares and limits");
check(fails?.includes("Complexity 0 → 2 in src/b.rs::grand"), "and its worst function");
check(fails?.includes("Not measured by erosion: tool.py"), "and its unmeasured code");
check(passes?.includes("Not measured by erosion: tool.py"), "a passing claim shows its unmeasured code");
check(!passes?.includes("Erosion fails") && !passes?.includes("Complexity"), "but not its shares or worst functions (Q23)");
await browser.close();
if (failures.length) { console.error(failures); process.exit(1); }

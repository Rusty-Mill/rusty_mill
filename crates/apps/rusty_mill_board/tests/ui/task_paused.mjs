// Browser check of Reject on a #598 task pause against tests/ui/fake_bbp.sh: Reject asks why, a
// cancelled prompt sends nothing, and an answered one cancels the task with that reason.
//
//   export FAKE_BBP_DIR=$(mktemp -d) ID=$(mktemp) && touch $FAKE_BBP_DIR/paused
//   echo '[{"token":"decisions-check-owner-token-0001","actor":"nano","role":"human"}]' > $ID && chmod 600 $ID
//   rusty_mill_board --http 127.0.0.1:8796 --identities $ID --bbp-bin tests/ui/fake_bbp.sh --bbp-root /srv/bbp &
//   node tests/ui/task_paused.mjs "$(npm root -g)/playwright" http://127.0.0.1:8796
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const BASE = process.argv[3] ?? "http://127.0.0.1:8796";
const TOKEN = "decisions-check-owner-token-0001";
const CALLS = `${process.env.FAKE_BBP_DIR}/calls`;
const acted = () => readFileSync(CALLS, "utf8").split("\n").filter(l => l && !l.startsWith("inbox"));

const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json", authorization: `Bearer ${TOKEN}` }, body: JSON.stringify(args) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };

const { card } = await api("card_add", { title: "Paused build" });
await api("card_execution_link", { id: card.id, execution: "x1", goal: "g", bbp_task: "T1", attempt: "a", assignment: "orch", generation: 1 });

const browser = await chromium.launch();
const page = await browser.newPage();
page.on("pageerror", e => failures.push(e.message));
await page.goto(`${BASE}/inbox#token=${TOKEN}`);
await page.waitForTimeout(800);
check((await page.textContent("#board")).includes("Task T1 paused."), "the pause is in the inbox");
let asked = "", answer = null;
// The prompt is answered or cancelled; the dry-run preview that follows an answer is accepted.
page.on("dialog", d => {
  if (d.type() !== "prompt") return d.accept();
  asked = d.message();
  return answer === null ? d.dismiss() : d.accept(answer);
});
await page.click("button:text-is('Reject')");
await page.waitForTimeout(500);
check(asked === "Why cancel this task?", "Reject on a pause asks why");
check(acted().length === 0, "a cancelled prompt sends nothing");
answer = "the breaker was right";
await page.click("button:text-is('Reject')");
await page.waitForTimeout(800);
const sent = acted();
check(sent.some(l => l.includes("cancel") && l.includes("the breaker was right")), `the answer cancels the task with it: ${sent.join(" | ")}`);
check(!(await page.textContent("#board")).includes("cancelling a task needs a reason"), "no missing-reason refusal");
await browser.close();
if (failures.length) { console.error(failures.join("\n")); process.exit(1); }

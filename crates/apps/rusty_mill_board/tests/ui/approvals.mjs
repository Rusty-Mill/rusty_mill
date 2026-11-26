// Browser check of #447: an unblock the approver proxy filed waits on the card, and the owner's
// Confirm in the card dialog crosses the gate through card_approve.
//
//   D=$(mktemp -d); cp deploy/factory.workflow.json $D/workflow.json
//   echo '{"human":["triage","admin"],"approver":["approve"]}' > $D/roles.json
//   rusty_mill_board --settings-dir $D --http 127.0.0.1:8799 2>$D/log &
//   node tests/ui/approvals.mjs "$(npm root -g)/playwright" http://127.0.0.1:8799 "$(cat $D/first-admin.token)"
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const [BASE, TOKEN] = [process.argv[3], process.argv[4]];

const call = async (token, op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json", authorization: `Bearer ${token}` }, body: JSON.stringify(args) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };

const proxy = (await call(TOKEN, "settings_agent_add", { agent: { name: "nano-proxy", role: "approver" } })).token;
const card = (await call(TOKEN, "card_add", { title: "Approval test" })).card;
await call(TOKEN, "card_move", { id: card.id, to: "blocked" });
const { text_digest } = await call(proxy, "card_get", { id: card.id });
const filed = (await call(proxy, "card_approve", { id: card.id, to: "ready", digest: text_digest, verbatim: "Unblock it? Yes.", source: "cmsg_test" })).card;
check(filed.column === "blocked", "the proxy's unblock is filed, not moved");

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 900 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(BASE);
await page.waitForSelector("#signin[open]");
await page.fill("#signin-token", TOKEN);
await page.click("#signin button");
await page.waitForFunction(() => document.querySelector("#actor").value === "admin");
await page.locator(".card", { hasText: "Approval test" }).click();
const pending = page.locator("#dlg .approvals .pending");
await pending.waitFor();
check((await pending.innerText()).includes("filed by nano-proxy"), "the dialog shows who filed it");
check((await pending.innerText()).includes("Unblock it? Yes."), "and the words it was approved with");
await pending.getByRole("button", { name: "Confirm" }).click();
await pending.waitFor({ state: "detached", timeout: 5000 });
const after = (await call(TOKEN, "card_get", { id: card.id })).card;
check(after.column === "ready", "the owner's tap crosses the gate: " + after.column);
check(after.duties.some((d) => d.duty === "approval" && d.confirmed_by === "admin"), "the ledger records who confirmed it");
check(errors.length === 0, "no page errors: " + errors.join("; "));
await browser.close();
process.exit(failures.length ? 1 : 0);

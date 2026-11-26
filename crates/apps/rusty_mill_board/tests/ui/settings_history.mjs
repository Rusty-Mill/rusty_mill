// Browser check of Settings > History: a real change is listed, and a synthetic response with
// `truncated: true` and an omitted value shows the notices.
//
//   D=$(mktemp -d); rusty_mill_board --settings-dir $D --http 127.0.0.1:8799 2>$D/log &
//   TOKEN=$(cat $D/first-admin.token)
//   node tests/ui/settings_history.mjs "$(npm root -g)/playwright" http://127.0.0.1:8799 "$TOKEN"
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const [BASE, TOKEN] = [process.argv[3], process.argv[4]];

const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 900 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(`${BASE}/#token=${TOKEN}`);
await page.waitForSelector("#gear");
await page.waitForFunction(() => document.querySelector("#actor").value);
await fetch(`${BASE}/api/settings_rules_set`, { method: "POST", headers: { "content-type": "application/json", authorization: `Bearer ${TOKEN}` }, body: JSON.stringify({ movement: "open" }) });

await page.click("#gear");
await page.waitForSelector("#settings[open]");
const dialog = page.locator("#settings");
await dialog.getByRole("button", { name: "History" }).click();
await dialog.locator("table").waitFor();
check((await dialog.locator("table tr").count()) >= 2, "a real settings change is listed");
check(/2 MiB/.test(await dialog.innerText()), "the help mentions the 2 MiB cap");
check((await dialog.getByText("Older entries are not shown").count()) === 0, "no cut notice when the response is whole");

const entry = (old, nw) => ({ at_ms: Date.now(), actor: "admin", via_connector: false, op: "settings_roles_set", section: "roles", old, new: nw });
await page.route("**/api/settings_history", (r) => r.fulfill({ json: { truncated: true, entries: [entry({ omitted_bytes: 70000 }, { omitted_bytes: 71000 })] } }));
await dialog.getByRole("button", { name: "Rules" }).click();
await dialog.getByRole("button", { name: "History" }).click();
await dialog.getByText("Older entries are not shown").waitFor();
check(true, "a truncated response shows the notice");
check((await dialog.getByText("too large to keep: 70000 bytes").count()) === 1, "an omitted value says so");
check(errors.length === 0, `no page errors ${errors.join("; ")}`);

await browser.close();
if (failures.length) { console.log(`\n${failures.length} failed`); process.exit(1); }

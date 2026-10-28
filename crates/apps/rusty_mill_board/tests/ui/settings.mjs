// Browser check of the settings view, against a board started with a settings directory:
//
//   D=$(mktemp -d); rusty_mill_board --settings-dir $D --http 127.0.0.1:8798 2>$D/log &
//   TOKEN=$(cat $D/first-admin.token)
//   node tests/ui/settings.mjs "$(npm root -g)/playwright" http://127.0.0.1:8798 "$TOKEN"
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const [BASE, TOKEN] = [process.argv[3], process.argv[4]];

const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };
const api = async (token, op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json", authorization: `Bearer ${token}` }, body: JSON.stringify(args) });
  return { status: r.status, body: await r.json().catch(() => ({})) };
};

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 900 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("dialog", (d) => d.accept());
await page.goto(`${BASE}/#token=${TOKEN}`);
await page.waitForSelector("#gear");
await page.waitForFunction(() => document.querySelector("#actor").value);
check((await page.inputValue("#actor")) === "admin" && (await page.locator("#actor").isDisabled()), "a per-model token fixes who you are in the header");

await page.click("#gear");
await page.waitForSelector("#settings[open]");
const dialog = page.locator("#settings");
check((await dialog.locator("table tr").count()) === 2, "the Agents tab lists the first admin");

// Agents: add one, see its token once, use it, rotate, remove.
await dialog.locator("input[placeholder^='new agent']").fill("claude-code");
await dialog.locator(".row select").first().selectOption("builder");
await dialog.getByRole("button", { name: "Add agent" }).click();
await dialog.locator(".secret code").waitFor();
const token = await dialog.locator(".secret code").innerText();
check(token.length === 48, "adding an agent shows its token");
check((await api(token, "board_whoami")).body.actor === "claude-code", "and the token works at once");
await dialog.getByRole("button", { name: "Repositories" }).click();
await dialog.getByRole("button", { name: "Agents" }).click();
check((await dialog.locator(".secret").count()) === 0, "the token is gone once you leave the tab");
check((await dialog.locator("table tr").count()) === 3, "the agent is listed");
await dialog.locator("tr", { hasText: "claude-code" }).getByRole("button", { name: "New token" }).click();
await dialog.locator(".secret code").waitFor();
const rotated = await dialog.locator(".secret code").innerText();
check(rotated !== token && (await api(token, "board_whoami")).status === 401, "a new token replaces the old one");

// A refusal is shown, not swallowed.
await dialog.locator("input[placeholder^='new agent']").fill("claude-code");
await dialog.getByRole("button", { name: "Add agent" }).click();
await page.waitForFunction(() => document.querySelector("#settings .err")?.textContent.includes("already"));
check(true, "a duplicate agent is refused with the reason");

// Repositories and projects.
await dialog.getByRole("button", { name: "Repositories" }).click();
await dialog.locator("input[placeholder='new repository name']").fill("rusty_mill");
await dialog.getByRole("button", { name: "+ repository" }).click();
await dialog.getByRole("button", { name: "+ project" }).last().click();
await dialog.locator("input[placeholder='project name']").fill("rocket_league");
await dialog.locator("textarea").fill("crates/apps/rocket_league");
await dialog.getByRole("button", { name: "Save repositories and projects" }).click();
await page.waitForFunction(() => !document.querySelector("#settings .err")?.textContent);
const listed = (await api(rotated, "board_projects")).body.repos;
check(listed.rusty_mill?.rocket_league?.[0] === "crates/apps/rocket_league", "a repository with a project is saved");
await dialog.getByRole("button", { name: "Close" }).click();
await page.locator("#nav1 .item", { hasText: "rusty_mill" }).waitFor({ timeout: 3000 }).catch(() => {});
check((await page.locator("#nav1 .item", { hasText: "rusty_mill" }).count()) >= 1, "and appears in the side panel at once");

// A credential limited to some projects sees each repository with just those: a repository's one visible
// project that is not its own stays in the Projects list, and cards added under it land there.
check((await api(TOKEN, "settings_projects_set", { repos: { rusty_mill: { rocket_league: ["crates/apps/rocket_league"] }, super_board: { skills: ["skills"] } } })).status === 200, "(setup) two repositories");
const scoped = await api(TOKEN, "settings_agent_add", { agent: { name: "scoped", role: "builder", projects: ["rocket_league", "skills"] } });
check(scoped.status === 200, "(setup) an agent limited to rocket_league and skills");
const limited = await browser.newPage({ viewport: { width: 1300, height: 900 } });
limited.on("pageerror", (e) => errors.push(String(e)));
await limited.goto(`${BASE}/#token=${scoped.body.token}`);
await limited.waitForFunction(() => document.querySelector("#actor").value === "scoped" && document.querySelectorAll("#nav1 .item").length > 0);
const sideItems = async () => (await limited.locator("#nav1 .item").allInnerTexts()).map((t) => t.split("\n")[0].trim());
const sideItem = (name) => limited.locator("#nav1 .item").filter({ hasText: new RegExp(`^${name}\\d`) });
check(JSON.stringify(await sideItems()) === JSON.stringify(["All repositories", "rusty_mill", "super_board", "rocket_league", "skills"]), `a limited agent sees its projects listed (${await sideItems()})`);
await sideItem("super_board").click();
check(JSON.stringify((await sideItems()).slice(3)) === '["skills"]', "choosing a repository keeps its one visible project");
await sideItem("skills").click();
await limited.fill("#title", "scoped card");
await limited.click("#add");
await limited.locator(".card", { hasText: "scoped card" }).waitFor();
check((await api(scoped.body.token, "board_list", { project: "skills" })).body.cards.some((c) => c.title === "scoped card"), "and a card added under it lands in that project");
await limited.close();

// Roles.
await page.click("#gear");
await dialog.getByRole("button", { name: "Roles" }).click();
await dialog.locator("input[placeholder^='new role']").fill("tester");
await dialog.getByRole("button", { name: "Add role" }).click();
await dialog.locator("tr", { hasText: "tester" }).waitFor();
check((await api(rotated, "board_roles")).body.roles.tester !== undefined, "a role is added");
await page.waitForTimeout(200);
await dialog.locator("tr", { hasText: "tester" }).locator("input[type=checkbox]").nth(2).check();
await dialog.getByRole("button", { name: "Save roles" }).click();
await page.waitForTimeout(400);
check(JSON.stringify((await api(rotated, "board_roles")).body.roles.tester) === '["qa"]', "its capabilities are saved");
check((await dialog.innerText()).includes("Verify built work"), "each capability is explained");

// Rules: turning movement rules off and on is saved at once.
await dialog.getByRole("button", { name: "Rules" }).click();
const enforce = dialog.getByLabel("Enforce card movement rules");
check(await enforce.isChecked(), "movement rules start on");
await enforce.uncheck();
await page.waitForFunction(() => document.querySelector("#settings")?.innerText.includes("except out of Done"));
check((await api(TOKEN, "settings_get")).body.rules.movement === "open", "unticking turns them off");
await dialog.getByLabel("Enforce card movement rules").check();
await page.waitForFunction(() => document.querySelector("#settings")?.innerText.includes("along the column table"));
check((await api(TOKEN, "settings_get")).body.rules.movement === "enforced", "ticking turns them back on");
// A failed save, either way, shows the error and puts the checkbox and its note back to what the server holds.
const rulesShown = (on, note) => page.waitForFunction(([on, note]) => {
  const box = [...document.querySelectorAll("#settings input[type=checkbox]")].find((b) => b.parentElement.textContent.includes("Enforce card movement rules"));
  return box && box.checked === on && !box.disabled && document.querySelector("#settings").innerText.includes(note);
}, [on, note]);
const refuse = () => page.route("**/api/settings_rules_set", (r) => r.fulfill({ status: 422, contentType: "application/json", body: JSON.stringify({ error: "disk full" }) }));
await refuse();
await dialog.getByLabel("Enforce card movement rules").uncheck();
await page.waitForFunction(() => document.querySelector("#settings .err")?.textContent.includes("disk full"));
await rulesShown(true, "along the column table");
check((await api(TOKEN, "settings_get")).body.rules.movement === "enforced", "a failed turn-off leaves the checkbox, its note and the server on");
await page.unroute("**/api/settings_rules_set");
await dialog.getByLabel("Enforce card movement rules").uncheck();
await rulesShown(false, "except out of Done");
await refuse();
await dialog.getByLabel("Enforce card movement rules").check();
await page.waitForFunction(() => document.querySelector("#settings .err")?.textContent.includes("disk full"));
await rulesShown(false, "except out of Done");
check((await api(TOKEN, "settings_get")).body.rules.movement === "open", "a failed turn-on leaves the checkbox, its note and the server off");
await page.unroute("**/api/settings_rules_set");
await dialog.getByLabel("Enforce card movement rules").check();
await rulesShown(true, "along the column table");

// Apps and Server: read-only views of sign-in and how the board was started.
await dialog.getByRole("button", { name: "Apps" }).click();
check((await dialog.innerText()).includes("OAuth sign-in is off"), "the Apps tab says when sign-in is off");
await dialog.getByRole("button", { name: "Server" }).click();
check((await dialog.innerText()).includes("roles.json"), "the Server tab shows the settings files");

// GitHub: a bad repository is refused; the token is not shown again.
await dialog.getByRole("button", { name: "GitHub" }).click();
await dialog.locator("input[placeholder='owner/name']").fill("../nope");
await dialog.locator("input[type=password]").fill("ghp_secret");
await dialog.getByRole("button", { name: /Save and start sync/ }).click();
await page.waitForFunction(() => document.querySelector("#settings .err")?.textContent.length > 0);
check(true, "an invalid GitHub repository is refused");
check(!(await dialog.innerText()).includes("ghp_secret") && (await dialog.locator("input[type=password]").inputValue()) === "ghp_secret", "the token is only ever in the field you typed it in");
await dialog.getByRole("button", { name: "Close" }).click();

// A non-admin sees no settings.
const builder = await api(rotated, "settings_get");
check(builder.status === 403 || builder.status === 200, "(sanity) the builder token reaches the API");
const denied = await api(rotated, "settings_agent_remove", { name: "admin" });
check(denied.status === 403, "an agent without admin cannot change settings");

// The administrator rotates their own token: the page must carry on with the new one.
await page.click("#gear");
await page.waitForFunction(() => document.querySelector("#settings").open);
await dialog.locator("tr", { hasText: "admin" }).first().getByRole("button", { name: "New token" }).click();
await dialog.locator(".secret code").waitFor();
const adminToken = await dialog.locator(".secret code").innerText();
check(adminToken !== TOKEN && adminToken.length === 48, "rotating your own token shows the new one");
check((await api(TOKEN, "board_whoami")).status === 401, "the old admin token stops working");
check((await api(adminToken, "settings_get")).status === 200, "the new admin token works");
check((await dialog.locator("table tr").count()) >= 3, "the settings view refreshed with the new token");
check((await page.evaluate(() => localStorage.getItem("token"))) === adminToken, "and a reload would keep working");
const before = await page.locator(".card").count();
await dialog.getByRole("button", { name: "Close" }).click();
await page.fill("#title", "after rotation");
await page.click("#add");
await page.waitForFunction((n) => document.querySelectorAll(".card").length === n + 1, before);
check(true, "card calls succeed after self-rotation");

check(errors.length === 0, `no page errors ${errors.join("; ")}`);
await browser.close();
if (failures.length) { console.error(`\n${failures.length} check(s) failed`); process.exit(1); }
console.log("\nall browser checks passed");

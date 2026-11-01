// Browser check of Settings > Columns, against a board started with a settings directory:
//
//   D=$(mktemp -d); rusty_mill_board --settings-dir $D --http 127.0.0.1:8798 2>$D/log &
//   TOKEN=$(cat $D/first-admin.token)
//   node tests/ui/columns.mjs "$(npm root -g)/playwright" http://127.0.0.1:8798 "$TOKEN"
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const [BASE, TOKEN] = [process.argv[3], process.argv[4]];

const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };
const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json", authorization: `Bearer ${TOKEN}` }, body: JSON.stringify(args) });
  return { status: r.status, body: await r.json().catch(() => ({})) };
};

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 900 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("dialog", (d) => d.accept());
await page.goto(`${BASE}/#token=${TOKEN}`);
await page.waitForSelector("#board .col");
const headingsOf = (p) => p.$$eval("#board .col h2", (hs) => hs.map((h) => h.textContent));
const headings = () => headingsOf(page);
// A second, already-open client: it must follow column changes made elsewhere without a reload.
const other = await browser.newPage({ viewport: { width: 1300, height: 900 } });
other.on("pageerror", (e) => errors.push(String(e)));
await other.goto(`${BASE}/#token=${TOKEN}`);
await other.waitForSelector("#board .col");
const otherShows = (pred) => other.waitForFunction(pred, null, { timeout: 10000 }).then(() => true, () => false);
check((await headings()).join(",") === "Backlog,Ready,Building,QA,Review,Done,Blocked", "the board shows the default columns by label");

// The gear works once the page has read who you are; until then a click may be lost.
const openColumns = async () => {
  for (let i = 0; i < 20 && !(await page.locator("#settings[open]").count()); i++) {
    await page.click("#gear");
    await page.waitForTimeout(250);
  }
  await page.locator("#settings").getByRole("button", { name: "Columns" }).click();
};
await openColumns();
const dialog = page.locator("#settings");

// Add a Design work column, relabel Ready, route Backlog -> Design, save.
await dialog.locator("input[placeholder^='new column id']").fill("design");
await dialog.locator("input[placeholder='label']").fill("Design");
await dialog.getByRole("button", { name: "Add column" }).click();
await dialog.getByLabel("label of ready").fill("Approved");
await dialog.getByRole("button", { name: "Add move" }).click();
const last = dialog.locator("table").nth(1).locator("tr").last();
await last.getByLabel("from").selectOption("backlog");
await last.getByLabel("to").selectOption("design");
await last.getByLabel("capability").selectOption("build");
await dialog.getByRole("button", { name: "Save columns" }).click();
await page.waitForFunction(() => [...document.querySelectorAll("#board .col h2")].some((h) => h.textContent === "Design"));
const heads = await headings();
check(heads.join(",") === "Backlog,Approved,Building,QA,Review,Design,Done,Blocked", `new column before Done, Ready relabelled (${heads})`);
const flow = (await api("board_columns")).body;
check(flow.transitions.some((t) => t.from === "backlog" && t.to === "design" && t.cap === "build"), "the new move is saved");

// A card in Design keeps the column from being removed; the error is shown and nothing changes.
const card = (await api("card_add", { title: "in design" })).body.card;
check((await api("card_move", { id: card.id, to: "design" })).status === 200, "a card moves into the new column");
await page.waitForFunction(() => [...document.querySelectorAll("#board .col[data-col=design] .card")].length === 1);
check(await otherShows(() => document.querySelectorAll("#board .col[data-col=design] .card").length === 1),
  "an open client shows the new column and the card moved into it");
check(await otherShows(() => [...document.querySelectorAll("#board .col h2")].some((h) => h.textContent === "Approved")),
  "and the relabel");
// A relabel alone (no card moves) still reaches the open client.
const relabelled = (await api("board_columns")).body;
relabelled.columns.find((c) => c.id === "design").label = "Designing";
check((await api("settings_workflow_set", { workflow: relabelled })).status === 200, "relabel over the API");
check(await otherShows(() => [...document.querySelectorAll("#board .col h2")].some((h) => h.textContent === "Designing")),
  "an open client follows a relabel with no card event");
const row = dialog.locator("tr", { hasText: "design" }).first();
await row.getByRole("button", { name: "Remove" }).click();
await dialog.getByRole("button", { name: "Save columns" }).click();
await dialog.locator(".err", { hasText: "still has cards" }).waitFor();
check((await api("board_columns")).body.columns.some((c) => c.id === "design"), "a column holding cards is not removed");

// Empty it, then remove it.
await api("card_move", { id: card.id, to: "blocked" });
await dialog.getByRole("button", { name: "Discard changes" }).click();
await page.locator("#settings").getByRole("button", { name: "Close" }).click();
await page.reload();
await page.waitForFunction(() => document.querySelector("#actor").value);
await openColumns();
await dialog.locator("tr", { hasText: "design" }).first().getByRole("button", { name: "Remove" }).click();
await dialog.getByRole("button", { name: "Save columns" }).click();
await page.waitForFunction(() => !document.querySelector("#board .col[data-col=design]"));
check(!(await api("board_columns")).body.columns.some((c) => c.id === "design"), "an empty column is removed");
check(await otherShows(() => !document.querySelector("#board .col[data-col=design]")), "and leaves the open client's board");
check((await dialog.locator("tr", { hasText: "Backlog" }).first().getByRole("button", { name: "Remove" }).count()) === 0, "Backlog, Done and Blocked cannot be removed");

if (process.env.SHOT) await page.screenshot({ path: process.env.SHOT });
check(errors.length === 0, `no page errors ${errors.join("; ")}`);
await browser.close();
process.exit(failures.length ? 1 : 0);

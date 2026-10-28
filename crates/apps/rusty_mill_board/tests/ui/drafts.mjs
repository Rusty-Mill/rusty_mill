// Browser check of the card dialog: live updates must never erase what a person is typing.
//
//   rusty_mill_board --http 127.0.0.1:8796 &           # open board, loopback
//   node tests/ui/drafts.mjs "$(npm root -g)/playwright" http://127.0.0.1:8796
//
// Needs Playwright and a Chromium (set CHROMIUM=/path if it is not at /opt/pw-browsers/chromium).
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const BASE = process.argv[3] ?? "http://127.0.0.1:8796";

const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "other", role: "human", ...args }) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 700 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));

const card = (await api("card_add", { title: "Draft test", body: "original body" })).card;
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester"); // an open board has no credential, so the UI needs a name
await page.locator(".card", { hasText: "Draft test" }).click();
const dialog = page.locator("#dlg");
const [title, body, tags] = [dialog.locator("input").nth(0), dialog.locator("textarea"), dialog.locator("input").nth(1)];
const commentBox = dialog.locator("input").nth(2);

// 1. Unsaved edits and a half-typed comment survive unrelated board activity.
await title.fill("My unsaved title");
await body.fill("my unsaved body");
await commentBox.fill("half-typed comment");
await api("card_comment", { id: card.id, text: "a note from someone else" }); // bumps the card's rev
await api("card_add", { title: "an unrelated card" });                         // unrelated event
await sleep(3500);                                                             // > two polls
check((await title.inputValue()) === "My unsaved title", "unsaved title survives a remote comment and an unrelated card");
check((await body.inputValue()) === "my unsaved body", "unsaved body survives");
check((await commentBox.inputValue()) === "half-typed comment", "the half-typed comment survives");
check((await dialog.innerText()).includes("a note from someone else"), "the remote comment appears in the open dialog");
check(!(await dialog.locator(".banner").innerText()).includes("Someone else"), "no conflict is raised when only a comment moved");

// 2. The text changing under an unsaved edit raises a conflict and keeps the typed text.
await api("card_update", { id: card.id, title: "Changed elsewhere" });
await sleep(3500);
check((await title.inputValue()) === "My unsaved title", "typed title is kept when the card's title changes remotely");
check((await dialog.locator(".banner").innerText()).includes("Someone else changed this card"), "a conflict banner is shown");
await dialog.getByRole("button", { name: "Take theirs" }).click();
check((await title.inputValue()) === "Changed elsewhere", "'Take theirs' loads the remote title");

// 3. 'Keep mine' lets the save through on top of their change.
await title.fill("Mine wins");
await api("card_update", { id: card.id, title: "Changed again elsewhere" });
await sleep(3500);
await dialog.getByRole("button", { name: "Keep mine" }).click();
await dialog.getByRole("button", { name: "Save" }).click();
await sleep(800);
const saved = (await api("card_get", { id: card.id })).card;
check(saved.title === "Mine wins", "after 'Keep mine', Save applies the edit");

// 4. A stale save is refused rather than overwriting.
let refused = false;
try { await api("card_update", { id: card.id, rev: 0, title: "stale" }); } catch (e) { refused = /changed concurrently/.test(String(e)); }
check(refused, "a save with an old revision is refused by the server");

// 5. With nothing typed, the dialog follows the board.
await page.keyboard.press("Escape");
await page.locator(".card", { hasText: "Mine wins" }).click();
await api("card_update", { id: card.id, title: "Followed" });
await sleep(3500);
check((await title.inputValue()) === "Followed", "an unedited dialog follows a remote edit");

// 6. A save still in flight when the dialog is closed and another card opened must not
//    leak its result into the new dialog (it would carry the wrong revision).
const cardB = (await api("card_add", { title: "Second card", body: "b" })).card;
await page.keyboard.press("Escape");
let release;
const held = new Promise((r) => { release = r; });
await page.route("**/api/card_update", async (route) => { await held; await route.continue(); });
await page.locator(".card", { hasText: "Followed" }).click();
await title.fill("Saved from A");
await dialog.getByRole("button", { name: "Save" }).click();
await page.keyboard.press("Escape");
await page.locator(".card", { hasText: "Second card" }).click();
await title.fill("Second saved");                                             // typing in B when A's save lands
const late = page.waitForResponse("**/api/card_update");
release();
await late;
await sleep(300);
check(!(await dialog.locator(".banner").innerText()).includes("Someone else"), "A's late save does not raise a false conflict in B");
await dialog.getByRole("button", { name: "Save" }).click();
await sleep(800);
await page.unroute("**/api/card_update");
const got = (await api("card_get", { id: cardB.id })).card;
check(got.title === "Second saved", `saving card B after A's late response applies (title is "${got.title}")`);

check(errors.length === 0, `no page errors ${errors.join("; ")}`);
await browser.close();
if (failures.length) { console.error(`\n${failures.length} check(s) failed`); process.exit(1); }
console.log("\nall browser checks passed");

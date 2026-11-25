// Browser check of the date filter: a field (created / updated / moved) and a day range narrow the board,
// the address keeps them across a reload, and the chip clears them.
//
//   D=$(mktemp -d); rusty_mill_board --settings-dir $D --http 127.0.0.1:8799 2>$D/log &
//   node tests/ui/date_filter.mjs "$(npm root -g)/playwright" http://127.0.0.1:8799 "$(cat $D/first-admin.token)"
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const [BASE, TOKEN] = [process.argv[3], process.argv[4]];

const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };
const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json", authorization: `Bearer ${TOKEN}` }, body: JSON.stringify(args) });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const day = (offset) => { const d = new Date(); d.setDate(d.getDate() + offset); return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`; };

await api("card_add", { title: "dated card" });

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 900 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(`${BASE}/#token=${TOKEN}`);
await page.waitForSelector(".card");
const titles = async () => (await page.locator(".card b").allInnerTexts()).sort();
const settle = () => page.waitForTimeout(400);

check((await titles()).join() === "dated card", "no filter: the card shows");
check(await page.locator("#df-clear").isHidden(), "no chip without a filter");

for (const field of ["created", "updated", "moved"]) {
  await page.selectOption("#df-field", field);
  await page.fill("#df-from", day(0));
  await page.fill("#df-to", day(0));
  await settle();
  check((await titles()).length === 1, `${field} today: the card shows`);
  await page.fill("#df-from", day(1));
  await page.fill("#df-to", day(2));
  await settle();
  check((await titles()).length === 0, `${field} tomorrow on: no card`);
  await page.fill("#df-from", day(-5));
  await page.fill("#df-to", day(-1));
  await settle();
  check((await titles()).length === 0, `${field} before today: no card`);
}

// Another client changes the board while a filter is active: the match set follows without a reload.
for (const field of ["created", "moved"]) {
  await page.selectOption("#df-field", field);
  await page.fill("#df-from", day(0));
  await page.fill("#df-to", day(0));
  await settle();
  const before = (await titles()).length;
  await api("card_add", { title: `${field} elsewhere` });
  await page.waitForFunction((t) => [...document.querySelectorAll(".card b")].some((b) => b.textContent === t), `${field} elsewhere`, { timeout: 8000 }).catch(() => {});
  check((await titles()).includes(`${field} elsewhere`) && (await titles()).length === before + 1, `${field}: a card added by another client appears without a reload`);
}
await page.fill("#df-from", day(1));
await page.fill("#df-to", day(2));
await settle();
await api("card_add", { title: "outside the range" });
await page.waitForTimeout(3500);
check((await titles()).length === 0, "a card outside the range stays hidden after the board changes");

// An older request that fails after a newer one has rendered must not undo it. Requests with a from and no to
// are held back and then fail; the final selection (from and to) answers at once and shows no card.
await page.route("**/api/board_list", async (route) => {
  const body = JSON.parse(route.request().postData() ?? "{}");
  if (body.created_from != null && body.created_to == null) {
    await new Promise((r) => setTimeout(r, 1500));
    return route.fulfill({ status: 500, contentType: "application/json", body: '{"error":"boom"}' });
  }
  return route.continue();
});
await page.selectOption("#df-field", "created");
await page.fill("#df-to", "");
await page.fill("#df-from", day(1));
await page.fill("#df-to", day(2));
await settle();
check((await titles()).length === 0, "the newer match set is shown");
await page.waitForTimeout(2500);
check((await titles()).length === 0 && (await page.locator("#df-clear").isVisible()), "a late failure of an older request leaves it in place");
await page.unroute("**/api/board_list");

// The address keeps the filter; a reload restores it.
await page.selectOption("#df-field", "updated");
await page.fill("#df-from", day(1));
await page.fill("#df-to", "");
await settle();
check(new URL(page.url()).searchParams.get("date") === "updated" && new URL(page.url()).searchParams.get("from") === day(1), `the address holds the filter (${page.url()})`);
await page.reload();
await page.waitForSelector("#df-clear:not([hidden])");
check((await page.inputValue("#df-field")) === "updated" && (await page.inputValue("#df-from")) === day(1), "a reload restores the field and range");
check((await titles()).length === 0, "and the narrowed board");

// The chip clears it, address included.
await page.click("#df-clear");
await page.waitForSelector(".card");
check((await titles()).length === 4, "clearing brings every card back");
check(!new URL(page.url()).searchParams.has("date") && (await page.inputValue("#df-field")) === "", "the address and controls are clear");

check(errors.length === 0, `no page errors (${errors.join("; ")})`);
await browser.close();
if (failures.length) { console.error(`${failures.length} failed`); process.exit(1); }

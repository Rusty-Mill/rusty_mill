// Browser check of the Inbox with a flat projects file, where the `default` repository holds the
// Inbox (project `default`) and other projects too: the Inbox row shows and counts only the Inbox.
//
//   echo '{"rl": ["crates/apps/rocket_league"]}' > /tmp/flat.json
//   rusty_mill_board --projects /tmp/flat.json --http 127.0.0.1:8795 &
//   node tests/ui/inbox_flat.mjs "$(npm root -g)/playwright" http://127.0.0.1:8795
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const BASE = process.argv[3] ?? "http://127.0.0.1:8795";

const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "other", role: "human", ...args }) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };

await api("card_add", { title: "Unsorted card" });
await api("card_add", { title: "RL card", project: "rl" });

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 700 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");
const titles = async () => (await page.locator(".card b").allInnerTexts()).sort();
const rows = (await page.locator("#nav1 .item").allInnerTexts()).map((t) => t.replace(/\s+/g, " ").trim());

check(rows.join("|") === "All repositories 2|Inbox 1|default 2|rl 1", `the Inbox counts only its own cards; the repository and its other project stay (${rows.join("|")})`);
check((await page.locator(".card", { hasText: "Unsorted card" }).locator(".tag.proj").innerText()) === "Inbox", "an Inbox card's chip says Inbox");

await page.locator("#nav1 .item", { hasText: "Inbox" }).click();
check((await titles()).join() === "Unsorted card", "choosing the Inbox shows only cards with no other project");
await page.fill("#title", "Added in Inbox");
await page.click("#add");
await page.waitForFunction(() => [...document.querySelectorAll(".card b")].some((b) => b.textContent === "Added in Inbox"));
const inbox = (await api("board_list", { project: "default" })).cards.map((c) => c.title).sort();
check(inbox.join() === "Added in Inbox,Unsorted card", `a card added in the Inbox keeps project default (${inbox.join()})`);

await page.locator(".card", { hasText: "Added in Inbox" }).click();
check((await page.locator("#dlg option[value=default]").innerText()) === "Inbox", "the dialog offers project default as the Inbox");
await page.keyboard.press("Escape");

await page.locator("#nav1 .item", { hasText: /^default/ }).click();
check((await titles()).join() === "Added in Inbox,RL card,Unsorted card", "the default repository still shows every card in it");

check(errors.length === 0, `no page errors ${errors.join("; ")}`);
await browser.close();
if (failures.length) { console.error(`\n${failures.length} check(s) failed`); process.exit(1); }
console.log("\nall browser checks passed");

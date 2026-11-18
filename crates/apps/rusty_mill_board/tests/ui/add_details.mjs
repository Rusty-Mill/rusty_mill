// Browser check of #1577: the add bar's Details takes a description, tags and a parent for a new card.
//
//   rusty_mill_board --http 127.0.0.1:8796 &           # open board, loopback
//   node tests/ui/add_details.mjs "$(npm root -g)/playwright" http://127.0.0.1:8796
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const BASE = process.argv[3] ?? "http://127.0.0.1:8796";

const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "other", role: "human", ...args }) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await (await browser.newContext({ viewport: { width: 1300, height: 800 } })).newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));

const parent = (await api("card_add", { title: "Add parent" })).card;
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");
const find = async (title) => (await api("board_list", { view: "full" })).cards.find((c) => c.title === title);

await page.fill("#title", "Plain add");
await page.click("#add");
await page.waitForFunction(() => [...document.querySelectorAll(".card")].some((c) => c.textContent.includes("Plain add")));
const plain = await find("Plain add");
check(plain && plain.body === "" && !(plain.tags ?? []).length && !plain.parent, "the bar alone still adds a bare card");

await page.click("#more summary");
await page.fill("#title", "Detailed add");
await page.fill("#nbody", "why it exists");
await page.fill("#ntags", "ui, parity");
await page.fill("#nparent", `#${parent.number}`);
await page.click("#add");
await page.waitForFunction(() => [...document.querySelectorAll(".card")].some((c) => c.textContent.includes("Detailed add")));
const made = await find("Detailed add");
check(made.body === "why it exists", "the description is stored");
check(made.tags.join() === "ui,parity", "the tags are stored");
check(made.parent === parent.id, "the parent is stored");
check((await page.inputValue("#title")) === "" && (await page.inputValue("#nbody")) === "", "the fields clear after a successful add");

await page.click("#more summary");
await page.fill("#title", "Bad parent");
await page.fill("#nparent", "#99999");
await page.click("#add");
await page.waitForSelector("#toast", { state: "visible" });
check((await find("Bad parent")) === undefined && (await page.inputValue("#title")) === "Bad parent", "a refused add keeps what was typed");
// A slow board: a repeated submit makes one card, and a draft typed meanwhile is not erased.
await page.route("**/api/card_add", async (route) => { await new Promise((r) => setTimeout(r, 600)); await route.continue(); });
await page.fill("#nparent", "");
await page.fill("#title", "Slow add");
const adds = [];
page.on("request", (r) => r.url().endsWith("/api/card_add") && adds.push(r));
await page.click("#add");
await page.click("#add", { force: true });
await page.press("#title", "Enter");
await page.fill("#title", "Next draft");
await page.waitForFunction(() => [...document.querySelectorAll(".card")].some((c) => c.textContent.includes("Slow add")));
await page.waitForTimeout(800);
const slow = (await api("board_list", { view: "full" })).cards.filter((c) => c.title === "Slow add");
check(adds.length === 1, "only one card_add request is sent: " + adds.length);
check(slow.length === 1, "a repeated submit while the request runs makes one card, not " + slow.length);
check((await page.inputValue("#title")) === "Next draft", "a draft typed while the request ran survives its response");
await page.unroute("**/api/card_add");
check(errors.length === 0, "no page errors: " + errors.join("; "));
await browser.close();
process.exit(failures.length ? 1 : 0);

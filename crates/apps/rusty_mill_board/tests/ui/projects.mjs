// Browser check of the repositories side panel: repositories with counts, a project dropdown for
// the chosen repository, filtering the board, adding into the selection, and moving a card
// between projects from its dialog.
//
//   cat > /tmp/p.json <<'J'
//   {"repos": {"rusty_mill": {"rl": ["crates/apps/rocket_league"], "orch": ["crates/apps/rusty_orch"]},
//              "super_board": {"skills": ["skills"]}}}
//   J
//   rusty_mill_board --projects /tmp/p.json --http 127.0.0.1:8797 &
//   node tests/ui/projects.mjs "$(npm root -g)/playwright" http://127.0.0.1:8797
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const BASE = process.argv[3] ?? "http://127.0.0.1:8797";

const api = async (op, args = {}) => {
  const r = await fetch(`${BASE}/api/${op}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actor: "other", role: "human", ...args }) });
  const j = await r.json();
  if (!r.ok) throw new Error(`${op}: ${j.error}`);
  return j;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };

await api("card_add", { title: "RL card", project: "rl" });
await api("card_add", { title: "Orch card", project: "orch" });
await api("card_add", { title: "Mill loose card", project: "rusty_mill" });
await api("card_add", { title: "Skills card", project: "skills" });

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 700 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");
const titles = async () => (await page.locator(".card b").allInnerTexts()).sort();
const repoButtons = async () => (await page.locator("#side .repo").allInnerTexts()).map((t) => t.replace(/\s+/g, " ").trim());

check((await titles()).length === 4, "all repositories show every card");
const rows = await repoButtons();
check(rows.join("|") === "All repositories 4|rusty_mill 3|super_board 1", `the panel lists repositories with counts (${rows.join("|")}); an empty default repository is hidden`);
check((await page.locator("#side select").count()) === 0, "no project dropdown until a repository is chosen");

await page.locator("#side .repo", { hasText: "super_board" }).click();
check((await titles()).join() === "Skills card", "choosing a repository filters the board");
await page.locator("#side .repo", { hasText: "rusty_mill" }).click();
check((await titles()).join() === "Mill loose card,Orch card,RL card", "another repository, its three cards");
const options = await page.locator("#side select option").allInnerTexts();
check(options.length === 4 && options[0].startsWith("all projects in rusty_mill"), `the dropdown lists that repository's projects (${options.join("|")})`);

await page.locator("#side select").selectOption("rl");
check((await titles()).join() === "RL card", "choosing a project narrows further");
check((await page.locator(".card .tag.proj").count()) === 0, "no project chip when one project is chosen");

await page.fill("#title", "Added under rl");
await page.click("#add");
await page.waitForFunction(() => [...document.querySelectorAll(".card b")].some((b) => b.textContent === "Added under rl"));
check((await api("board_list", { project: "rl" })).cards.some((c) => c.title === "Added under rl"), "a card added while a project is chosen belongs to it");

await page.locator("#side select").selectOption("");
await page.fill("#title", "Added under repo");
await page.click("#add");
await page.waitForFunction(() => [...document.querySelectorAll(".card b")].some((b) => b.textContent === "Added under repo"));
const own = (await api("board_list", { project: "rusty_mill" })).cards.map((c) => c.title);
check(own.includes("Added under repo"), "with only a repository chosen, it goes to the repository's own project");
check((await page.locator(".card .tag.proj").count()) === 5, "chips name each card's project when several are shown");

await page.locator(".card", { hasText: "Added under rl" }).click();
check((await page.locator("dialog optgroup").count()) === 3, "the dialog groups projects by repository");
await page.locator("dialog select").selectOption("skills");
await sleep(600);
const moved = (await api("board_list", { project: "skills" })).cards.map((c) => c.title);
check(moved.includes("Added under rl"), "the dialog moves a card to a project of another repository");
await page.keyboard.press("Escape");
check(!(await titles()).includes("Added under rl"), "it leaves the filtered view");

await page.reload();
await page.waitForSelector(".card");
check((await repoButtons()).some((t) => t.startsWith("rusty_mill")) && (await page.locator("#side .repo.on").innerText()).startsWith("rusty_mill"), "the chosen repository is remembered");

await page.click("#toggle");
check(!(await page.locator("#side").isVisible()), "the toggle hides the panel");
await page.click("#toggle");
check(await page.locator("#side").isVisible(), "and shows it again");

check(errors.length === 0, `no page errors ${errors.join("; ")}`);
await browser.close();
if (failures.length) { console.error(`\n${failures.length} check(s) failed`); process.exit(1); }
console.log("\nall browser checks passed");

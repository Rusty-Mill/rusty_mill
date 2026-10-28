// Browser check of the side panel: repositories (and agents) on the left, the chosen repository's
// projects in a second pane; filtering the board, adding into the selection, and moving a card
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
const orch = (await api("board_list", { project: "orch" })).cards[0];
await api("card_claim", { id: orch.id, actor: "claude", role: "builder" });

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 700 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(BASE);
await page.waitForSelector(".card");
await page.fill("#actor", "tester");
const titles = async () => (await page.locator(".card b").allInnerTexts()).sort();
const items = async (nav) => (await page.locator(`${nav} .item`).allInnerTexts()).map((t) => t.replace(/\s+/g, " ").trim());
const repoButtons = () => items("#nav1");

check((await titles()).length === 4, "all repositories show every card");
const rows = await repoButtons();
check(rows.slice(0, 3).join("|") === "All repositories 4|rusty_mill 3|super_board 1", `the panel lists repositories with counts (${rows.join("|")}); an empty default repository is hidden`);
check(rows.slice(3).join("|") === "Everyone 1|claude 1", `agents holding claims are listed with their counts (${rows.slice(3).join("|")})`);
check(await page.locator("#nav2").isHidden(), "no project pane until a repository is chosen");

await page.locator("#nav1 .item", { hasText: "super_board" }).click();
check((await titles()).join() === "Skills card", "choosing a repository filters the board");
await page.locator("#nav1 .item", { hasText: "rusty_mill" }).click();
check((await titles()).join() === "Mill loose card,Orch card,RL card", "another repository, its three cards");
const options = await items("#nav2");
check(options.join("|") === "All projects 3|orch 1|rl 1|rusty_mill 1", `the second pane lists that repository's projects (${options.join("|")})`);

await page.locator("#nav2 .item", { hasText: "rl" }).click();
check((await titles()).join() === "RL card", "choosing a project narrows further");
check((await page.locator(".card .tag.proj").count()) === 0, "no project chip when one project is chosen");

await page.fill("#title", "Added under rl");
await page.click("#add");
await page.waitForFunction(() => [...document.querySelectorAll(".card b")].some((b) => b.textContent === "Added under rl"));
check((await api("board_list", { project: "rl" })).cards.some((c) => c.title === "Added under rl"), "a card added while a project is chosen belongs to it");

await page.locator("#nav2 .item", { hasText: "All projects" }).click();
await page.fill("#title", "Added under repo");
await page.click("#add");
await page.waitForFunction(() => [...document.querySelectorAll(".card b")].some((b) => b.textContent === "Added under repo"));
const own = (await api("board_list", { project: "rusty_mill" })).cards.map((c) => c.title);
check(own.includes("Added under repo"), "with only a repository chosen, it goes to the repository's own project");
check((await page.locator(".card .tag.proj").count()) === 5, "chips name each card's project when several are shown");

await page.locator(".card", { hasText: "Added under rl" }).click();
check((await page.locator("#dlg optgroup").count()) === 3, "the dialog groups projects by repository");
await page.locator("#dlg select").selectOption("skills");
await sleep(600);
const moved = (await api("board_list", { project: "skills" })).cards.map((c) => c.title);
check(moved.includes("Added under rl"), "the dialog moves a card to a project of another repository");
await page.keyboard.press("Escape");
check(!(await titles()).includes("Added under rl"), "it leaves the filtered view");

await page.reload();
await page.waitForSelector(".card");
check((await page.locator("#nav1 .item.on", { hasText: "rusty_mill" }).count()) === 1, "the chosen repository is remembered");

// Each agent has one colour, shared by its card and its entry in the panel.
const swatch = (loc) => loc.evaluate((e) => getComputedStyle(e).backgroundColor);
const cardEdge = (title) => page.locator(".card", { hasText: title }).evaluate((e) => getComputedStyle(e).borderLeftColor);
const dotOf = (name) => swatch(page.locator("#nav1 .item", { hasText: name }).locator(".dot"));
const claudeDot = await dotOf("claude");
check((await cardEdge("Orch card")) === claudeDot, `a card an agent holds wears the agent's colour (${claudeDot})`);
check((await page.locator(".card", { hasText: "RL card" }).evaluate((e) => e.classList.contains("claimed"))) === false, "an unclaimed card has none");
check((await page.locator(".card", { hasText: "Orch card" }).locator(".tag.agent").innerText()) === "claude", "and names the agent");
await page.locator("#nav1 .item", { hasText: "claude" }).click();
check((await titles()).join() === "Orch card", "choosing an agent shows only the cards it holds");
await page.locator("#nav1 .item", { hasText: "Everyone" }).click();

// Agents never share a colour, even when their names hash close together.
const board = (await api("board_list")).cards;
for (const [title, actor] of [["Skills card", "codex"], ["Mill loose card", "gemini"], ["Added under repo", "mill-runner"], ["Orch card", "claude"]]) {
  await api("card_claim", { id: board.find((c) => c.title === title).id, actor, role: "builder" });
}
await page.locator("#nav1 .item", { hasText: "All repositories" }).click();
await page.waitForFunction(() => document.querySelectorAll("#nav1 .item .dot").length >= 4);
const colours = await Promise.all(["claude", "codex", "gemini", "mill-runner"].map(dotOf));
check(new Set(colours).size === 4, `four agents, four colours (${colours.join(" ")})`);

await page.click("#toggle");
check(!(await page.locator("#side").isVisible()), "the toggle hides the panel");
await page.click("#toggle");
check(await page.locator("#side").isVisible(), "and shows it again");

await api("card_add", { title: "Migrated card", project: "orch", tags: ["migrated:mill-board", "kept"] });
await page.reload();
await page.waitForSelector(".card");
const chips = await page.locator(".card", { hasText: "Migrated card" }).locator(".tag:not(.proj)").allInnerTexts();
check(chips.join() === "kept", `the migration marker is not shown as a chip (${chips.join()})`);

check(!(await page.locator("#toast").isVisible()), "a normal load shows no error");

// A refused project list is said so, not shown as an empty panel.
const refused = await browser.newPage({ viewport: { width: 1300, height: 700 } });
await refused.route("**/api/board_projects", (route) => route.fulfill({ status: 403, contentType: "application/json", body: '{"error":"not allowed"}' }));
await refused.goto(BASE);
await refused.waitForFunction(() => document.querySelector("#toast").textContent.includes("board_projects failed: not allowed"));
check(true, "a failed project list shows why");
await refused.close();

check(errors.length === 0, `no page errors ${errors.join("; ")}`);
await browser.close();
if (failures.length) { console.error(`\n${failures.length} check(s) failed`); process.exit(1); }
console.log("\nall browser checks passed");

// Regression: Confirm must save exactly the text the diff showed, even if the user types
// while the preview request is still out. Mocks the Tauri `invoke`; no backend, no npm deps.
// Run: node tests/ui/review-snapshot.cjs   (needs Playwright's `playwright` module resolvable)
const { chromium } = require("playwright");
const path = require("path");
const assert = require("assert");

(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage();
  await page.addInitScript(() => {
    window.__saved = null;
    window.__release = null;
    window.__TAURI__ = { core: { invoke: async (cmd, a) => {
      if (cmd === "list") return { tools: ["claude"], skipped: [], rows: [{ id: "demo", source: "/s", cells: [{ tool: "claude", state: "enabled" }] }] };
      if (cmd === "files") return { ownership: "source", files: [{ path: "SKILL.md", editable: true }] };
      if (cmd === "file") return { ownership: "source", text: "a\n", hash: "h" };
      if (cmd === "preview_edit") {
        await new Promise((r) => { window.__release = r; });
        return { ownership: "source", marks_edited: false, diff: [{ kind: "removed", text: "a" }, { kind: "added", text: a.text.trim() }] };
      }
      if (cmd === "apply_edit") { window.__saved = a.text; return null; }
    } } };
  });
  await page.goto("file://" + path.join(__dirname, "../../ui/index.html"));
  await page.click("text=Files…");
  await page.click("text=SKILL.md");
  await page.click("#ed-edit");
  await page.fill("#ed-text", "reviewed\n");
  await page.click("#ed-review");
  assert.ok(await page.evaluate(() => document.getElementById("ed-text").readOnly), "box is frozen while the preview is out");
  await page.locator("#ed-text").pressSequentially("MORE", { timeout: 1000 }).catch(() => {});
  assert.strictEqual(await page.inputValue("#ed-text"), "reviewed\n", "typing during the preview is refused");
  await page.evaluate(() => window.__release());
  await page.waitForSelector("#ed-save:not([hidden])");
  await page.click("#ed-save");
  await page.waitForFunction(() => window.__saved !== null);
  assert.strictEqual(await page.evaluate(() => window.__saved), "reviewed\n", "saved text equals reviewed text");
  await browser.close();
  console.log("ok");
})().catch((e) => { console.error(e); process.exit(1); });

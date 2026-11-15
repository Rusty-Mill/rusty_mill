// Push-to-source flow against a mocked Tauri `invoke`: the button shows only for an edited copy,
// the diff is reviewed before anything is sent, and Confirm sends exactly the hashes it showed.
// Run: node tests/ui/push-flow.cjs   (needs a resolvable `playwright`)
const { chromium } = require("playwright");
const path = require("path");
const assert = require("assert");

(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage();
  await page.addInitScript(() => {
    window.__calls = [];
    window.__state = "edited";
    window.__TAURI__ = { core: { invoke: async (cmd, a) => {
      window.__calls.push([cmd, a]);
      if (cmd === "list") return { tools: ["claude"], skipped: [], rows: [{ id: "demo", source: "/s", cells: [{ tool: "claude", state: window.__state }] }] };
      if (cmd === "files") return { ownership: a.tool ? "copy" : "source", files: [{ path: "SKILL.md", editable: true }] };
      if (cmd === "file") return { ownership: "copy", text: "x", hash: "h" };
      if (cmd === "preview_push") return { files: [{ path: "SKILL.md", source_hash: "s1", copy_hash: "c1", diff: [{ kind: "removed", text: "old" }, { kind: "added", text: "new" }] }], left: [{ path: "n.md", why: "only in the copy" }] };
      if (cmd === "apply_push") { window.__state = "enabled"; return { written: ["SKILL.md"], remarked: true, left: [] }; }
    } } };
  });
  await page.goto("file://" + path.join(__dirname, "../../ui/index.html"));
  const visible = (sel) => page.isVisible(sel);

  await page.click("text=Files…");
  assert.ok(!(await visible("#ed-push")), "no push for the source folder");
  await page.selectOption("#ed-place", "claude");
  await page.click("#ed-files >> text=SKILL.md");
  assert.ok(await visible("#ed-push"), "push offered for an edited copy");

  await page.click("#ed-push");
  const diff = await page.textContent("#ed-diff");
  assert.ok(diff.includes("- old") && diff.includes("+ new") && diff.includes("left alone: n.md"), diff);
  assert.ok((await page.textContent("#ed-foot")).includes("overwrites the source"), "overwrite warning");
  assert.ok(!(await visible("#ed-warn")), "no unsaved-changes prompt in a push review");
  assert.ok(!(await page.evaluate(() => window.__calls.some((c) => c[0] === "apply_push"))), "nothing sent before Confirm");

  await page.click("#ed-save");
  const sent = await page.evaluate(() => window.__calls.find((c) => c[0] === "apply_push")[1]);
  assert.deepStrictEqual(sent, { entry: "demo", tool: "claude", bases: [{ path: "SKILL.md", source_hash: "s1", copy_hash: "c1" }] });
  assert.ok((await page.textContent("#ed-foot")).includes("matches the source again"));
  await page.click("#ed-files >> text=SKILL.md");
  assert.ok(!(await visible("#ed-push")), "no push once the copy is current");

  await browser.close();
  console.log("push-flow ok");
})();

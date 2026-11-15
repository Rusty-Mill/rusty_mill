// Editor flow checks against a mocked Tauri `invoke` (no backend, no npm deps):
// unsaved text is never dropped silently, "changed on disk" has a way out, a shared source file
// is called out, and the footer does not keep "Saved." into the next edit.
// Run: node tests/ui/editor-flow.cjs   (needs a resolvable `playwright`)
const { chromium } = require("playwright");
const path = require("path");
const assert = require("assert");

(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage();
  await page.addInitScript(() => {
    window.__disk = "a\n";
    window.__stale = false;
    window.__TAURI__ = { core: { invoke: async (cmd, a) => {
      if (cmd === "list") return { tools: ["claude", "codex"], skipped: [], rows: [{ id: "demo", source: "/s", cells: [{ tool: "claude", state: "enabled" }, { tool: "codex", state: "enabled" }] }] };
      if (cmd === "files") return { ownership: "source", files: [{ path: "SKILL.md", editable: true }, { path: "b.md", editable: true }] };
      if (cmd === "file") return { ownership: "source", text: window.__disk, hash: window.__disk };
      if (cmd === "preview_edit") return { ownership: "source", marks_edited: false, diff: [{ kind: "added", text: a.text.trim() }] };
      if (cmd === "apply_edit") {
        if (a.hash !== window.__disk) throw "SKILL.md changed since it was read; reload it";
        window.__disk = a.text;
        return null;
      }
    } } };
  });
  await page.goto("file://" + path.join(__dirname, "../../ui/index.html"));
  const visible = (sel) => page.isVisible(sel);

  await page.click("text=Files…");
  assert.ok(!(await visible("#ed-warn")), "no prompt on open");
  await page.click("#ed-files >> text=SKILL.md");
  assert.ok((await page.textContent("#ed-note")).includes("Enabled for claude, codex: linked ones change"), "shared-file note");
  assert.ok(!(await visible("#ed-warn")), "no prompt just from opening a file");

  // Unsaved text: switching files, Close and Esc all ask first; Keep editing keeps the text.
  await page.click("#ed-edit");
  assert.strictEqual(await page.textContent("#ed-foot"), "Nothing is written until you confirm the diff.");
  await page.fill("#ed-text", "mine\n");
  await page.click("#ed-files >> text=b.md");
  assert.ok(await visible("#ed-warn"), "prompt on file switch");
  await page.click("#ed-keep");
  assert.ok(!(await visible("#ed-warn")), "Keep editing hides the prompt");
  assert.strictEqual(await page.inputValue("#ed-text"), "mine\n", "keep editing keeps the text");
  await page.click("#ed-close");
  assert.ok(await visible("#ed-warn"), "prompt on close");
  await page.click("#ed-keep");
  await page.keyboard.press("Escape");
  assert.ok(await visible("#ed-warn") && (await visible("#editor")), "prompt on Esc");
  await page.click("#ed-keep");

  // Changed on disk: save fails, text stays, "Discard and reload" is the way out.
  await page.evaluate(() => { window.__disk = "other\n"; });
  await page.click("#ed-review");
  await page.click("#ed-save");
  assert.ok((await page.textContent("#ed-error")).includes("changed since it was read"));
  assert.strictEqual(await page.inputValue("#ed-text"), "mine\n", "text kept after a failed save");
  await page.click("#ed-reload");
  assert.strictEqual(await page.textContent("#ed-view"), "other\n", "reload shows the disk text");
  assert.ok(!(await visible("#ed-warn")), "no prompt after reload");

  // Save, then the footer does not carry "Saved." into the next edit.
  await page.click("#ed-edit");
  await page.fill("#ed-text", "new\n");
  await page.click("#ed-review");
  await page.click("#ed-save");
  await page.waitForSelector("#ed-edit:not([hidden])");
  assert.strictEqual(await page.textContent("#ed-foot"), "Saved.");
  await page.click("#ed-edit");
  assert.strictEqual(await page.textContent("#ed-foot"), "Nothing is written until you confirm the diff.");

  // Discard path closes without further prompts once the text is dropped.
  await page.fill("#ed-text", "x\n");
  await page.click("#ed-close");
  await page.click("#ed-drop");
  assert.ok(!(await visible("#editor")), "discard closes the dialog");
  await browser.close();
  console.log("ok");
})().catch((e) => { console.error(e); process.exit(1); });

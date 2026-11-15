// Code widget checks: the tokenizer (pure, under Node) and the editor view against a mocked Tauri
// `invoke`: the colour layer mirrors the text, the gutter counts lines, scrolling stays in step,
// and nothing needs an inline style (the page's CSP forbids them).
// Run: node tests/ui/code-widget.cjs   (needs a resolvable `playwright`)
const { chromium } = require("playwright");
const path = require("path");
const assert = require("assert");
const Code = require("../../ui/code.js");

const classes = (text, p = "SKILL.md") => Code.tokenize(text, p).map((l) => l.map(([c]) => c).filter(Boolean));
const joined = (text, p = "SKILL.md") => Code.tokenize(text, p).map((l) => l.map(([, t]) => t).join("")).join("\n");

// Tokenizer: round trips every input, and colours frontmatter, headings, fences, lists and inline code.
const doc = "---\nname: demo\ndescription: Does `x`\n---\n# Title\n- item **bold** and `code`\n> quote\n```sh\n# not a heading\n```\nplain\n";
assert.strictEqual(joined(doc), doc.replace(/\n$/, "\n"), "tokens join back to the text");
assert.deepStrictEqual(classes(doc).slice(0, 5), [["tok-meta"], ["tok-key", "tok-meta"], ["tok-key", "tok-meta"], ["tok-meta"], ["tok-head"]]);
assert.deepStrictEqual(classes(doc)[5], ["tok-mark", "tok-bold", "tok-code"]);
assert.deepStrictEqual(classes(doc).slice(6, 10), [["tok-quote"], ["tok-fence"], ["tok-code"], ["tok-fence"]], "a # inside a fence is code");
assert.deepStrictEqual(classes("a\n---\nb"), [[], ["tok-meta"], []], "a rule outside frontmatter");
assert.deepStrictEqual(classes("# x", "run.sh"), [[]], "only Markdown is coloured");
assert.deepStrictEqual(classes("---\nname: x\n"), [["tok-meta"], ["tok-key", "tok-meta"], []], "unclosed frontmatter is tolerated");
const big = "# x\n".repeat(Code.MAX_LINES + 1);
assert.deepStrictEqual(classes(big).flat(), [], "a huge file is plain");
assert.strictEqual(joined(""), "", "empty text");
const crlf = "---\r\nname: x\r\n---\r\n# T\r\n";
assert.strictEqual(joined(crlf), crlf, "CRLF round trips");
assert.deepStrictEqual(classes(crlf).slice(0, 4), [["tok-meta"], ["tok-key", "tok-meta"], ["tok-meta"], ["tok-head"]], "CRLF keeps frontmatter and headings");

(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1100, height: 900 } });
  const lines = Array.from({ length: 120 }, (_, i) => (i === 0 ? "# Heading" : `line ${i} with \`code\``)).join("\n") + "\n";
  await page.addInitScript((text) => {
    window.__TAURI__ = { core: { invoke: async (cmd) => {
      if (cmd === "list") return { tools: ["claude"], skipped: [], rows: [{ id: "demo", source: "/s", cells: [{ tool: "claude", state: "enabled" }] }] };
      if (cmd === "files") return { ownership: "source", files: [{ path: "SKILL.md", editable: true }] };
      if (cmd === "file") return { ownership: "source", text, hash: "h" };
      if (cmd === "preview_edit") return { ownership: "source", marks_edited: false, diff: [] };
    } } };
  }, lines);
  await page.goto("file://" + path.join(__dirname, "../../ui/index.html"));
  await page.click("text=Files…");
  await page.click("#ed-files >> text=SKILL.md");

  assert.ok(await page.isVisible("#ed-gutter"), "gutter in view mode");
  assert.strictEqual(await page.textContent("#ed-view"), lines, "view text is unchanged by colouring");
  assert.strictEqual(await page.locator("#ed-view .tok-head").count(), 1);
  assert.ok((await page.textContent("#ed-gutter")).endsWith("121"), "one number per line, plus the empty last line");

  await page.click("#ed-edit");
  assert.strictEqual(await page.textContent("#ed-hl"), lines + "\n", "colour layer mirrors the textarea");
  await page.fill("#ed-text", "# a\n\n`b`\n");
  await page.waitForFunction(() => document.getElementById("ed-gutter").textContent === "1\n2\n3\n4");
  assert.strictEqual(await page.locator("#ed-hl .tok-code").count(), 1, "colours follow typing");
  assert.strictEqual(await page.locator("#ed-hl .tok-head").count(), 1);

  await page.fill("#ed-text", lines);
  await page.waitForFunction(() => document.getElementById("ed-gutter").textContent.endsWith("121"));
  await page.evaluate(() => { document.getElementById("ed-text").scrollTop = 400; });
  await page.waitForFunction(() => document.getElementById("ed-hl").scrollTop === 400 && document.getElementById("ed-gutter").scrollTop === 400);

  // The textarea and the colour layer put each line at the same height.
  await page.evaluate(() => { document.getElementById("ed-text").scrollTop = 0; });
  const ys = await page.evaluate(() => {
    const ta = document.getElementById("ed-text").getBoundingClientRect();
    const hl = document.getElementById("ed-hl").getBoundingClientRect();
    return [ta.top - hl.top, ta.left - hl.left, ta.width - hl.width];
  });
  assert.deepStrictEqual(ys, [0, 0, 0], "layers overlap exactly");
  assert.ok(await page.evaluate(() => document.getElementById("ed-code").classList.contains("live")), "colour layer live");
  // A huge file falls back to a plain opaque textarea: no colour layer to rebuild.
  await page.evaluate((n) => { const t = document.getElementById("ed-text"); t.value = "x".repeat(n); t.dispatchEvent(new Event("input")); }, Code.MAX_CHARS + 1);
  await page.waitForFunction(() => !document.getElementById("ed-code").classList.contains("live"));
  assert.ok(await page.isHidden("#ed-hl"), "colour layer hidden for a huge file");
  const inline = await page.evaluate(() => document.querySelectorAll("#editor [style]").length);
  assert.strictEqual(inline, 0, "no inline style attributes");

  await page.screenshot({ path: process.env.SHOT ?? "/tmp/code-widget.png" });
  await browser.close();
  console.log("code-widget ok");
})();

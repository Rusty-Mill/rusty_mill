import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';

/** Check generated production utilities, not just the theme's custom properties. */
export async function assertThemeCss(page, theme) {
  const palettes = {
    nebula: { bg: [10, 10, 15], accent: [76, 225, 247], text: [232, 232, 240] },
    cyberpunk: { bg: [11, 0, 20], accent: [255, 42, 109], text: [240, 230, 255] },
    minimal: { bg: [14, 14, 17], accent: [143, 168, 199], text: [222, 222, 228] },
  };
  const result = await page.evaluate(() => {
    // These classes are used by real components; no injected stylesheet or dev server.
    const probe = document.createElement('div');
    probe.className = 'bg-nebula-bg text-nebula-accent border border-nebula-accent/40 rounded-nebula-md shadow-nebula-soft font-nebula-command duration-nebula-fast ease-nebula';
    document.body.append(probe);
    const style = getComputedStyle(probe);
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = 1;
    const ctx = canvas.getContext('2d');
    const rgba = (color) => {
      ctx.clearRect(0, 0, 1, 1);
      ctx.fillStyle = color;
      ctx.fillRect(0, 0, 1, 1);
      return [...ctx.getImageData(0, 0, 1, 1).data];
    };
    const values = {
      bg: rgba(style.backgroundColor), accent: rgba(style.color), border: rgba(style.borderColor),
      radius: style.borderRadius, shadow: style.boxShadow, font: style.fontFamily,
      duration: style.transitionDuration, easing: style.transitionTimingFunction,
    };
    probe.className = 'text-nebula-text/60';
    values.opacityText = rgba(getComputedStyle(probe).color);
    probe.remove();
    return values;
  });
  const palette = palettes[theme];
  for (const [name, expected] of Object.entries({
    bg: [...palette.bg, 255], accent: [...palette.accent, 255],
    border: [...palette.accent, 102], opacityText: [...palette.text, 153],
  })) {
    expected.forEach((value, i) => assert.ok(Math.abs(result[name][i] - value) <= 1,
      `${theme} ${name}[${i}]: expected ${value}, got ${result[name][i]}`));
  }
  assert.equal(result.radius, '10px');
  // v4 serializes empty ring/inset layers before the configured visible shadow.
  assert.equal(result.shadow.replaceAll('rgba(0, 0, 0, 0) 0px 0px 0px 0px, ', ''),
    'rgba(0, 0, 0, 0.35) 0px 4px 12px 0px');
  assert.ok(result.font.includes('JetBrains Mono'));
  assert.equal(result.duration, '0.08s');
  assert.equal(result.easing, 'cubic-bezier(0.4, 0, 0.2, 1)');
  const card = page.locator('[data-testid="command-card"]').first();
  assert.equal(await card.evaluate((el) => getComputedStyle(el).borderRadius), '10px');
  if (process.env.THEME_SCREENSHOT_DIR) {
    await mkdir(process.env.THEME_SCREENSHOT_DIR, { recursive: true });
    await page.screenshot({ path: join(process.env.THEME_SCREENSHOT_DIR, `term-${theme}.png`), fullPage: true, animations: 'disabled' });
  }
  console.log(`ok: ${theme} production theme utilities, opacity, and card styles`);
}

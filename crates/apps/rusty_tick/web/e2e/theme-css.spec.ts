import { expect, test } from '@playwright/test'
import { TOKEN } from '../playwright.config'

// The configured server serves dist/, so these assertions exercise production CSS.
for (const theme of ['light', 'dark'] as const) {
  test(`${theme}: production theme, opacity, focus, popover and editor styles`, async ({ page }) => {
    await page.addInitScript((token) => {
      localStorage.setItem('tick-local:mode', 'server')
      sessionStorage.setItem('tick-local:token', token)
    }, TOKEN)
    await page.goto('/#/p/inbox/tasks?modalType=settings&tabs=appearance')
    await page.getByRole('radiogroup', { name: 'Theme' })
      .getByText(theme === 'dark' ? 'Dark' : 'Light', { exact: true }).click()
    await expect(page.getByRole('radio', { name: theme === 'dark' ? 'Dark' : 'Light', exact: true })).toBeChecked()
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme)
    const surface = theme === 'dark' ? 'rgb(26, 26, 28)' : 'rgb(255, 255, 255)'
    const primary = theme === 'dark' ? [92, 133, 255] : [71, 114, 250]
    const text = theme === 'dark' ? 'rgb(232, 232, 232)' : 'rgb(25, 25, 25)'
    const dialog = page.getByRole('dialog', { name: 'Settings' })
    await expect(dialog).toHaveCSS('background-color', surface)
    await expect(dialog).toHaveCSS('border-radius', '16px')
    await expect(dialog).toHaveCSS('color', text)
    await page.keyboard.press('Escape')

    // Probe actual component utilities without adding styles: catches lost @config
    // even when body defaults and custom properties alone still look correct.
    const values = await page.evaluate(() => {
      const el = document.createElement('div')
      el.className = 'bg-primary/10 text-primary border border-line rounded-row text-s shadow-pop'
      document.body.append(el)
      const s = getComputedStyle(el)
      const canvas = document.createElement('canvas')
      canvas.width = canvas.height = 1
      const ctx = canvas.getContext('2d')!
      ctx.fillStyle = s.backgroundColor
      ctx.fillRect(0, 0, 1, 1)
      const result = { opacity: [...ctx.getImageData(0, 0, 1, 1).data], color: s.color,
        radius: s.borderRadius, fontSize: s.fontSize, lineHeight: s.lineHeight, shadow: s.boxShadow }
      el.remove()
      return result
    })
    // Canvas unpremultiplies 8-bit channels: at 10% opacity RGB can round by 5.
    primary.forEach((channel, i) => expect(Math.abs(values.opacity[i]! - channel)).toBeLessThanOrEqual(5))
    expect(values.opacity[3]).toBe(26)
    expect(values.color).toBe(`rgb(${primary.join(', ')})`)
    expect(values.radius).toBe('8px')
    expect(values.fontSize).toBe('12px')
    expect(values.lineHeight).toBe('18px')
    expect(values.shadow).toContain('rgba(0, 0, 0, 0.12) 0px 4px 16px 0px')

    const title = `CSS ${theme} ${Date.now()}`
    await page.getByLabel('Add task').fill(title)
    await page.keyboard.press('Enter')
    await page.getByRole('list', { name: 'Tasks' }).getByText(title).click()
    await expect(page.getByLabel('Title', { exact: true })).toHaveCSS('color', text)
    const editor = page.getByRole('textbox', { name: 'Description', exact: true })
    await editor.fill('Theme regression notes')
    await expect(editor).toHaveCSS('font-size', '14px')
    await expect(editor).toHaveCSS('outline-style', 'none')
    // outline-hidden retains an accessible outline in forced-colors mode.
    await page.emulateMedia({ forcedColors: 'active' })
    await expect(editor).toHaveCSS('outline-style', 'solid')
    await expect(editor).toHaveCSS('outline-width', '2px')
    await page.emulateMedia({ forcedColors: 'none' })
    await page.getByRole('button', { name: 'Due Date', exact: true }).click()
    const popover = page.getByRole('dialog', { name: 'Due date' })
    await expect(popover).toHaveCSS('background-color', surface)
    await expect(popover).toHaveCSS('border-radius', '12px')
    await expect(popover).toHaveCSS('box-shadow', 'rgba(0, 0, 0, 0.12) 0px 4px 16px 0px')
    await page.screenshot({ path: test.info().outputPath(`${theme}-popover.png`), fullPage: true })
    await page.keyboard.press('Escape')
    await page.goto('/#/focus')
    const focusButton = page.getByRole('button', { name: 'Start', exact: true })
    await focusButton.focus()
    await expect(focusButton).toHaveCSS('background-color', `rgb(${primary.join(', ')})`)
    await expect(focusButton).toHaveCSS('outline-style', 'none')
    await expect(focusButton).toHaveCSS('box-shadow', new RegExp(`rgb\\(${primary.join(', ')}\\)`))
    await page.screenshot({ path: test.info().outputPath(`${theme}-focus.png`), fullPage: true })
  })
}

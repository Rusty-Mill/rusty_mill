import { expect, test } from '@playwright/test'
import { TOKEN } from '../playwright.config'

test.beforeEach(async ({ page }) => {
  await page.addInitScript((token) => {
    if (sessionStorage.getItem('tick-local:token')) return
    localStorage.setItem('tick-local:mode', 'server')
    sessionStorage.setItem('tick-local:token', token)
  }, TOKEN)
})

test('drag a task to another day in the month view', async ({ page }) => {
  const t = `move ${Math.random().toString(36).slice(2, 8)}`
  await page.goto('/#/p/inbox/tasks')
  await page.getByLabel('Add task').fill(`${t} today`)
  await page.keyboard.press('Enter')
  await expect(page.getByRole('list', { name: 'Tasks' }).getByText(t)).toBeVisible()

  await page.goto('/#/c/all/calendar/m')
  const bar = page.locator('button[data-task-id]', { hasText: t })
  // Bars are drawn over the day cells, not inside them: find the cell under the bar's centre.
  const dayUnder = (el: Element): string => {
    const r = el.getBoundingClientRect()
    const cells = [...document.querySelectorAll('[role="gridcell"]')]
    return cells.find((c) => { const b = c.getBoundingClientRect(); return r.left + r.width / 2 >= b.left && r.left + r.width / 2 <= b.right && r.top + r.height / 2 >= b.top && r.top + r.height / 2 <= b.bottom })?.getAttribute('data-day') ?? ''
  }
  const before = await bar.evaluate(dayUnder)
  expect(before).not.toBe('')
  await bar.dragTo(page.locator(`[role="gridcell"]:not([data-day="${before}"])`).nth(9))

  // It now sits on another day, and the server agrees after a reload.
  await page.reload()
  const moved = page.locator('button[data-task-id]', { hasText: t })
  await expect(moved).toBeVisible()
  expect(await moved.evaluate(dayUnder)).not.toBe(before)
})

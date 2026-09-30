import { expect, test } from '@playwright/test'
import { TOKEN } from '../playwright.config'

test.beforeEach(async ({ page }) => {
  await page.addInitScript((token) => {
    if (sessionStorage.getItem('tick-local:token')) return
    localStorage.setItem('tick-local:mode', 'server')
    sessionStorage.setItem('tick-local:token', token)
  }, TOKEN)
  await page.goto('/#/p/inbox/tasks')
})

test('move a task to the trash and restore it', async ({ page }) => {
  const t = `bin ${Math.random().toString(36).slice(2, 8)}`
  await page.getByLabel('Add task').fill(t)
  await page.keyboard.press('Enter')
  const list = page.getByRole('list', { name: 'Tasks' })
  await list.getByText(t).click()
  await page.getByRole('complementary', { name: 'Task details' }).getByRole('button', { name: 'More' }).click()
  await page.getByRole('menuitem', { name: 'Move to Trash' }).click()
  await expect(list.getByText(t)).toHaveCount(0)

  await page.goto('/#/q/all/trash')
  await expect(page.getByText(t)).toBeVisible()
  await page.getByText(t).hover() // the row's actions appear on hover or focus
  await page.getByRole('button', { name: `Restore ${t}` }).click()
  await page.goto('/#/p/inbox/tasks')
  await expect(page.getByRole('list', { name: 'Tasks' }).getByText(t)).toBeVisible()
})

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

const title = () => `e2e ${Math.random().toString(36).slice(2, 8)}`

test('add a task with quick add', async ({ page }) => {
  const t = title()
  await page.getByLabel('Add task').fill(`${t} tomorrow !high`)
  await page.keyboard.press('Enter')
  const list = page.getByRole('list', { name: 'Tasks' })
  await expect(list.getByText(t)).toBeVisible()
  await page.reload() // it is on the server, not just on screen
  await expect(page.getByRole('list', { name: 'Tasks' }).getByText(t)).toBeVisible()
})

test('complete a task', async ({ page }) => {
  const t = title()
  await page.getByLabel('Add task').fill(t)
  await page.keyboard.press('Enter')
  const list = page.getByRole('list', { name: 'Tasks' })
  await list.getByRole('checkbox', { name: t }).click()
  await expect(list.getByText(t)).toHaveCount(0)
  await page.goto('/#/q/all/completed')
  await expect(page.getByText(t)).toBeVisible()
})

test('open a task in the detail pane and rename it', async ({ page }) => {
  const t = title()
  await page.getByLabel('Add task').fill(t)
  await page.keyboard.press('Enter')
  await page.getByRole('list', { name: 'Tasks' }).getByText(t).click()
  await expect(page).toHaveURL(/#\/p\/inbox\/tasks\/[0-9a-f-]{36}$/)
  const box = page.getByLabel('Title')
  await expect(box).toHaveValue(t)
  await box.fill(`${t} renamed`)
  await box.blur()
  await expect(page.getByRole('list', { name: 'Tasks' }).getByText(`${t} renamed`)).toBeVisible()
})

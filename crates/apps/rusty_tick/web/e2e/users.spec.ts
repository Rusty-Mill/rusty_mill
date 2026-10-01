import { readFileSync } from 'node:fs'
import { expect, test } from '@playwright/test'
import { MULTI_PORT } from '../playwright.config'

const base = `http://127.0.0.1:${MULTI_PORT}`
const alice = () => readFileSync('.e2e-users/alice.token', 'utf8').trim()

test('a per-user token signs in, and a wrong secret is refused', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('tick-local:mode', 'server'))
  await page.goto(`${base}/#/p/inbox/tasks`)

  await page.getByLabel('API token').fill('alice.not-the-secret')
  await page.getByRole('button', { name: 'Connect' }).click()
  await expect(page.getByRole('alert')).toBeVisible()

  await page.getByLabel('API token').fill(alice())
  await page.getByRole('button', { name: 'Connect' }).click()
  await expect(page.getByLabel('Add task')).toBeVisible()

  const t = `alice ${Math.random().toString(36).slice(2, 8)}`
  await page.getByLabel('Add task').fill(t)
  await page.keyboard.press('Enter')
  await page.reload()
  await expect(page.getByRole('list', { name: 'Tasks' }).getByText(t)).toBeVisible()
})

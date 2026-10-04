import { expect, test, type Page } from '@playwright/test'

/**
 * The README's screenshots, taken on the real binary with a small family dealt
 * in. Runs after flows.spec.ts (alphabetical, serial), on the same data directory.
 */
test.describe.configure({ mode: 'serial' })
test.use({ viewport: { width: 1360, height: 860 }, colorScheme: 'light' })

const shot = (page: Page, name: string) => page.screenshot({ path: `docs/screenshots/${name}.png` })

test('deck with the detail pane', async ({ page }) => {
  await page.goto('/#/deck')
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'light'))
  await page.getByTestId('card-tile').filter({ hasText: /^Cleaning/ }).click()
  await expect(page.getByRole('complementary', { name: 'Card details' }).getByLabel('Name')).toHaveValue('Cleaning')
  await page.getByRole('button', { name: /Changes from the original/ }).click()
  await expect(page.getByText('This card matches the original deck card.')).toBeVisible()
  await shot(page, 'deck-detail')
})

test('balance', async ({ page }) => {
  await page.goto('/#/balance')
  await expect(page.getByTestId('balance-row')).toHaveCount(2)
  await shot(page, 'balance')
})

test('players', async ({ page }) => {
  await page.goto('/#/players')
  await expect(page.getByRole('list', { name: 'People' }).getByRole('listitem')).toHaveCount(2)
  await shot(page, 'players')
})

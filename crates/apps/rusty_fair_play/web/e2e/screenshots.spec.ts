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

test('choose the family deck, with the detail pane folded away', async ({ page }) => {
  await page.goto('/#/deck')
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'light'))
  await page.getByRole('button', { name: 'Choose cards' }).click()
  await page.getByRole('button', { name: 'Set aside every Unicorn Space card' }).click()
  await expect(page.getByRole('list', { name: 'Unicorn Space cards' }).getByRole('checkbox', { checked: false })).toHaveCount(2)
  await page.getByRole('complementary', { name: 'Card details' }).getByRole('button', { name: 'Hide details' }).click()
  await shot(page, 'choose-deck')
  // Leave the data as the other screenshots expect it.
  await page.getByRole('button', { name: 'Put every Unicorn Space card in the deck' }).click()
  await page.getByRole('button', { name: 'Done choosing' }).click()
})

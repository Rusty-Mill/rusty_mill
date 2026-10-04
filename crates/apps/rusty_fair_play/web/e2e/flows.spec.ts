import { expect, test, type Page } from '@playwright/test'

// One data directory for the whole file: the flows build on each other, in order.
test.describe.configure({ mode: 'serial' })

const pane = (page: Page) => page.getByRole('complementary', { name: 'Card details' })
const tile = (page: Page, name: RegExp) => page.getByTestId('card-tile').filter({ hasText: name })

test('the deck loads with 100 cards on six shelves', async ({ page }) => {
  await page.goto('/#/deck')
  await expect(page.getByTestId('card-tile')).toHaveCount(100)
  await expect(page.getByText('100 cards')).toBeVisible()
  for (const [suit, n] of [['Home', 22], ['Out', 22], ['Caregiving', 22], ['Magic', 22], ['Wild', 10], ['Unicorn Space', 2]] as const) {
    await expect(page.getByRole('heading', { name: `${suit} · ${n}` })).toBeVisible()
  }
})

test('add two people', async ({ page }) => {
  await page.goto('/#/players')
  await expect(page.getByText('Nobody yet')).toBeVisible()
  const input = page.getByLabel("New person's name")
  const people = page.getByRole('list', { name: 'People' }).getByRole('listitem')
  // Each add clears the input once the server answers; typing the next name before that would be wiped.
  await input.fill('Ada')
  await input.press('Enter')
  await expect(people).toHaveCount(1)
  await input.fill('Bob')
  await input.press('Enter')
  await expect(people).toHaveCount(2)
  await expect(people.nth(1)).toContainText('Player 2 · holds 0 cards (0 leaves)')
  await input.fill('Ada')
  await input.press('Enter')
  await expect(page.getByRole('alert')).toHaveText('Ada already exists')
  await page.reload() // on the server, not just on screen
  await expect(page.getByRole('list', { name: 'People' }).getByRole('listitem')).toHaveCount(2)
})

test('deal a card and see the owner chip', async ({ page }) => {
  await page.goto('/#/deck')
  await tile(page, /^Dishes/).click()
  await expect(page).toHaveURL(/#\/deck\/[0-9a-f-]{36}$/)
  await pane(page).getByLabel('Deal to').selectOption({ label: 'Ada' })
  await expect(tile(page, /^Dishes/).getByTestId('owner')).toContainText('Ada')
  await page.reload()
  await expect(tile(page, /^Dishes/).getByTestId('owner')).toContainText('Ada')
  await expect(pane(page).getByLabel('Deal to')).toHaveValue(/.+/)
})

test('split a card into two with different owners', async ({ page }) => {
  await page.goto('/#/deck')
  await tile(page, /^Cleaning/).click()
  await pane(page).getByLabel('Deal to').selectOption({ label: 'Ada' })
  await pane(page).getByRole('button', { name: 'Split…' }).click()
  const dialog = page.getByRole('dialog', { name: /Split/ })
  await dialog.getByLabel('Child 1 name').fill('Floors')
  await dialog.getByLabel('Child 1 owner').selectOption({ label: 'Bob' })
  await dialog.getByLabel('Child 2 name').fill('Bathrooms')
  await dialog.getByLabel('Child 2 owner').selectOption({ label: 'Ada' })
  await dialog.getByRole('button', { name: 'Split', exact: true }).click()
  await expect(dialog).toBeHidden()
  await expect(tile(page, /^Cleaning/)).toContainText('split · 2')
  const kids = pane(page).getByRole('list', { name: 'Child cards' }).getByRole('listitem')
  await expect(kids).toHaveCount(2)
  await expect(kids.nth(0)).toContainText('Floors')
  await expect(kids.nth(0)).toContainText('Bob')
  await expect(kids.nth(1)).toContainText('Bathrooms')
  await expect(page.getByTestId('card-tile')).toHaveCount(102)
  // Reorder with one order request, and open a child.
  await kids.nth(1).getByRole('button', { name: 'Move Bathrooms up' }).click()
  await expect(kids.nth(0)).toContainText('Bathrooms')
  await kids.nth(1).getByRole('link', { name: /Floors/ }).click()
  await expect(pane(page).getByLabel('Name')).toHaveValue('Floors')
  await expect(pane(page).getByRole('navigation', { name: 'Breadcrumb' })).toContainText('Cleaning')
  await expect(pane(page).getByText('Custom card', { exact: false }).first()).toBeVisible()
})

test('edit Execution, see the edited badge, reset, see it gone', async ({ page }) => {
  await page.goto('/#/deck')
  await tile(page, /^Dishes/).click()
  const original = await pane(page).getByLabel('Execution').inputValue()
  await pane(page).getByLabel('Execution').fill('Rinse, stack, run the machine every night.')
  await pane(page).getByRole('button', { name: 'Save' }).click()
  await expect(tile(page, /^Dishes/)).toContainText('edited')
  await pane(page).getByRole('button', { name: /Changes from the original/ }).click()
  const table = pane(page).getByRole('table', { name: 'Differences from the original' })
  await expect(table.getByRole('row')).toHaveCount(2)
  await expect(table).toContainText(original)
  await pane(page).getByRole('button', { name: 'Reset to original' }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Reset', exact: true }).click()
  await expect(tile(page, /^Dishes/)).not.toContainText('edited')
  await expect(pane(page).getByLabel('Execution')).toHaveValue(original)
  await expect(pane(page).getByText('This card matches the original deck card.')).toBeVisible()
  await expect(pane(page).getByLabel('Deal to')).toHaveValue(/.+/) // the owner survived the reset
})

test('the balance page counts leaves differently from all cards after the split', async ({ page }) => {
  await page.goto('/#/balance')
  const rows = page.getByTestId('balance-row')
  await expect(rows).toHaveCount(2)
  // Ada: Dishes, Cleaning (split parent), Bathrooms = 3 cards, 2 leaves.
  await expect(rows.nth(0)).toContainText('Ada')
  await expect(rows.nth(0).getByTestId('count-all')).toHaveText('3')
  await expect(rows.nth(0).getByTestId('count-leaves')).toHaveText('2')
  await expect(rows.nth(1).getByTestId('count-all')).toHaveText('1')
  await expect(rows.nth(1).getByTestId('count-leaves')).toHaveText('1')
  // 102 cards, 4 held (Dishes, Cleaning, Floors, Bathrooms), the parent is not a leaf: 98 undealt leaves.
  await expect(page.getByRole('heading', { name: 'Still undealt · 98' })).toBeVisible()
  await page.getByRole('list', { name: 'Undealt Home cards' }).getByLabel(/^Deal Laundry to$/).selectOption({ label: 'Bob' })
  await expect(page.getByRole('heading', { name: 'Still undealt · 97' })).toBeVisible()
  await expect(rows.nth(1).getByTestId('count-all')).toHaveText('2')
})

test('create a custom card from the board', async ({ page }) => {
  await page.goto('/#/deck')
  await page.getByRole('button', { name: 'New card…' }).click()
  const dialog = page.getByRole('dialog', { name: 'New card' })
  await dialog.getByLabel('Card name').fill('Dog walking')
  await dialog.getByLabel('Suit').selectOption('Out')
  await dialog.getByLabel('Owner').selectOption({ label: 'Ada' })
  await dialog.getByRole('button', { name: 'Create' }).click()
  await expect(dialog).toBeHidden()
  await expect(page).toHaveURL(/#\/deck\/[0-9a-f-]{36}$/)
  await expect(pane(page).getByLabel('Name')).toHaveValue('Dog walking')
  const dog = page.getByRole('list', { name: 'Out cards' }).getByTestId('card-tile').filter({ hasText: /^Dog walking/ })
  await expect(dog).toContainText('custom')
  await expect(dog.getByTestId('owner')).toContainText('Ada')
  await page.goto('/#/balance')
  const ada = page.getByTestId('balance-row').nth(0)
  await expect(ada.getByTestId('count-all')).toHaveText('4')
  await expect(ada.getByLabel('Ada by suit')).toContainText('Out 1')
})

test('re-parent and re-suit, split, reorder, unsplit, delete a card, and remove a person', async ({ page }) => {
  await page.goto('/#/deck')
  await tile(page, /^Dog walking/).click()
  // Suit and parent go in the same single save.
  await pane(page).getByLabel('Suit').selectOption('Home')
  await pane(page).getByLabel('Parent').selectOption({ label: '#3 Dishes' })
  await pane(page).getByRole('button', { name: 'Save' }).click()
  await expect(pane(page).getByRole('navigation', { name: 'Breadcrumb' })).toContainText('Dishes')
  await expect(page.getByRole('list', { name: 'Home cards' }).getByTestId('card-tile').filter({ hasText: /^Dog walking/ })).toBeVisible()
  await expect(tile(page, /^Dishes/)).toContainText('split · 1')

  await pane(page).getByRole('button', { name: 'Split…' }).click()
  const dialog = page.getByRole('dialog', { name: /Split/ })
  await dialog.getByLabel('Child 1 name').fill('Morning')
  await dialog.getByLabel('Child 2 name').fill('Evening')
  await dialog.getByRole('button', { name: 'Split', exact: true }).click()
  await expect(dialog).toBeHidden()
  const kids = pane(page).getByRole('list', { name: 'Child cards' }).getByRole('listitem')
  await expect(kids).toHaveCount(2)
  await expect(page.getByTestId('card-tile')).toHaveCount(105)
  await kids.nth(1).getByRole('button', { name: 'Move Evening up' }).click()
  await expect(kids.nth(0)).toContainText('Evening')
  await page.reload() // the order is on the server
  await expect(kids.nth(0)).toContainText('Evening')
  await expect(pane(page).getByRole('button', { name: 'Delete card' })).toBeDisabled()

  await pane(page).getByRole('button', { name: 'Unsplit…' }).click()
  const unsplit = page.getByRole('dialog', { name: 'Unsplit "Dog walking"?' })
  await expect(unsplit).toContainText('This removes 2 cards under it')
  await unsplit.getByRole('button', { name: 'Remove 2 cards' }).click()
  await expect(unsplit).toBeHidden()
  await expect(kids).toHaveCount(0)
  await expect(page.getByTestId('card-tile')).toHaveCount(103)

  await pane(page).getByRole('button', { name: 'Delete card' }).click()
  await page.getByRole('dialog', { name: 'Delete "Dog walking"?' }).getByRole('button', { name: 'Delete' }).click()
  // A child's delete lands on its parent.
  await expect(pane(page).getByLabel('Name')).toHaveValue('Dishes')
  await expect(page.getByTestId('card-tile')).toHaveCount(102)
  await expect(tile(page, /^Dishes/)).not.toContainText('split')
  await page.reload()
  await expect(page.getByTestId('card-tile')).toHaveCount(102)

  await page.goto('/#/players')
  const people = page.getByRole('list', { name: 'People' }).getByRole('listitem')
  await expect(people.nth(0).getByRole('button', { name: 'Remove Ada' })).toBeDisabled()
  const input = page.getByLabel("New person's name")
  await input.fill('Cleo')
  await input.press('Enter')
  await expect(people).toHaveCount(3)
  await people.nth(2).getByRole('button', { name: 'Remove Cleo' }).click()
  await page.getByRole('dialog', { name: 'Remove Cleo?' }).getByRole('button', { name: 'Remove' }).click()
  await expect(people).toHaveCount(2)
  await page.reload()
  await expect(page.getByRole('list', { name: 'People' }).getByRole('listitem')).toHaveCount(2)
})

test('two tabs edit the same card: the second save is refused, its draft is kept, and Overwrite takes it', async ({ browser }) => {
  const context = await browser.newContext()
  const one = await context.newPage()
  const two = await context.newPage()
  for (const page of [one, two]) {
    await page.goto('/#/deck')
    await tile(page, /^Laundry/).click()
    await expect(pane(page).getByLabel('Name')).toHaveValue('Laundry')
  }

  // Both tabs have read the card; the first saves its notes.
  await pane(one).getByLabel('Notes').fill('first tab')
  await pane(one).getByRole('button', { name: 'Save' }).click()
  await expect(pane(one).getByRole('button', { name: 'Save' })).toBeHidden()

  // The second tab, still on the old version, saves a different field. The server refuses it.
  await pane(two).getByLabel('Planning').fill('second tab plan')
  await pane(two).getByRole('button', { name: 'Save' }).click()
  await expect(two.getByRole('status')).toContainText('changed elsewhere')
  await expect(pane(two).getByRole('alert')).toContainText('changed elsewhere')
  await expect(pane(two).getByLabel('Planning')).toHaveValue('second tab plan') // the draft is kept
  await expect(pane(two).getByLabel('Notes')).toHaveValue('first tab') // the rest shows the new version

  // Nothing was written by the refused save.
  await two.reload()
  await expect(pane(two).getByLabel('Planning')).not.toHaveValue('second tab plan')

  // Typing again and choosing Overwrite lands it on top of the newer card.
  await pane(two).getByLabel('Planning').fill('second tab plan')
  await one.reload() // the first tab changes the card again before the second saves
  await pane(one).getByLabel('Notes').fill('first tab, again')
  await pane(one).getByRole('button', { name: 'Save' }).click()
  await expect(pane(one).getByRole('button', { name: 'Save' })).toBeHidden()
  await pane(two).getByRole('button', { name: 'Save' }).click()
  await expect(pane(two).getByRole('alert')).toBeVisible()
  await pane(two).getByRole('button', { name: 'Overwrite' }).click()
  await expect(pane(two).getByRole('alert')).toBeHidden()
  await two.reload()
  await expect(pane(two).getByLabel('Planning')).toHaveValue('second tab plan')
  await expect(pane(two).getByLabel('Notes')).toHaveValue('first tab, again')
  await context.close()
})

test('tiles are one size, the detail pane folds away, and the family chooses its deck', async ({ page }) => {
  await page.goto('/#/deck')
  await tile(page, /^Garbage/).click()
  await expect(pane(page).getByLabel('Name')).toHaveValue('Garbage')

  // Every tile on the board has the same width and height, whatever its name or badges.
  const sizes = await page.getByTestId('card-tile').evaluateAll((els) => [...new Set(els.map((e) => `${Math.round(e.getBoundingClientRect().width)}x${Math.round(e.getBoundingClientRect().height)}`))])
  expect(sizes).toHaveLength(1)

  // The pane folds to a rail and the board takes the room; opening a card brings it back.
  const board = page.getByRole('main', { name: 'Deck' })
  const before = (await board.boundingBox())!.width
  await pane(page).getByRole('button', { name: 'Hide details' }).click()
  await expect(pane(page).getByLabel('Name')).toBeHidden()
  expect((await board.boundingBox())!.width).toBeGreaterThan(before + 300)
  await page.reload() // remembered
  await expect(pane(page).getByRole('button', { name: 'Show details' })).toBeVisible()
  await tile(page, /^Mail/).click()
  await expect(pane(page).getByLabel('Name')).toHaveValue('Mail')

  // Choose the deck: put the Unicorn Space suit out, see it leave the board, and get it back.
  const total = await page.getByTestId('card-tile').count()
  await page.getByRole('button', { name: 'Choose cards' }).click()
  await page.getByRole('button', { name: 'Set aside every Unicorn Space card' }).click()
  await expect(page.getByRole('list', { name: 'Unicorn Space cards' }).getByRole('checkbox', { checked: true })).toHaveCount(0)
  await page.getByRole('button', { name: 'Done choosing' }).click()
  await expect(page.getByTestId('card-tile')).toHaveCount(total - 2)
  await expect(page.getByText('· 2 set aside')).toBeVisible()
  await page.reload() // on the server, not just on screen
  await expect(page.getByTestId('card-tile')).toHaveCount(total - 2)
  await page.getByRole('button', { name: 'Set aside · 2' }).click()
  await expect(page.getByTestId('card-tile')).toHaveCount(2)
  await page.getByRole('button', { name: 'Choose cards' }).click()
  await page.getByRole('button', { name: 'Put every Unicorn Space card in the deck' }).click()
  await page.getByRole('button', { name: 'Done choosing' }).click()
  await expect(page.getByTestId('card-tile')).toHaveCount(total)
})

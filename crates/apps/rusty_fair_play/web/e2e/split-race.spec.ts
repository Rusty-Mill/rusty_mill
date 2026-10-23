import { expect, test } from '@playwright/test'

test('split waits for an outstanding owner acknowledgement', async ({ page }, testInfo) => {
  const network: string[] = []
  const consoleMessages: string[] = []
  page.on('request', (request) => {
    if (request.url().includes('/api/')) network.push(`${request.method()} ${request.url()} If-Match=${request.headers()['if-match'] ?? ''}`)
  })
  page.on('response', (response) => {
    if (response.url().includes('/api/')) network.push(`${response.status()} ${response.url()}`)
  })
  page.on('console', (message) => consoleMessages.push(`${message.type()}: ${message.text()}`))
  page.on('pageerror', (error) => consoleMessages.push(error.message))
  const personResponse = await page.request.post('/api/v1/people', { data: { name: `Split race ${testInfo.repeatEachIndex}` } })
  expect(personResponse.ok()).toBe(true)
  const person = await personResponse.json() as { id: string; name: string }
  const cardResponse = await page.request.post('/api/v1/cards', { data: { name: 'Race card', suit: 'Home' } })
  expect(cardResponse.ok()).toBe(true)
  const card = await cardResponse.json() as { id: string }
  let release!: () => void
  const gate = new Promise<void>((resolve) => { release = resolve })
  let committed!: () => void
  const ownerCommitted = new Promise<void>((resolve) => { committed = resolve })
  await page.route(`**/api/v1/cards/${card.id}`, async (route) => {
    if (route.request().method() !== 'PATCH') return route.continue()
    const response = await route.fetch()
    expect(response.status()).toBe(200)
    network.push(`Owner PATCH committed; holding response: ${await response.text()}`)
    committed()
    await gate
    await route.fulfill({ response })
  })
  try {
    await page.goto(`/#/deck/${card.id}`)
    const pane = page.getByRole('complementary', { name: 'Card details' })
    await pane.getByLabel('Deal to').selectOption(person.id)
    await ownerCommitted
    await pane.getByRole('button', { name: 'Split…' }).click()
    const dialog = page.getByRole('dialog', { name: /Split/ })
    await dialog.getByLabel('Child 1 name').fill('First')
    await dialog.getByLabel('Child 2 name').fill('Second')
    const splitResponse = page.waitForResponse((response) => response.url().endsWith(`/cards/${card.id}/split`))
    await dialog.getByRole('button', { name: 'Split', exact: true }).click()
    // The user submitted while the owner response was still held. No split may
    // reach the server until that response has been merged into the store.
    expect(network.some((line) => line.startsWith('POST ') && line.includes('/split'))).toBe(false)
    release()
    expect((await splitResponse).status()).toBe(201)
    await expect(dialog).toBeHidden()
    await expect(pane.getByRole('list', { name: 'Child cards' }).getByRole('listitem')).toHaveCount(2)
    await page.reload()
    await expect(pane.getByLabel('Deal to')).toHaveValue(person.id)
    await expect(pane.getByRole('list', { name: 'Child cards' }).getByRole('listitem')).toHaveCount(2)
  } finally {
    release()
    await page.unrouteAll({ behavior: 'wait' })
    await testInfo.attach('network', { body: network.join('\n'), contentType: 'text/plain' })
    await testInfo.attach('console', { body: consoleMessages.join('\n'), contentType: 'text/plain' })
  }
})

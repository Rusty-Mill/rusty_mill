import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { renderApp, seedFamily } from '@/test/renderApp'
import { STALE_MESSAGE } from '@/store/data'

describe('the board', () => {
  it('does not rebase a confirmed suit selection after a background refresh', async () => {
    const user = userEvent.setup()
    const { api, services } = await renderApp('/deck', async (api) => {
      const { ada } = await seedFamily(api)
      const first = (await api.snapshot()).cards.find((c) => c.suit === 'Home')!
      await api.updateCard(first.id, { ownerId: ada })
    })
    const selected = (await api.snapshot()).cards.find((c) => c.suit === 'Home')!
    await user.click(screen.getByRole('button', { name: 'Choose cards' }))
    await user.click(screen.getByRole('button', { name: 'Set aside every Home card' }))
    const bob = (await api.snapshot()).people.find((p) => p.name === 'Bob')!
    await api.updateCard(selected.id, { ownerId: bob.id }, selected.etag)
    await services.store.getState().refresh()
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Set aside' }))
    await waitFor(() => expect(services.store.getState().toasts.at(-1)?.message).toBe(STALE_MESSAGE))
    expect(await api.getCard(selected.id)).toMatchObject({ inPlay: true, ownerId: bob.id })
    expect((await api.snapshot()).cards.every((c) => c.inPlay)).toBe(true)
  })

  it('shows six shelves with counts and a tile per card', async () => {
    await renderApp('/deck', async (api) => void (await seedFamily(api)))
    expect(screen.getByRole('heading', { name: /Home · 22/ })).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: /Unicorn Space · 2/ })).toBeInTheDocument()
    expect(screen.getAllByTestId('card-tile')).toHaveLength(100)
    expect(screen.getByText('100 cards')).toBeInTheDocument()
    expect(within(screen.getByRole('list', { name: 'Home cards' })).getAllByText('Unassigned')).toHaveLength(22)
  })

  it('offers to load the deck when there is none', async () => {
    const user = userEvent.setup()
    await renderApp('/deck')
    await user.click(screen.getByRole('button', { name: 'Load the Fair Play deck' }))
    await waitFor(() => expect(screen.getAllByTestId('card-tile')).toHaveLength(100))
  })

  it('filters by suit chips, owner chips, state, leaves and search', async () => {
    const user = userEvent.setup()
    await renderApp('/deck', async (api) => {
      const { ada } = await seedFamily(api)
      const cards = (await api.snapshot()).cards
      await api.updateCard(cards[0]!.id, { ownerId: ada, execution: 'ours' })
      await api.split(cards[1]!.id, { children: [{ name: 'Floors', ownerId: ada }] })
    })
    expect(screen.getAllByTestId('card-tile')).toHaveLength(101)
    await user.click(screen.getByRole('button', { name: 'Home', pressed: false }))
    expect(screen.getAllByTestId('card-tile')).toHaveLength(23)
    expect(screen.getByText('23 of 101 cards')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Home', pressed: true })) // toggle off
    await user.click(within(screen.getByRole('group', { name: 'Owner' })).getByRole('button', { name: /Ada/ }))
    expect(screen.getAllByTestId('card-tile').map((t) => t.textContent)).toEqual([expect.stringContaining('Childcare Helpers'), expect.stringContaining('Floors')])
    await user.click(screen.getByRole('button', { name: 'Clear' }))
    await user.click(screen.getByRole('button', { name: 'Edited' }))
    expect(screen.getAllByTestId('card-tile')).toHaveLength(1)
    expect(screen.getByText('edited')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Custom' }))
    expect(screen.getAllByTestId('card-tile')[0]).toHaveTextContent('custom')
    await user.click(screen.getByRole('button', { name: 'All' }))
    await user.click(screen.getByRole('checkbox', { name: 'Leaves only' }))
    expect(screen.getAllByTestId('card-tile')).toHaveLength(100)
    expect(screen.queryByText('split · 1')).toBeNull()
    await user.click(screen.getByRole('checkbox', { name: 'Leaves only' }))
    expect(screen.getByText('split · 1')).toBeInTheDocument()
    await user.type(screen.getByRole('searchbox', { name: 'Search cards' }), 'dish')
    expect(screen.getAllByTestId('card-tile')).toHaveLength(1)
    expect(screen.getAllByTestId('card-tile')[0]).toHaveTextContent('Dishes')
  })

  it('focuses search on "/" and opens a tile into the pane; Escape closes it', async () => {
    const user = userEvent.setup()
    const { router } = await renderApp('/deck', async (api) => void (await seedFamily(api)))
    await user.keyboard('/')
    expect(screen.getByRole('searchbox', { name: 'Search cards' })).toHaveFocus()
    await user.click(screen.getByRole('link', { name: /Dishes/ }))
    expect(router.state.location.pathname).toMatch(/^\/deck\/.+/)
    const pane = screen.getByRole('complementary', { name: 'Card details' })
    expect(within(pane).getByLabelText('Name')).toHaveValue('Dishes')
    await user.click(document.body)
    await user.keyboard('{Escape}')
    expect(router.state.location.pathname).toBe('/deck')
  })

  it('shows a 404-ish pane for an unknown card', async () => {
    await renderApp('/deck/nope', async (api) => void (await seedFamily(api)))
    expect(screen.getByText('This card does not exist.')).toBeInTheDocument()
  })

  it('creates a custom card from the New card dialog and opens it', async () => {
    const user = userEvent.setup()
    const { api, router } = await renderApp('/deck', async (api) => void (await seedFamily(api)))
    await user.click(screen.getByRole('button', { name: 'New card…' }))
    const dialog = screen.getByRole('dialog', { name: 'New card' })
    await user.click(within(dialog).getByRole('button', { name: 'Create' }))
    expect(within(dialog).getByRole('alert')).toHaveTextContent('A name is needed.')
    expect((await api.snapshot()).cards).toHaveLength(100)
    await user.type(within(dialog).getByLabelText('Card name'), 'Dog walking')
    await user.selectOptions(within(dialog).getByLabelText('Suit'), 'Out')
    await user.selectOptions(within(dialog).getByLabelText('Owner'), 'Ada')
    await user.type(within(dialog).getByLabelText('Execution'), 'Twice a day')
    await user.type(within(dialog).getByLabelText('New standard'), 'Morning and evening{Enter}')
    await user.click(within(dialog).getByRole('button', { name: 'Create' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    const card = (await api.snapshot()).cards.find((c) => c.name === 'Dog walking')!
    expect(card).toMatchObject({ suit: 'Out', state: 'custom', execution: 'Twice a day', minimumStandardOfCare: ['Morning and evening'] })
    expect(card.ownerId).toBeTruthy()
    expect(router.state.location.pathname).toBe(`/deck/${card.id}`)
    const tile = within(screen.getByRole('list', { name: 'Out cards' })).getByRole('link', { name: /Dog walking/ })
    expect(tile).toHaveTextContent('custom')
    expect(tile).toHaveTextContent('Ada')
    // The pane re-mounts when the route and the store settle, so re-query it on every poll rather than holding one node.
    await waitFor(() => expect(within(screen.getByRole('complementary', { name: 'Card details' })).getByLabelText('Name')).toHaveValue('Dog walking'))
  })

  it('chooses the family deck: set cards aside one by one or by suit, ask before taking a dealt card, and bring them back', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/deck', async (api) => {
      const { ada } = await seedFamily(api)
      await api.updateCard((await api.snapshot()).cards.find((c) => c.number === 3)!.id, { ownerId: ada }) // Dishes, Home
    })
    await user.click(screen.getByRole('button', { name: 'Choose cards' }))
    expect(screen.getByText(/Pick the cards your family plays with/)).toBeInTheDocument()
    const unicorn = within(screen.getByRole('list', { name: 'Unicorn Space cards' }))
    await user.click(unicorn.getAllByRole('checkbox')[0]!)
    await waitFor(() => expect(unicorn.getAllByRole('checkbox', { checked: false })).toHaveLength(1))

    // A suit at once; Home has a dealt card, so it asks first.
    await user.click(screen.getByRole('button', { name: 'Set aside every Home card' }))
    const dialog = await screen.findByRole('dialog', { name: 'Set aside 22 cards?' })
    expect(dialog).toHaveTextContent('1 of them are dealt')
    await user.click(within(dialog).getByRole('button', { name: 'Set aside' }))
    await waitFor(() => expect(within(screen.getByRole('list', { name: 'Home cards' })).getAllByRole('checkbox', { checked: false })).toHaveLength(22))
    expect((await api.snapshot()).cards.filter((c) => !c.inPlay)).toHaveLength(23)
    expect((await api.snapshot()).cards.find((c) => c.number === 3)).toMatchObject({ ownerId: null, inPlay: false })

    await user.click(screen.getByRole('button', { name: 'Done choosing' }))
    expect(screen.getAllByTestId('card-tile')).toHaveLength(77) // the deck is what is in play
    expect(screen.getByText('77 cards')).toBeInTheDocument()
    expect(screen.getByText('· 23 set aside')).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: /^Home/ })).toBeNull()

    await user.click(screen.getByRole('button', { name: 'Set aside · 23' }))
    expect(screen.getAllByTestId('card-tile')).toHaveLength(23)
    await user.click(screen.getByRole('button', { name: 'Choose cards' }))
    await user.click(screen.getByRole('button', { name: 'Put every Home card in the deck' }))
    await waitFor(async () => expect((await api.snapshot()).cards.filter((c) => !c.inPlay)).toHaveLength(1))

    // Putting the last one back leaves the "set aside" view instead of stranding the user on an empty board.
    await user.click(screen.getByRole('button', { name: 'Done choosing' }))
    await user.click(screen.getByRole('button', { name: 'Set aside · 1' }))
    await user.click(screen.getByRole('button', { name: 'Choose cards' }))
    await user.click(screen.getByRole('button', { name: 'Put every Unicorn Space card in the deck' }))
    await user.click(screen.getByRole('button', { name: 'Done choosing' }))
    await waitFor(() => expect(screen.getAllByTestId('card-tile')).toHaveLength(100))
    expect(screen.queryByRole('button', { name: /^Set aside/ })).toBeNull()
  })

  it('keeps every tile the same size and folds the detail pane to a rail and back', async () => {
    const user = userEvent.setup()
    await renderApp('/deck', async (api) => void (await seedFamily(api)))
    for (const tile of screen.getAllByTestId('card-tile')) expect(tile.className).toContain('h-full w-full') // sized by the grid cell, not by the content
    await user.click(screen.getByRole('link', { name: /^Dishes/ }))
    const pane = () => screen.getByRole('complementary', { name: 'Card details' })
    await user.click(within(pane()).getByRole('button', { name: 'Hide details' }))
    expect(within(pane()).queryByLabelText('Execution')).toBeNull()
    await user.click(within(pane()).getByRole('button', { name: 'Show details' }))
    expect(within(pane()).getByLabelText('Execution')).toBeInTheDocument()
    await user.click(within(pane()).getByRole('button', { name: 'Hide details' }))
    await user.click(screen.getByRole('link', { name: /^Garbage/ })) // opening a card brings it back
    expect(within(pane()).getByLabelText('Name')).toHaveValue('Garbage')
  })
})

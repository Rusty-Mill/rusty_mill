import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { renderApp, seedFamily } from '@/test/renderApp'

describe('the board', () => {
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
})

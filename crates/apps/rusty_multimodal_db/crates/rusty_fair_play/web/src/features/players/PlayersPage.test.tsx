import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { renderApp, seedFamily } from '@/test/renderApp'

describe('players', () => {
  it('adds people, shows what they hold, and reports a duplicate', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/players', async (api) => {
      const { ada } = await seedFamily(api)
      const cards = (await api.snapshot()).cards
      await api.updateCard(cards[0]!.id, { ownerId: ada })
      await api.split(cards[0]!.id, { children: [{ name: 'Mornings', ownerId: ada }] })
    })
    const list = screen.getByRole('list', { name: 'People' })
    expect(within(list).getAllByRole('listitem')[0]).toHaveTextContent('Player 1 · holds 2 cards (1 leaf)')
    expect(within(list).getAllByRole('listitem')[1]).toHaveTextContent('holds 0 cards (0 leaves)')
    await user.type(screen.getByLabelText("New person's name"), 'Cleo{Enter}')
    await waitFor(() => expect(within(list).getAllByRole('listitem')).toHaveLength(3))
    expect((await api.snapshot()).people.map((p) => p.name)).toEqual(['Ada', 'Bob', 'Cleo'])
    await user.type(screen.getByLabelText("New person's name"), 'Ada{Enter}')
    expect(await screen.findByRole('alert')).toHaveTextContent('Ada already exists')
  })

  it('renames in place', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/players', async (api) => void (await seedFamily(api)))
    const field = screen.getByLabelText('Name of player 2')
    await user.clear(field)
    await user.type(field, 'Robert{Enter}')
    await waitFor(async () => expect((await api.snapshot()).people[1]?.name).toBe('Robert'))
    expect(screen.getByLabelText('Name of player 2')).toHaveValue('Robert')
  })

  it('says so when there is nobody', async () => {
    await renderApp('/players', (api) => api.seed().then(() => undefined))
    expect(screen.getByText(/Nobody yet/)).toBeInTheDocument()
  })
})

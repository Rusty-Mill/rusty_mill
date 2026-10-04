import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { renderApp, seedFamily } from '@/test/renderApp'

describe('balance', () => {
  it('links to players when there is nobody', async () => {
    await renderApp('/balance', (api) => api.seed().then(() => undefined))
    expect(screen.getByText(/No people yet/)).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Add players' })).toHaveAttribute('href', '/players')
    expect(screen.getByRole('heading', { name: /Still undealt · 12/ })).toBeInTheDocument()
  })

  it('shows all-cards and leaf-only counts, by suit, and the undealt leaves with a quick deal', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/balance', async (api) => {
      const { ada, bob } = await seedFamily(api)
      const cards = (await api.snapshot()).cards
      await api.updateCard(cards[1]!.id, { ownerId: ada })
      await api.split(cards[1]!.id, { children: [{ name: 'Floors', ownerId: bob }, { name: 'Bathrooms', ownerId: ada }] })
      await api.updateCard(cards[3]!.id, { ownerId: bob })
    })
    const rows = screen.getAllByTestId('balance-row')
    expect(rows).toHaveLength(2)
    expect(within(rows[0]!).getByTestId('count-all')).toHaveTextContent('2') // Cleaning + Bathrooms
    expect(within(rows[0]!).getByTestId('count-leaves')).toHaveTextContent('1')
    expect(within(rows[1]!).getByTestId('count-all')).toHaveTextContent('2') // Floors + Auto
    expect(within(rows[1]!).getByTestId('count-leaves')).toHaveTextContent('2')
    expect(within(rows[0]!).getByLabelText('Ada by suit')).toHaveTextContent('Home 2(1)')
    expect(within(rows[1]!).getByLabelText('Bob by suit')).toHaveTextContent('Out 1')

    expect(screen.getByRole('heading', { name: /Still undealt · 10/ })).toBeInTheDocument()
    const home = screen.getByRole('list', { name: 'Undealt Home cards' })
    expect(within(home).getAllByRole('listitem').map((li) => li.textContent)).toEqual([expect.stringContaining('Childcare Helpers'), expect.stringContaining('Dishes')])
    await user.selectOptions(within(home).getByLabelText('Deal Dishes to'), 'Ada')
    await waitFor(() => expect(screen.getByRole('heading', { name: /Still undealt · 9/ })).toBeInTheDocument())
    expect(within(rows[0]!).getByTestId('count-all')).toHaveTextContent('3')
    expect((await api.snapshot()).cards.find((c) => c.name === 'Dishes')?.ownerId).toBeTruthy()
  })
})

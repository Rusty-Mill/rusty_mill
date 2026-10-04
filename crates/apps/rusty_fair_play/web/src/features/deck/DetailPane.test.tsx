import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { MemoryAdapter } from '@/api/memory'
import { renderApp, seedFamily } from '@/test/renderApp'

const cleaningOf = async (api: MemoryAdapter) => (await api.snapshot()).cards.find((c) => c.number === 2)!
const pane = () => screen.getByRole('complementary', { name: 'Card details' })

describe('the detail pane', () => {
  it('deals a card with the owner select', async () => {
    const user = userEvent.setup()
    let id = ''
    const { api } = await renderApp('/deck', async (api) => {
      await seedFamily(api)
      id = (await cleaningOf(api)).id
    })
    await user.click(screen.getByRole('link', { name: /Cleaning/ }))
    await user.selectOptions(within(pane()).getByLabelText('Deal to'), 'Bob')
    await waitFor(() => expect(within(pane()).getAllByTestId('owner')[0]).toHaveTextContent('Bob'))
    expect((await api.getCard(id)).ownerId).toBeTruthy()
    const tile = screen.getByRole('link', { name: /Cleaning/ })
    expect(tile).toHaveTextContent('Bob')
    expect(within(pane()).getByText('original')).toBeInTheDocument()
  })

  it('edits execution and the standards in one save, shows the edited badge and the diff, then resets', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/deck', async (api) => void (await seedFamily(api)))
    const cleaning = await cleaningOf(api)
    await user.click(screen.getByRole('link', { name: /Cleaning/ }))
    expect(within(pane()).queryByRole('button', { name: 'Save' })).toBeNull()
    const exec = within(pane()).getByLabelText('Execution')
    await user.clear(exec)
    await user.type(exec, 'Our way')
    await user.click(within(pane()).getByRole('button', { name: 'Remove standard 1' }))
    await user.type(within(pane()).getByLabelText('New standard'), 'Sparkling{Enter}')
    await user.click(within(pane()).getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(within(pane()).getByText('edited')).toBeInTheDocument())
    const saved = await api.getCard(cleaning.id)
    expect(saved.execution).toBe('Our way')
    expect(saved.minimumStandardOfCare).toEqual([...cleaning.minimumStandardOfCare.slice(1), 'Sparkling'])
    expect(saved.conception).toBe(cleaning.conception) // untouched fields were not sent
    expect(screen.getByRole('link', { name: /Cleaning/ })).toHaveTextContent('edited')

    await user.click(within(pane()).getByRole('button', { name: /Changes from the original/ }))
    const table = await within(pane()).findByRole('table', { name: 'Differences from the original' })
    expect(within(table).getAllByRole('row').map((r) => (r as HTMLTableRowElement).cells[0]?.textContent)).toEqual(['Field', 'Execution', 'Minimum standard of care'])
    expect(within(table).getByText(cleaning.execution)).toBeInTheDocument()

    await user.click(within(pane()).getByRole('button', { name: 'Reset to original' }))
    await user.click(screen.getByRole('dialog').querySelector('button.bg-danger')!)
    await waitFor(() => expect(within(pane()).queryByText('edited')).toBeNull())
    expect(within(pane()).getByLabelText('Execution')).toHaveValue(cleaning.execution)
    expect(await within(pane()).findByText('This card matches the original deck card.')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: /Cleaning/ })).not.toHaveTextContent('edited')
  })

  it('cancel drops the draft; notes never make the card edited', async () => {
    const user = userEvent.setup()
    await renderApp('/deck', async (api) => void (await seedFamily(api)))
    await user.click(screen.getByRole('link', { name: /Dishes/ }))
    await user.type(within(pane()).getByLabelText('Planning'), ' more')
    await user.click(within(pane()).getByRole('button', { name: 'Cancel' }))
    expect(within(pane()).getByLabelText('Planning')).not.toHaveValue(expect.stringContaining(' more'))
    await user.type(within(pane()).getByLabelText('Notes'), 'remember the sponge')
    await user.click(within(pane()).getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(within(pane()).queryByRole('button', { name: 'Save' })).toBeNull())
    expect(within(pane()).getByText('original')).toBeInTheDocument()
  })

  it('renames in place with Enter and reverts with Escape', async () => {
    const user = userEvent.setup()
    await renderApp('/deck', async (api) => void (await seedFamily(api)))
    await user.click(screen.getByRole('link', { name: /Dishes/ }))
    const name = within(pane()).getByLabelText('Name')
    await user.clear(name)
    await user.type(name, 'Washing up{Enter}')
    await waitFor(() => expect(screen.getByRole('link', { name: /Washing up/ })).toBeInTheDocument())
    expect(within(pane()).getByText('edited')).toBeInTheDocument()
    await user.clear(name)
    await user.type(name, 'nope{Escape}')
    expect(name).toHaveValue('Washing up')
  })

  it('splits a card through the dialog, lists the children, and reorders them', async () => {
    const user = userEvent.setup()
    const { api, router } = await renderApp('/deck', async (api) => void (await seedFamily(api)))
    await user.click(screen.getByRole('link', { name: /Cleaning/ }))
    await user.click(within(pane()).getByRole('button', { name: 'Split…' }))
    const dialog = screen.getByRole('dialog', { name: /Split/ })
    expect(within(dialog).getByRole('button', { name: 'Split' })).toBeDisabled()
    await user.type(within(dialog).getByLabelText('Child 1 name'), 'Floors')
    await user.selectOptions(within(dialog).getByLabelText('Child 1 owner'), 'Bob')
    await user.type(within(dialog).getByLabelText('Child 2 name'), 'Bathrooms')
    await user.selectOptions(within(dialog).getByLabelText('Child 2 owner'), 'Ada')
    await user.click(within(dialog).getByRole('button', { name: 'Add row' }))
    expect(within(dialog).getByLabelText('Child 3 name')).toBeInTheDocument()
    await user.click(within(dialog).getByRole('button', { name: 'Remove child 3' }))
    await user.selectOptions(within(dialog).getByLabelText('Hand the parent to'), 'Unassigned')
    await user.click(within(dialog).getByRole('button', { name: 'Split' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())

    const list = within(pane()).getByRole('list', { name: 'Child cards' })
    expect(within(list).getAllByRole('listitem').map((li) => li.textContent)).toEqual([expect.stringContaining('Floors'), expect.stringContaining('Bathrooms')])
    expect(within(list).getAllByRole('listitem')[0]).toHaveTextContent('Bob')
    expect(screen.getByRole('link', { name: /Cleaning/ })).toHaveTextContent('split · 2')
    expect(within(pane()).getByLabelText('Deal to')).toHaveValue('')
    const cleaning = await cleaningOf(api)
    expect(cleaning.ownerId).toBeNull()
    const kids = (await api.snapshot()).cards.filter((c) => c.parentCardId === cleaning.id)
    expect(kids.map((c) => c.state)).toEqual(['custom', 'custom'])

    await user.click(within(list).getByRole('button', { name: 'Move Bathrooms up' }))
    await waitFor(() => expect(within(list).getAllByRole('listitem')[0]).toHaveTextContent('Bathrooms'))
    expect(within(list).getByRole('button', { name: 'Move Bathrooms up' })).toBeDisabled()

    await user.click(within(list).getByRole('link', { name: /Floors/ }))
    expect(router.state.location.pathname).toBe(`/deck/${kids[0]!.id}`)
    expect(within(pane()).getByText('Custom card: made by this family, with no deck original to compare against.')).toBeInTheDocument()
    expect(within(pane()).getByRole('navigation', { name: 'Breadcrumb' })).toHaveTextContent('DeckCleaningFloors')
    await user.click(within(within(pane()).getByRole('navigation', { name: 'Breadcrumb' })).getByRole('link', { name: 'Cleaning' }))
    expect(router.state.location.pathname).toBe(`/deck/${cleaning.id}`)
  })
})

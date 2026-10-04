import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { MemoryAdapter } from '@/api/memory'
import { STALE_MESSAGE } from '@/store/data'
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
    await user.click(screen.getByRole('link', { name: /^Cleaning/ }))
    await user.selectOptions(within(pane()).getByLabelText('Deal to'), 'Bob')
    await waitFor(() => expect(within(pane()).getAllByTestId('owner')[0]).toHaveTextContent('Bob'))
    expect((await api.getCard(id)).ownerId).toBeTruthy()
    const tile = screen.getByRole('link', { name: /^Cleaning/ })
    expect(tile).toHaveTextContent('Bob')
    expect(within(pane()).getByText('original')).toBeInTheDocument()
  })

  it('edits execution and the standards in one save, shows the edited badge and the diff, then resets', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/deck', async (api) => void (await seedFamily(api)))
    const cleaning = await cleaningOf(api)
    await user.click(screen.getByRole('link', { name: /^Cleaning/ }))
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
    expect(screen.getByRole('link', { name: /^Cleaning/ })).toHaveTextContent('edited')

    await user.click(within(pane()).getByRole('button', { name: /Changes from the original/ }))
    const table = await within(pane()).findByRole('table', { name: 'Differences from the original' })
    expect(within(table).getAllByRole('row').map((r) => (r as HTMLTableRowElement).cells[0]?.textContent)).toEqual(['Field', 'Execution', 'Minimum standard of care'])
    expect(within(table).getByText(cleaning.execution)).toBeInTheDocument()

    await user.click(within(pane()).getByRole('button', { name: 'Reset to original' }))
    await user.click(screen.getByRole('dialog').querySelector('button.bg-danger')!)
    await waitFor(() => expect(within(pane()).queryByText('edited')).toBeNull())
    expect(within(pane()).getByLabelText('Execution')).toHaveValue(cleaning.execution)
    expect(await within(pane()).findByText('This card matches the original deck card.')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: /^Cleaning/ })).not.toHaveTextContent('edited')
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

  it('renames against the edit-start version and preserves both deliberate conflict choices', async () => {
    const user = userEvent.setup()
    const { api, services } = await renderApp('/deck', async (api) => void (await seedFamily(api)))
    await user.click(screen.getByRole('link', { name: /Dishes/ }))
    const dishes = (await api.snapshot()).cards.find((c) => c.number === 3)!
    const update = vi.spyOn(api, 'updateCard')
    const name = within(pane()).getByLabelText('Name')
    await user.clear(name)
    await user.type(name, 'My dishes')

    await api.updateCard(dishes.id, { name: 'Their dishes' }, dishes.etag) // the other client wins first
    const realSnapshot = api.snapshot.bind(api)
    let releaseRefresh!: () => void
    const refreshGate = new Promise<void>((resolve) => (releaseRefresh = resolve))
    vi.spyOn(api, 'snapshot').mockImplementationOnce(async () => {
      await refreshGate
      return realSnapshot()
    })
    const refresh = services.store.getState().refresh()
    releaseRefresh()
    await refresh
    expect(name).toHaveValue('My dishes')

    await user.type(name, '{Enter}')
    expect(await within(pane()).findByRole('alert')).toHaveTextContent(/name changed elsewhere/i)
    expect(name).toHaveValue('My dishes')
    expect(update.mock.calls.at(-1)?.[2]).toBe(dishes.etag)
    await user.click(within(pane()).getByRole('button', { name: 'Overwrite' }))
    await waitFor(() => expect(name).toHaveValue('My dishes'))
    await waitFor(async () => expect((await api.getCard(dishes.id)).name).toBe('My dishes'))

    await user.clear(name)
    await user.type(name, 'Second attempt')
    const ours = await api.getCard(dishes.id)
    await api.updateCard(dishes.id, { name: 'Third-party name' }, ours.etag)
    await services.store.getState().refresh()
    await user.type(name, '{Enter}')
    expect(await within(pane()).findByRole('alert')).toBeInTheDocument()
    expect(name).toHaveValue('Second attempt')
    await user.click(within(pane()).getByRole('button', { name: 'Discard mine' }))
    expect(name).toHaveValue('Third-party name')
    expect((await api.getCard(dishes.id)).name).toBe('Third-party name')
  })

  it('splits a card through the dialog, lists the children, and reorders them', async () => {
    const user = userEvent.setup()
    const { api, router } = await renderApp('/deck', async (api) => void (await seedFamily(api)))
    await user.click(screen.getByRole('link', { name: /^Cleaning/ }))
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
    expect(screen.getByRole('link', { name: /^Cleaning/ })).toHaveTextContent('split · 2')
    expect(within(pane()).getByLabelText('Deal to')).toHaveValue('')
    const cleaning = await cleaningOf(api)
    expect(cleaning.ownerId).toBeNull()
    const kids = (await api.snapshot()).cards.filter((c) => c.parentCardId === cleaning.id)
    expect(kids.map((c) => c.state)).toEqual(['custom', 'custom'])

    const reorder = vi.spyOn(api, 'reorderChildren')
    const position = vi.spyOn(api, 'setPosition')
    await user.click(within(list).getByRole('button', { name: 'Move Bathrooms up' }))
    await waitFor(() => expect(within(list).getAllByRole('listitem')[0]).toHaveTextContent('Bathrooms'))
    expect(within(list).getByRole('button', { name: 'Move Bathrooms up' })).toBeDisabled()
    expect(reorder).toHaveBeenCalledTimes(1) // one order request, not two position PUTs
    expect(reorder).toHaveBeenCalledWith(cleaning.id, [kids[1]!.id, kids[0]!.id], cleaning.treeEtag) // guarded by the subtree tag
    expect(position).not.toHaveBeenCalled()

    await user.click(within(list).getByRole('link', { name: /Floors/ }))
    expect(router.state.location.pathname).toBe(`/deck/${kids[0]!.id}`)
    expect(within(pane()).getByText('Custom card: made by this family, with no deck original to compare against.')).toBeInTheDocument()
    expect(within(pane()).getByRole('navigation', { name: 'Breadcrumb' })).toHaveTextContent('DeckCleaningFloors')
    await user.click(within(within(pane()).getByRole('navigation', { name: 'Breadcrumb' })).getByRole('link', { name: 'Cleaning' }))
    expect(router.state.location.pathname).toBe(`/deck/${cleaning.id}`)
  })

  it('changes suit and parent in one save; the parent list leaves out the card and what is under it', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/deck', async (api) => {
      await seedFamily(api)
      await api.split((await cleaningOf(api)).id, { children: [{ name: 'Floors' }] })
    })
    const cleaning = await cleaningOf(api)
    await user.click(screen.getByRole('link', { name: /^Cleaning/ }))
    const parents = within(pane()).getByLabelText('Parent')
    const labels = within(parents).getAllByRole('option').map((o) => o.textContent)
    expect(labels[0]).toBe('None (top level)')
    expect(labels).toContain('#3 Dishes')
    expect(labels).not.toContain('#2 Cleaning')
    expect(labels).not.toContain('#2 Cleaning › Floors')
    await user.click(within(pane()).getByRole('button', { name: 'Close' }))

    await user.click(screen.getByRole('link', { name: /^Dishes/ }))
    expect(within(pane()).getByLabelText('Suit')).toHaveValue('Home')
    const update = vi.spyOn(api, 'updateCard')
    await user.selectOptions(within(pane()).getByLabelText('Suit'), 'Out')
    await user.selectOptions(within(pane()).getByLabelText('Parent'), '#2 Cleaning › Floors')
    await user.click(within(pane()).getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(within(pane()).getByRole('navigation', { name: 'Breadcrumb' })).toHaveTextContent('DeckCleaningFloorsDishes'))
    expect(update).toHaveBeenCalledTimes(1)
    const floors = (await api.snapshot()).cards.find((c) => c.name === 'Floors')!
    expect(update.mock.calls[0]?.slice(0, 2)).toEqual([expect.any(String), { suit: 'Out', parentCardId: floors.id }])
    expect(await api.getCard(update.mock.calls[0]![0])).toMatchObject({ suit: 'Out', parentCardId: floors.id, state: 'edited' })
    expect(within(screen.getByRole('list', { name: 'Out cards' })).getByRole('link', { name: /^Dishes/ })).toBeInTheDocument()
    expect(cleaning.parentCardId).toBeNull()
  })

  it('unsplits after a confirm that counts the cards, then deletes the card and lands on the deck', async () => {
    const user = userEvent.setup()
    const { api, router } = await renderApp('/deck', async (api) => {
      await seedFamily(api)
      const { children } = await api.split((await cleaningOf(api)).id, { children: [{ name: 'Floors' }, { name: 'Walls' }] })
      await api.split(children[0]!.id, { children: [{ name: 'Mopping' }] })
    })
    const cleaning = await cleaningOf(api)
    await user.click(screen.getByRole('link', { name: /^Cleaning/ }))
    expect(within(pane()).getByRole('button', { name: 'Delete card' })).toBeDisabled()
    expect(within(pane()).getByText('Unsplit first: it has 2 children')).toBeInTheDocument()
    await user.click(within(pane()).getByRole('button', { name: 'Unsplit…' }))
    const dialog = screen.getByRole('dialog', { name: 'Unsplit "Cleaning"?' })
    expect(dialog).toHaveTextContent('This removes 3 cards under it')
    await user.click(within(dialog).getByRole('button', { name: 'Remove 3 cards' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(within(pane()).queryByRole('list', { name: 'Child cards' })).toBeNull()
    expect(screen.getAllByTestId('card-tile')).toHaveLength(100)
    expect(screen.getByRole('link', { name: /^Cleaning/ })).not.toHaveTextContent('split')
    expect(within(pane()).queryByRole('button', { name: 'Unsplit…' })).toBeNull()

    const del = within(pane()).getByRole('button', { name: 'Delete card' })
    expect(del).toBeEnabled()
    await user.click(del)
    await user.click(within(screen.getByRole('dialog', { name: 'Delete "Cleaning"?' })).getByRole('button', { name: 'Delete' }))
    await waitFor(() => expect(router.state.location.pathname).toBe('/deck'))
    expect(screen.queryByRole('link', { name: /^Cleaning/ })).toBeNull()
    expect(screen.getAllByTestId('card-tile')).toHaveLength(99)
    expect((await api.snapshot()).cards.some((c) => c.id === cleaning.id)).toBe(false)
  })

  it('deleting a child lands on its parent', async () => {
    const user = userEvent.setup()
    const floors = '00000000-0000-4000-8000-00000000f100'
    const { api, router } = await renderApp(`/deck/${floors}`, async (api) => {
      await seedFamily(api)
      await api.split((await cleaningOf(api)).id, { children: [{ id: floors, name: 'Floors' }] })
    })
    const cleaning = await cleaningOf(api)
    await user.click(within(pane()).getByRole('button', { name: 'Delete card' }))
    await user.click(screen.getByRole('dialog').querySelector('button.bg-danger')!)
    await waitFor(() => expect(router.state.location.pathname).toBe(`/deck/${cleaning.id}`))
    await waitFor(() => expect(within(pane()).getByLabelText('Name')).toHaveValue('Cleaning'))
  })

  it('on a save that lost a race, reloads the card, keeps the draft, and says so', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/deck', async (api) => void (await seedFamily(api)))
    await user.click(screen.getByRole('link', { name: /^Dishes/ }))
    const dishes = (await api.snapshot()).cards.find((c) => c.number === 3)!
    await user.type(within(pane()).getByLabelText('Notes'), 'mine')
    await api.updateCard(dishes.id, { execution: 'theirs' }) // someone else, meanwhile
    await user.click(within(pane()).getByRole('button', { name: 'Save' }))
    expect(await screen.findByRole('status')).toHaveTextContent(STALE_MESSAGE)
    await waitFor(() => expect(within(pane()).getByLabelText('Execution')).toHaveValue('theirs'))
    expect(within(pane()).getByLabelText('Notes')).toHaveValue('mine') // not thrown away
    expect((await api.getCard(dishes.id)).notes).toBe('')
    expect(within(pane()).getByRole('alert')).toHaveTextContent(/changed elsewhere/)
    await user.click(within(pane()).getByRole('button', { name: 'Overwrite' }))
    await waitFor(() => expect(within(pane()).queryByRole('button', { name: 'Overwrite' })).toBeNull())
    expect(await api.getCard(dishes.id)).toMatchObject({ notes: 'mine', execution: 'theirs' })
  })

  it('warns before a save that would overwrite a change a refresh brought in, and Discard mine takes theirs', async () => {
    const user = userEvent.setup()
    const { api, services } = await renderApp('/deck', async (api) => void (await seedFamily(api)))
    await user.click(screen.getByRole('link', { name: /^Dishes/ }))
    const dishes = (await api.snapshot()).cards.find((c) => c.number === 3)!
    await user.type(within(pane()).getByLabelText('Planning'), ' mine')
    await api.updateCard(dishes.id, { planning: 'theirs' }) // another tab saves the same field first
    await services.store.getState().refresh() // the 30 s refresh lands before this tab saves
    expect(await within(pane()).findByRole('alert')).toHaveTextContent(/changed elsewhere/)
    expect(within(pane()).getByLabelText('Planning')).toHaveValue(`${dishes.planning} mine`) // typed text is kept
    expect(within(pane()).queryByRole('button', { name: 'Save' })).toBeNull() // no quiet Save: it is an Overwrite now
    await user.click(within(pane()).getByRole('button', { name: 'Discard mine' }))
    await waitFor(() => expect(within(pane()).getByLabelText('Planning')).toHaveValue('theirs'))
    expect(within(pane()).queryByRole('alert')).toBeNull()
    expect((await api.getCard(dishes.id)).planning).toBe('theirs')
  })

  it('keeps what is typed while a save is in flight', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/deck', async (api) => void (await seedFamily(api)))
    await user.click(screen.getByRole('link', { name: /^Dishes/ }))
    const dishes = (await api.snapshot()).cards.find((c) => c.number === 3)!
    const real = api.updateCard.bind(api)
    let release!: () => void
    const gate = new Promise<void>((resolve) => (release = resolve))
    vi.spyOn(api, 'updateCard').mockImplementationOnce(async (id, patch, etag) => {
      const saved = await real(id, patch, etag)
      await gate // the answer is slow
      return saved
    })
    await user.type(within(pane()).getByLabelText('Execution'), ' mine')
    await user.click(within(pane()).getByRole('button', { name: 'Save' }))
    await user.type(within(pane()).getByLabelText('Notes'), 'typed meanwhile') // fields stay editable
    release()
    await waitFor(() => expect(within(pane()).getByLabelText('Execution')).toHaveValue(`${dishes.execution} mine`))
    expect(within(pane()).getByLabelText('Notes')).toHaveValue('typed meanwhile') // not reset by the answer
    expect(within(pane()).queryByRole('alert')).toBeNull() // and not mistaken for a conflict
    expect(await api.getCard(dishes.id)).toMatchObject({ execution: `${dishes.execution} mine`, notes: '' })
    await user.click(within(pane()).getByRole('button', { name: 'Save' }))
    await waitFor(async () => expect((await api.getCard(dishes.id)).notes).toBe('typed meanwhile'))
  })

  it('keeps an in-flight edit back to the previous base and saves it next', async () => {
    const user = userEvent.setup()
    const { api } = await renderApp('/deck', async (api) => void (await seedFamily(api)))
    await user.click(screen.getByRole('link', { name: /^Dishes/ }))
    const dishes = (await api.snapshot()).cards.find((c) => c.number === 3)!
    const execution = within(pane()).getByLabelText('Execution')
    const real = api.updateCard.bind(api)
    let release!: () => void
    const gate = new Promise<void>((resolve) => (release = resolve))
    vi.spyOn(api, 'updateCard').mockImplementationOnce(async (id, patch, etag) => {
      const saved = await real(id, patch, etag)
      await gate
      return saved
    })

    await user.clear(execution)
    await user.type(execution, 'B')
    await user.click(within(pane()).getByRole('button', { name: 'Save' }))
    await user.clear(execution)
    await user.type(execution, dishes.execution) // a real later edit, despite equalling the old base
    release()

    await waitFor(() => expect(execution).toHaveValue(dishes.execution))
    expect(within(pane()).getByRole('button', { name: 'Save' })).toBeInTheDocument()
    expect(await api.getCard(dishes.id)).toMatchObject({ execution: 'B' })
    await user.click(within(pane()).getByRole('button', { name: 'Save' }))
    await waitFor(async () => expect((await api.getCard(dishes.id)).execution).toBe(dishes.execution))
  })
})

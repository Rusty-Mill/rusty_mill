import { describe, expect, it, vi } from 'vitest'
import { MemoryAdapter } from '@/api/memory'
import { ConflictError, StaleError, UnauthorizedError } from '@/api/errors'
import type { ApiClient } from '@/api/client'
import type { Card } from '@/api/types'
import { createDataStore, STALE_MESSAGE } from './data'

type Store = ReturnType<typeof createDataStore>
const etagOf = (store: Store, id: string): string => store.getState().index.byId.get(id)!.etag

async function seeded() {
  const api = new MemoryAdapter()
  await api.seed()
  const ada = await api.createPerson('Ada')
  const store = createDataStore({ api })
  await store.getState().boot()
  return { api, ada, store }
}

describe('data store', () => {
  it('boots from the snapshot and indexes it', async () => {
    const { store } = await seeded()
    const s = store.getState()
    expect(s.status).toBe('ready')
    expect(s.cards).toHaveLength(100)
    expect(s.people.map((p) => p.name)).toEqual(['Ada'])
    expect(s.index.roots).toHaveLength(100)
  })

  it('merges the returned card after a write and re-indexes', async () => {
    const { store, ada } = await seeded()
    const id = store.getState().cards[1]!.id
    await store.getState().updateCard(id, { ownerId: ada.id }, etagOf(store, id))
    expect(store.getState().index.byId.get(id)?.ownerId).toBe(ada.id)
    const { children } = await store.getState().split(id, { children: [{ name: 'A' }, { name: 'B' }] })
    expect(store.getState().cards).toHaveLength(102)
    expect(store.getState().index.children.get(id)?.map((c) => c.name)).toEqual(['A', 'B'])
    await store.getState().reorderChildren(id, [children[1]!.id, children[0]!.id])
    expect(store.getState().index.children.get(id)?.map((c) => [c.name, c.position])).toEqual([
      ['B', 0],
      ['A', 1],
    ])
    expect(store.getState().index.leafIds.has(id)).toBe(false)
  })

  it('sends the version each write was based on', async () => {
    const { store, api, ada } = await seeded()
    const card = store.getState().cards[0]!
    const update = vi.spyOn(api, 'updateCard')
    const saved = await store.getState().updateCard(card.id, { ownerId: ada.id }, card.etag)
    expect(update).toHaveBeenLastCalledWith(card.id, { ownerId: ada.id }, card.etag)
    expect(saved.etag).not.toBe(card.etag)
    const split = vi.spyOn(api, 'split')
    const { children } = await store.getState().split(card.id, { children: [{ name: 'A' }, { name: 'B' }] })
    expect(split).toHaveBeenLastCalledWith(card.id, { children: [{ name: 'A' }, { name: 'B' }] }, saved.etag) // the merged card's tag
    const reorder = vi.spyOn(api, 'reorderChildren')
    const tree = store.getState().index.byId.get(card.id)!.treeEtag
    await store.getState().reorderChildren(card.id, [children[1]!.id, children[0]!.id])
    expect(reorder).toHaveBeenLastCalledWith(card.id, [children[1]!.id, children[0]!.id], tree) // the subtree tag, not the card's
    const unsplit = vi.spyOn(api, 'unsplitCard')
    const tree2 = store.getState().index.byId.get(card.id)!.treeEtag
    await store.getState().unsplit(card.id)
    expect(unsplit).toHaveBeenLastCalledWith(card.id, tree2)
  })

  it('on a 412 merges the current card, re-reads, toasts, and rethrows instead of overwriting', async () => {
    const { store, api } = await seeded()
    const card = store.getState().cards[0]!
    const elsewhere = await api.updateCard(card.id, { notes: 'theirs' }) // behind the store's back
    const snapshot = vi.spyOn(api, 'snapshot')
    await expect(store.getState().updateCard(card.id, { notes: 'mine' }, card.etag)).rejects.toBeInstanceOf(StaleError)
    expect(store.getState().index.byId.get(card.id)).toMatchObject({ notes: 'theirs', etag: elsewhere.etag })
    expect(store.getState().toasts).toEqual([{ id: expect.any(String), kind: 'info', message: STALE_MESSAGE }])
    await vi.waitFor(() => expect(snapshot).toHaveBeenCalledTimes(1))
    expect((await api.getCard(card.id)).notes).toBe('theirs')
    await store.getState().updateCard(card.id, { notes: 'mine' }, elsewhere.etag) // re-applied on the fresh version
    expect((await api.getCard(card.id)).notes).toBe('mine')
  })

  it('deletes a card, unsplits a subtree, and removes a person', async () => {
    const { store, api, ada } = await seeded()
    const [a, b] = store.getState().cards as [Card, Card]
    const { children } = await store.getState().split(a.id, { children: [{ name: 'X' }, { name: 'Y' }] })
    await store.getState().split(children[0]!.id, { children: [{ name: 'Z' }] })
    await expect(store.getState().deleteCard(a.id)).rejects.toBeInstanceOf(ConflictError)
    await store.getState().deleteCard(children[1]!.id)
    expect(store.getState().index.byId.has(children[1]!.id)).toBe(false)
    const { deleted } = await store.getState().unsplit(a.id)
    expect(deleted).toHaveLength(2)
    expect(store.getState().cards).toHaveLength(100)
    expect(store.getState().index.leafIds.has(a.id)).toBe(true)
    await store.getState().updateCard(b.id, { ownerId: ada.id }, etagOf(store, b.id))
    await expect(store.getState().deletePerson(ada.id)).rejects.toBeInstanceOf(ConflictError)
    expect(store.getState().toasts.at(-1)?.message).toMatch(/holds 1 card/)
    await store.getState().updateCard(b.id, { ownerId: null }, etagOf(store, b.id))
    await store.getState().deletePerson(ada.id)
    expect(store.getState().people).toEqual([])
    expect((await api.snapshot()).people).toEqual([])
  })

  /** `api.snapshot` that reads now but answers when `release()` is called. */
  function heldSnapshot(api: MemoryAdapter) {
    const real = api.snapshot.bind(api)
    let release!: () => void
    const gate = new Promise<void>((resolve) => (release = resolve))
    vi.spyOn(api, 'snapshot').mockImplementationOnce(async () => {
      const snap = await real()
      await gate
      return snap
    })
    return release
  }

  it('does not let a snapshot read before a write roll the write back', async () => {
    const { store, api } = await seeded()
    const card = store.getState().cards[0]!
    const release = heldSnapshot(api)
    const refreshing = store.getState().refresh() // reads the old notes, answers late
    await store.getState().updateCard(card.id, { notes: 'saved' }, card.etag)
    release()
    await refreshing
    expect(store.getState().index.byId.get(card.id)?.notes).toBe('saved')
  })

  it('keeps a card created while an older snapshot was in flight', async () => {
    const { store, api } = await seeded()
    const release = heldSnapshot(api)
    const refreshing = store.getState().refresh()
    const made = await store.getState().createCard({ name: 'Dog walking', suit: 'Home' })
    release()
    await refreshing
    expect(store.getState().index.byId.has(made.id)).toBe(true)
    expect(store.getState().cards).toHaveLength(101)
  })

  it('drops a snapshot that a write overtook, then takes the next one', async () => {
    const { store, api } = await seeded()
    const card = store.getState().cards[0]!
    const release = heldSnapshot(api)
    const refreshing = store.getState().refresh()
    await store.getState().updateCard(card.id, { notes: 'saved' }, card.etag)
    release()
    await refreshing
    await store.getState().refresh() // nothing in flight: adopted
    expect(store.getState().index.byId.get(card.id)?.notes).toBe('saved')
  })

  it('does not let an older read replace a newer one that already landed', async () => {
    const { store, api } = await seeded()
    const card = store.getState().cards[0]!
    const release = heldSnapshot(api) // the first refresh is slow …
    const slow = store.getState().refresh()
    await api.updateCard(card.id, { notes: 'elsewhere' }) // … the world moves on …
    await store.getState().refresh() // … a second read lands first
    expect(store.getState().index.byId.get(card.id)?.notes).toBe('elsewhere')
    release()
    await slow
    expect(store.getState().index.byId.get(card.id)?.notes).toBe('elsewhere')
  })

  it('leaves the cards alone when a reorder request fails', async () => {
    const { store, api } = await seeded()
    const id = store.getState().cards[1]!.id
    const { children } = await store.getState().split(id, { children: [{ name: 'A' }, { name: 'B' }] })
    const before = store.getState().index.children.get(id)!.map((c) => [c.name, c.position, c.etag])
    vi.spyOn(api, 'reorderChildren').mockRejectedValueOnce(new Error('connection lost'))
    await expect(store.getState().reorderChildren(id, [children[1]!.id, children[0]!.id])).rejects.toThrow('connection lost')
    expect(store.getState().index.children.get(id)!.map((c) => [c.name, c.position, c.etag])).toEqual(before) // not half-moved
    expect(store.getState().toasts.at(-1)?.kind).toBe('error')
  })

  it('puts cards in or out of the deck, skipping the ones already there', async () => {
    const { store, api, ada } = await seeded()
    const [a, b, c] = store.getState().cards as [Card, Card, Card]
    await store.getState().updateCard(a.id, { ownerId: ada.id }, a.etag)
    const update = vi.spyOn(api, 'updateCard')
    await store.getState().setInPlay([a.id, b.id], false)
    expect(update).toHaveBeenCalledTimes(2)
    expect(store.getState().index.byId.get(a.id)).toMatchObject({ inPlay: false, ownerId: null })
    expect(store.getState().index.byId.get(b.id)?.inPlay).toBe(false)
    expect(store.getState().index.byId.get(c.id)?.inPlay).toBe(true)
    await store.getState().setInPlay([a.id, b.id], false) // already out: nothing to write
    expect(update).toHaveBeenCalledTimes(2)
    await store.getState().setInPlay([a.id], true)
    expect(store.getState().index.byId.get(a.id)?.inPlay).toBe(true)
  })

  it('toasts a failed write and rethrows it', async () => {
    const { store } = await seeded()
    await expect(store.getState().createPerson('Ada')).rejects.toBeInstanceOf(ConflictError)
    expect(store.getState().toasts.map((t) => t.message)).toEqual(['Ada already exists'])
    store.getState().dismissToast(store.getState().toasts[0]!.id)
    expect(store.getState().toasts).toEqual([])
  })

  it('goes unauthorized on a 401, at boot and later', async () => {
    const api = new MemoryAdapter()
    const snapshot = vi.spyOn(api, 'snapshot').mockRejectedValueOnce(new UnauthorizedError())
    const store = createDataStore({ api })
    await store.getState().boot()
    expect(store.getState().status).toBe('unauthorized')
    snapshot.mockRestore()
    await store.getState().boot()
    expect(store.getState().status).toBe('ready')
    vi.spyOn(api, 'createPerson').mockRejectedValueOnce(new UnauthorizedError())
    await expect(store.getState().createPerson('x')).rejects.toBeInstanceOf(UnauthorizedError)
    expect(store.getState().status).toBe('unauthorized')
    expect(store.getState().toasts).toEqual([]) // the gate shows, not a toast
  })

  it('reports an unreachable server at boot', async () => {
    const api: ApiClient = { ...new MemoryAdapter(), snapshot: () => Promise.reject(new Error('boom')) } as ApiClient
    const store = createDataStore({ api })
    await store.getState().boot()
    expect(store.getState()).toMatchObject({ status: 'error', error: 'boom' })
  })

  it('refreshes on an interval and on focus', async () => {
    vi.useFakeTimers()
    try {
      const { store, api } = await seeded()
      const spy = vi.spyOn(api, 'snapshot')
      const stop = store.getState().startBackgroundSync(1000)
      vi.advanceTimersByTime(1000)
      expect(spy).toHaveBeenCalledTimes(1)
      window.dispatchEvent(new Event('focus'))
      expect(spy).toHaveBeenCalledTimes(2)
      stop()
      vi.advanceTimersByTime(5000)
      expect(spy).toHaveBeenCalledTimes(2)
    } finally {
      vi.useRealTimers()
    }
  })
})

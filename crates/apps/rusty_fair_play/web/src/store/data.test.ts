import { describe, expect, it, vi } from 'vitest'
import { MemoryAdapter } from '@/api/memory'
import { ConflictError, UnauthorizedError } from '@/api/errors'
import type { ApiClient } from '@/api/client'
import { createDataStore } from './data'

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
    expect(s.cards).toHaveLength(12)
    expect(s.people.map((p) => p.name)).toEqual(['Ada'])
    expect(s.index.roots).toHaveLength(12)
  })

  it('merges the returned card after a write and re-indexes', async () => {
    const { store, ada } = await seeded()
    const id = store.getState().cards[1]!.id
    await store.getState().updateCard(id, { ownerId: ada.id })
    expect(store.getState().index.byId.get(id)?.ownerId).toBe(ada.id)
    const { children } = await store.getState().split(id, { children: [{ name: 'A' }, { name: 'B' }] })
    expect(store.getState().cards).toHaveLength(14)
    expect(store.getState().index.children.get(id)?.map((c) => c.name)).toEqual(['A', 'B'])
    await store.getState().swapPositions(children[0]!, children[1]!)
    expect(store.getState().index.children.get(id)?.map((c) => c.name)).toEqual(['B', 'A'])
    expect(store.getState().index.leafIds.has(id)).toBe(false)
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

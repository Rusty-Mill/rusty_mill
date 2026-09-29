import { describe, expect, it, vi } from 'vitest'
import type { ApiClient } from '@/api/client'
import { InvalidError, NetworkError, UnauthorizedError } from '@/api/errors'
import { INBOX_ID, MemoryAdapter } from '@/api/memory'
import { CACHE_KEY, createDataStore, QUEUE_KEY, type DataStore } from './data'

/** A server (a MemoryAdapter) behind a switchboard: fail, hold or count calls. */
function controllable() {
  const server = new MemoryAdapter()
  const ctl = {
    server,
    fail: [] as Error[],
    hold: null as Promise<void> | null,
    calls: [] as string[],
    /** Fail the next `n` calls with a network error. */
    down(n = 1) {
      for (let i = 0; i < n; i++) ctl.fail.push(new NetworkError())
    },
  }
  const api = new Proxy(server, {
    get(target, prop) {
      const value = Reflect.get(target, prop) as unknown
      if (typeof value !== 'function') return value
      return async (...args: unknown[]) => {
        ctl.calls.push(String(prop))
        if (ctl.hold) await ctl.hold
        const error = ctl.fail.shift()
        if (error) throw error
        return (value as (...a: unknown[]) => unknown).apply(target, args)
      }
    },
  }) as unknown as ApiClient
  return { api, ctl }
}

/** Timers the test fires by hand, so backoff needs no real waiting. */
function manualTimers() {
  const pending: (() => void)[] = []
  return {
    schedule: (fn: () => void) => {
      pending.push(fn)
      return () => {
        const i = pending.indexOf(fn)
        if (i >= 0) pending.splice(i, 1)
      }
    },
    get count() {
      return pending.length
    },
    fire() {
      pending.splice(0).forEach((fn) => fn())
    },
  }
}

async function setup(opts: { storage?: Storage | null } = {}) {
  const { api, ctl } = controllable()
  const timers = manualTimers()
  const store = createDataStore({ api, storage: opts.storage ?? null, schedule: timers.schedule })
  await store.getState().boot()
  return { store, api, ctl, timers }
}

const drained = (store: DataStore) => vi.waitFor(() => expect(store.getState().pending).toBe(0))
const titles = (store: DataStore) => Object.values(store.getState().tasks).map((t) => t.title).sort()

describe('boot', () => {
  it('loads the snapshot', async () => {
    const { api } = controllable()
    const list = await api.createList({ name: 'Work' })
    await api.createTask({ listId: list.id, title: 'existing' })
    const store = createDataStore({ api })
    expect(store.getState().status).toBe('loading')
    await store.getState().boot()
    const s = store.getState()
    expect(s.status).toBe('ready')
    expect(s.inboxId).toBe(INBOX_ID)
    expect(Object.values(s.lists).map((l) => l.name).sort()).toEqual(['Inbox', 'Work'])
    expect(titles(store)).toEqual(['existing'])
  })

  it('asks for a token when the server refuses it', async () => {
    const { api, ctl } = controllable()
    ctl.fail.push(new UnauthorizedError())
    const store = createDataStore({ api })
    await store.getState().boot()
    expect(store.getState().status).toBe('unauthorized')
  })

  it('reports an unreachable server with nothing cached as an error', async () => {
    const { api, ctl } = controllable()
    ctl.down()
    const store = createDataStore({ api, storage: localStorage })
    await store.getState().boot()
    expect(store.getState().status).toBe('error')
  })

  it('starts from the cache when offline, and says so', async () => {
    const first = await setup({ storage: localStorage })
    await first.store.getState().createTask({ listId: INBOX_ID, title: 'cached' })
    await drained(first.store)
    expect(localStorage.getItem(CACHE_KEY)).toContain('cached')

    const { api, ctl } = controllable()
    ctl.down(2)
    const store = createDataStore({ api, storage: localStorage })
    await store.getState().boot()
    expect(store.getState()).toMatchObject({ status: 'ready', online: false })
    expect(titles(store)).toEqual(['cached'])
  })
})

describe('optimistic updates', () => {
  it('shows a new task before the server has answered', async () => {
    const { store, ctl } = await setup()
    let release!: () => void
    ctl.hold = new Promise((r) => (release = r))
    const created = await store.getState().createTask({ listId: INBOX_ID, title: 'instant' })
    expect(created.title).toBe('instant')
    expect(titles(store)).toEqual(['instant'])
    expect(store.getState().pending).toBe(1)
    expect((await ctl.server.snapshot()).tasks).toHaveLength(0) // the server has not seen it

    ctl.hold = null
    release()
    await drained(store)
    expect((await ctl.server.snapshot()).tasks.map((t) => t.title)).toEqual(['instant'])
  })

  it('keeps the client-chosen id, so the row does not change identity when confirmed', async () => {
    const { store, ctl } = await setup()
    const t = await store.getState().createTask({ listId: INBOX_ID, title: 'stable' })
    await drained(store)
    expect(Object.keys(store.getState().tasks)).toEqual([t.id])
    expect((await ctl.server.snapshot()).tasks[0]?.id).toBe(t.id)
  })

  it('applies a follow-up edit with the etag the server just issued (no false conflict)', async () => {
    const { store, ctl } = await setup()
    const t = await store.getState().createTask({ listId: INBOX_ID, title: 'a' })
    await store.getState().updateTask(t.id, { notes: 'first' })
    await drained(store)
    await store.getState().updateTask(t.id, { title: 'b' })
    await drained(store)
    expect(store.getState().toasts).toEqual([])
    const server = (await ctl.server.snapshot()).tasks[0]
    expect(server).toMatchObject({ title: 'b', notes: 'first' })
    expect(Number(server?.etag)).toBeGreaterThan(1)
  })

  it('refuses input the rules reject, changing nothing', async () => {
    const { store, ctl } = await setup()
    await expect(store.getState().createTask({ listId: INBOX_ID, title: '   ' })).rejects.toBeInstanceOf(InvalidError)
    expect(titles(store)).toEqual([])
    expect(store.getState().pending).toBe(0)
    expect(ctl.calls.filter((c) => c === 'createTask')).toHaveLength(0)
  })

  it('a rejected edit leaves no half-applied change behind', async () => {
    const { store } = await setup()
    const t = await store.getState().createTask({ listId: INBOX_ID, title: 'keep' })
    await drained(store)
    // The title is valid but the priority is not: nothing may stick.
    await expect(store.getState().updateTask(t.id, { title: 'changed', priority: 2 as never })).rejects.toBeInstanceOf(InvalidError)
    expect(store.getState().tasks[t.id]?.title).toBe('keep')
  })

  it('rolls back an edit the server refuses, and says why', async () => {
    const { store, ctl } = await setup()
    const t = await store.getState().createTask({ listId: INBOX_ID, title: 'ok' })
    await drained(store)
    ctl.fail.push(new InvalidError(422, 'nope'))
    await store.getState().updateTask(t.id, { title: 'rejected later' })
    expect(store.getState().tasks[t.id]?.title).toBe('rejected later') // shown at once
    await drained(store)
    expect(store.getState().tasks[t.id]?.title).toBe('ok') // and undone
    expect(store.getState().toasts.map((x) => x.message)).toEqual(['nope'])
  })

  it('coalesces rapid edits to one task into a single request', async () => {
    const { store, ctl } = await setup()
    const t = await store.getState().createTask({ listId: INBOX_ID, title: 'x' })
    await drained(store)
    ctl.hold = new Promise(() => undefined) // keep the queue from draining
    await store.getState().updateTask(t.id, { notes: 'a' }) // goes in flight
    await store.getState().updateTask(t.id, { notes: 'ab' })
    await store.getState().updateTask(t.id, { notes: 'abc', priority: 5 })
    expect(store.getState().pending).toBe(2) // one in flight, one coalesced behind it
    expect(store.getState().tasks[t.id]).toMatchObject({ notes: 'abc', priority: 5 })
  })
})

describe('server-side rules replayed locally', () => {
  it('trash cascades to subtasks and restore brings them back', async () => {
    const { store } = await setup()
    const p = await store.getState().createTask({ listId: INBOX_ID, title: 'p' })
    const c = await store.getState().createTask({ listId: INBOX_ID, title: 'c', parentId: p.id })
    await drained(store)
    await store.getState().trashTask(p.id)
    expect(store.getState().tasks[c.id]?.deletedMs).not.toBeNull()
    await store.getState().restoreTask(p.id)
    expect(store.getState().tasks[c.id]?.deletedMs).toBeNull()
    await drained(store)
    expect(store.getState().tasks[p.id]?.deletedMs).toBeNull()
  })

  it('deleting a list trashes its tasks; they restore to the Inbox', async () => {
    const { store } = await setup()
    const list = await store.getState().createList({ name: 'Doomed' })
    const t = await store.getState().createTask({ listId: list.id, title: 'orphan' })
    await store.getState().deleteList(list.id)
    expect(store.getState().lists[list.id]).toBeUndefined()
    expect(store.getState().tasks[t.id]?.deletedMs).not.toBeNull()
    await store.getState().restoreTask(t.id)
    expect(store.getState().tasks[t.id]?.listId).toBe(INBOX_ID)
    await drained(store)
  })

  it('a task with new tags creates the tag entities', async () => {
    const { store } = await setup()
    await store.getState().createTask({ listId: INBOX_ID, title: 't', tags: ['Errands'] })
    expect(store.getState().tags.errands?.label).toBe('Errands')
    await drained(store)
    expect(store.getState().tags.errands?.label).toBe('Errands') // and still there once confirmed
  })

  it('renaming a tag rewrites its tasks', async () => {
    const { store } = await setup()
    const t = await store.getState().createTask({ listId: INBOX_ID, title: 't', tags: ['old'] })
    await drained(store)
    await store.getState().renameTag('old', 'New')
    expect(store.getState().tasks[t.id]?.tags).toEqual(['new'])
    await drained(store)
    expect(store.getState().tasks[t.id]?.tags).toEqual(['new'])
    expect(store.getState().tags.old).toBeUndefined()
  })

  it('completing stamps the time, reopening clears it', async () => {
    const { store } = await setup()
    const t = await store.getState().createTask({ listId: INBOX_ID, title: 't' })
    await store.getState().toggleDone(t.id)
    expect(store.getState().tasks[t.id]).toMatchObject({ status: 'done', completedMs: expect.any(Number) })
    await store.getState().toggleDone(t.id)
    expect(store.getState().tasks[t.id]).toMatchObject({ status: 'open', completedMs: null })
    await drained(store)
  })

  it('completing a repeating task moves it to its next date and keeps it open', async () => {
    const { store } = await setup()
    const due = new Date(2026, 8, 29, 17).getTime()
    const t = await store.getState().createTask({ listId: INBOX_ID, title: 'daily', dueMs: due, repeatFlag: 'RRULE:FREQ=DAILY' })
    await store.getState().toggleDone(t.id)
    const after = store.getState().tasks[t.id]!
    expect(after.status).toBe('open')
    expect(after.dueMs).toBe(new Date(2026, 8, 30, 17).getTime())
    await drained(store)
  })
})

describe('offline', () => {
  it('keeps working when the server is unreachable, then catches up', async () => {
    const { store, ctl, timers } = await setup()
    ctl.down(2)
    await store.getState().createTask({ listId: INBOX_ID, title: 'made offline' })
    await vi.waitFor(() => expect(store.getState().online).toBe(false))
    expect(titles(store)).toEqual(['made offline']) // still visible
    expect(store.getState().pending).toBe(1)
    expect(timers.count).toBe(1) // a retry is scheduled

    timers.fire() // second failure
    await vi.waitFor(() => expect(timers.count).toBe(1))
    timers.fire() // the server is back
    await drained(store)
    expect(store.getState().online).toBe(true)
    expect((await ctl.server.snapshot()).tasks.map((t) => t.title)).toEqual(['made offline'])
  })

  it('retryNow does not wait for the backoff', async () => {
    const { store, ctl } = await setup()
    ctl.down()
    await store.getState().createTask({ listId: INBOX_ID, title: 'x' })
    await vi.waitFor(() => expect(store.getState().online).toBe(false))
    store.getState().retryNow()
    await drained(store)
    expect(store.getState().online).toBe(true)
  })

  it('sends queued edits in order', async () => {
    const { store, ctl } = await setup()
    ctl.down()
    const a = await store.getState().createTask({ listId: INBOX_ID, title: 'a' })
    await vi.waitFor(() => expect(store.getState().online).toBe(false))
    await store.getState().updateTask(a.id, { title: 'a2' })
    await store.getState().createTask({ listId: INBOX_ID, title: 'b' })
    store.getState().retryNow()
    await drained(store)
    const server = (await ctl.server.snapshot()).tasks
    expect(server.map((t) => t.title).sort()).toEqual(['a2', 'b'])
    expect(ctl.calls.filter((c) => c === 'createTask' || c === 'updateTask').slice(-3)).toEqual(['createTask', 'updateTask', 'createTask'])
  })

  it('survives a reload: the saved queue is replayed and sent', async () => {
    const one = await setup({ storage: localStorage })
    one.ctl.down(5)
    await one.store.getState().createTask({ listId: INBOX_ID, title: 'unsent' })
    await vi.waitFor(() => expect(one.store.getState().online).toBe(false))
    expect(localStorage.getItem(QUEUE_KEY)).toContain('unsent')

    // "Reload": a new store over the same storage, against a server that does not have it yet.
    const { api, ctl } = controllable()
    const store = createDataStore({ api, storage: localStorage })
    await store.getState().boot()
    expect(titles(store)).toEqual(['unsent']) // visible immediately
    await drained(store)
    expect((await ctl.server.snapshot()).tasks.map((t) => t.title)).toEqual(['unsent'])
    expect(localStorage.getItem(QUEUE_KEY)).toBeNull()
  })

  it('treats a create the server already has as done (the reply was lost)', async () => {
    const { store, ctl } = await setup()
    const id = 'aaaaaaaa-aaaa-7aaa-8aaa-aaaaaaaaaaaa'
    await ctl.server.createTask({ id, listId: INBOX_ID, title: 'already there' })
    await store.getState().createTask({ id, listId: INBOX_ID, title: 'already there' }).catch(() => undefined)
    await drained(store)
    expect(store.getState().toasts).toEqual([])
    expect((await ctl.server.snapshot()).tasks).toHaveLength(1)
  })
})

describe('conflicts', () => {
  it('retries once on top of what changed elsewhere, keeping both changes', async () => {
    const { store, ctl } = await setup()
    const t = await store.getState().createTask({ listId: INBOX_ID, title: 'shared' })
    await drained(store)
    await ctl.server.updateTask(t.id, { notes: 'edited elsewhere' }) // another device
    await store.getState().updateTask(t.id, { priority: 5 }) // we still hold the old etag
    await drained(store)
    expect(store.getState().toasts).toEqual([])
    expect((await ctl.server.snapshot()).tasks[0]).toMatchObject({ priority: 5, notes: 'edited elsewhere' })
    expect(store.getState().tasks[t.id]).toMatchObject({ priority: 5, notes: 'edited elsewhere' })
  })

  it('gives up with a message if it is stale twice', async () => {
    const { store, ctl } = await setup()
    const t = await store.getState().createTask({ listId: INBOX_ID, title: 'busy' })
    await drained(store)
    const real = ctl.server.updateTask.bind(ctl.server)
    let bumps = 0
    // Someone else edits between our attempt and our retry, every time.
    ctl.server.updateTask = async (id, patch, etag) => {
      if (etag !== undefined && bumps++ < 2) await real(id, { notes: `elsewhere ${bumps}` })
      return real(id, patch, etag)
    }
    await store.getState().updateTask(t.id, { priority: 5 })
    await drained(store)
    expect(store.getState().toasts).toHaveLength(1)
    expect(store.getState().toasts[0]?.message).toMatch(/changed somewhere else/)
  })
})

describe('refresh', () => {
  it('picks up changes made elsewhere', async () => {
    const { store, ctl } = await setup()
    await ctl.server.createTask({ listId: INBOX_ID, title: 'from another device' })
    await store.getState().refresh()
    expect(titles(store)).toEqual(['from another device'])
  })

  it('does not overwrite pending local edits', async () => {
    const { store, ctl } = await setup()
    ctl.hold = new Promise(() => undefined)
    await store.getState().createTask({ listId: INBOX_ID, title: 'local' })
    await store.getState().refresh() // queue is not empty: refresh stands aside
    expect(titles(store)).toEqual(['local'])
  })

  it('goes offline quietly when the refresh cannot reach the server', async () => {
    const { store, ctl } = await setup()
    ctl.down()
    await store.getState().refresh()
    expect(store.getState().online).toBe(false)
    expect(store.getState().status).toBe('ready')
  })
})

describe('toasts', () => {
  it('keeps the last few and can dismiss one', async () => {
    const { store } = await setup()
    for (let i = 0; i < 6; i++) store.getState().notify('info', `m${i}`)
    expect(store.getState().toasts.map((t) => t.message)).toEqual(['m2', 'm3', 'm4', 'm5'])
    store.getState().dismissToast(store.getState().toasts[0]!.id)
    expect(store.getState().toasts).toHaveLength(3)
  })
})

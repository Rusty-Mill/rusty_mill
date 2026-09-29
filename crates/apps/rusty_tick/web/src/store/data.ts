/**
 * The app's data: server truth plus a queue of pending operations.
 *
 *   visible state = server truth ("base") with the queued operations replayed over it
 *
 * so the UI updates instantly, a failed operation is rolled back by simply
 * dropping it from the queue, and a dropped connection just leaves the queue to
 * be sent later (it is saved, so it survives a reload). The replay uses
 * `MemoryAdapter`, i.e. the same rules the server applies.
 */
import { createStore, type StoreApi } from 'zustand/vanilla'
import type { ApiClient } from '@/api/client'
import { ConflictError, InvalidError, NetworkError, NotFoundError, StaleError, UnauthorizedError } from '@/api/errors'
import { MemoryAdapter } from '@/api/memory'
import type { List, ListPatch, NewList, NewTask, Snapshot, Tag, TagPatch, Task, TaskPatch } from '@/api/types'
import { newId } from '@/lib/id'
import { advanceDates } from '@/lib/recurrence'
import { isCreate, runOp, type EtagOf, type Op, type QueuedOp } from './ops'

export const QUEUE_KEY = 'tick-local:queue:v1'
export const CACHE_KEY = 'tick-local:cache:v1'
const MAX_ATTEMPTS = 5

export type Status = 'loading' | 'ready' | 'error' | 'unauthorized'

export interface Toast {
  id: string
  kind: 'error' | 'info'
  message: string
}

export interface DataState {
  status: Status
  error: string | null
  /** False while the server cannot be reached; edits still work and are queued. */
  online: boolean
  inboxId: string
  lists: Record<string, List>
  tasks: Record<string, Task>
  /** Keyed by tag `name`. */
  tags: Record<string, Tag>
  /** Operations not yet confirmed by the server. */
  pending: number
  toasts: Toast[]
}

export interface DataActions {
  boot(): Promise<void>
  refresh(): Promise<void>
  /** Send the queue now (after a reconnect or a new token). */
  retryNow(): void

  createTask(input: Omit<NewTask, 'id'> & { id?: string }): Promise<Task>
  updateTask(id: string, patch: TaskPatch): Promise<void>
  /** Complete or reopen; completing a repeating task moves it to its next date instead. */
  toggleDone(id: string): Promise<void>
  moveTask(id: string, listId: string): Promise<void>
  reorderTask(id: string, sortOrder: number): Promise<void>
  trashTask(id: string): Promise<void>
  restoreTask(id: string): Promise<void>
  purgeTask(id: string): Promise<void>
  emptyTrash(): Promise<void>

  createList(input: Omit<NewList, 'id'> & { id?: string }): Promise<List>
  updateList(id: string, patch: ListPatch): Promise<void>
  deleteList(id: string): Promise<void>

  createTag(label: string, color?: string | null): Promise<void>
  updateTag(name: string, patch: TagPatch): Promise<void>
  renameTag(name: string, label: string): Promise<void>
  deleteTag(name: string): Promise<void>

  notify(kind: Toast['kind'], message: string): void
  dismissToast(id: string): void
  /** Refresh on an interval and when the window regains focus or the network returns. */
  startBackgroundSync(intervalMs?: number): () => void
}

export type DataStore = StoreApi<DataState & DataActions>

export interface DataOptions {
  api: ApiClient
  storage?: Storage | null
  now?: () => number
  /** Run `fn` after `ms`; returns a cancel function. Injected so tests control time. */
  schedule?: (fn: () => void, ms: number) => () => void
}

const defaultSchedule = (fn: () => void, ms: number): (() => void) => {
  const t = setTimeout(fn, ms)
  return () => clearTimeout(t)
}

const byId = <T extends { id: string }>(xs: T[]): Record<string, T> => Object.fromEntries(xs.map((x) => [x.id, x]))
const byName = (xs: Tag[]): Record<string, Tag> => Object.fromEntries(xs.map((x) => [x.name, x]))

export function createDataStore(options: DataOptions): DataStore {
  const { api } = options
  const storage = options.storage ?? null
  const now = options.now ?? Date.now
  const schedule = options.schedule ?? defaultSchedule

  // Everything below is the store's private machinery; only `set`-published state is public.
  let base: Snapshot = { inboxId: '', serverTimeMs: 0, lists: [], tasks: [], tags: [] }
  let queue: QueuedOp[] = []
  let view = MemoryAdapter.fromSnapshot(base, { now })
  let running = false
  let epoch = 0 // bumped whenever the server confirms an operation
  let cancelRetry: (() => void) | null = null
  let retryDelay = 2000
  let toastSeq = 0

  const store = createStore<DataState & DataActions>()((set, get) => {
    // ---- persistence -------------------------------------------------

    const saveQueue = (): void => {
      try {
        if (queue.length) storage?.setItem(QUEUE_KEY, JSON.stringify(queue))
        else storage?.removeItem(QUEUE_KEY)
      } catch {
        /* storage full or unavailable: the queue still works in memory */
      }
    }
    const loadQueue = (): QueuedOp[] => {
      try {
        const raw = storage?.getItem(QUEUE_KEY)
        return raw ? (JSON.parse(raw) as QueuedOp[]) : []
      } catch {
        return []
      }
    }
    const saveCache = (): void => {
      try {
        storage?.setItem(CACHE_KEY, JSON.stringify(base))
      } catch {
        /* as above */
      }
    }
    const loadCache = (): Snapshot | null => {
      try {
        const raw = storage?.getItem(CACHE_KEY)
        return raw ? (JSON.parse(raw) as Snapshot) : null
      } catch {
        return null
      }
    }

    // ---- view = base + queue ----------------------------------------

    /** Publish the replay adapter's current state. */
    const publish = (): void => {
      const s = view.peek()
      set({ inboxId: base.inboxId, lists: byId(s.lists), tasks: byId(s.tasks), tags: byName(s.tags), pending: queue.length })
    }

    /** Rebuild the view from scratch: base with every queued op replayed over it. */
    const recompute = (): void => {
      view = MemoryAdapter.fromSnapshot(base, { now })
      for (const q of queue) void runOp(view, q.op).catch(() => undefined) // an op that cannot apply is dropped when it is sent
      publish()
    }

    const toast = (kind: Toast['kind'], message: string): void => {
      set((s) => ({ toasts: [...s.toasts, { id: `toast-${++toastSeq}`, kind, message }].slice(-4) }))
    }

    // ---- sending -----------------------------------------------------

    const etagOf: EtagOf = (op) => {
      switch (op.kind) {
        case 'updateTask':
        case 'trashTask':
          return base.tasks.find((t) => t.id === op.id)?.etag
        case 'updateList':
          return base.lists.find((l) => l.id === op.id)?.etag
        case 'updateTag':
          return base.tags.find((t) => t.name === op.name)?.etag
        default:
          return undefined
      }
    }

    /**
     * Record what the server confirmed: mirror the op onto base, then take the server's own
     * copy of the record. Synchronous on purpose: `MemoryAdapter` changes state at call time,
     * so the caller can dequeue the op in the same tick and no observer ever sees a state where
     * the op is neither queued nor in base.
     */
    const confirm = (op: Op, result: unknown): void => {
      const mirror = MemoryAdapter.fromSnapshot(base, { now })
      void runOp(mirror, op).catch(() => undefined)
      base = { ...mirror.peek(), serverTimeMs: base.serverTimeMs }
      const record = result as { id?: string; name?: string; etag?: string } | undefined
      if (record?.etag === undefined) return
      if (op.kind.endsWith('Task') && record.id) base.tasks = base.tasks.map((t) => (t.id === record.id ? (record as Task) : t))
      else if (op.kind.endsWith('List') && record.id) base.lists = base.lists.map((l) => (l.id === record.id ? (record as List) : l))
      else if (op.kind.endsWith('Tag') && record.name) base.tags = base.tags.map((g) => (g.name === record.name ? (record as Tag) : g))
    }

    /** Replace one record in base with the server's current copy (after a 412). */
    const adoptCurrent = (op: Op, current: unknown): void => {
      const rec = current as Task & List & Tag
      if (op.kind === 'updateTask' || op.kind === 'trashTask') base.tasks = base.tasks.map((t) => (t.id === rec.id ? (current as Task) : t))
      else if (op.kind === 'updateList') base.lists = base.lists.map((l) => (l.id === rec.id ? (current as List) : l))
      else if (op.kind === 'updateTag') base.tags = base.tags.map((g) => (g.name === rec.name ? (current as Tag) : g))
    }

    type Outcome = 'again' | 'next' | 'wait'

    const dropOp = (item: QueuedOp, message?: string): Outcome => {
      queue = queue.filter((q) => q !== item)
      saveQueue()
      if (message) toast('error', message)
      recompute() // the dropped op no longer applies: this is the rollback
      return 'next'
    }

    const handleFailure = (error: unknown, item: QueuedOp): Outcome => {
      if (error instanceof NetworkError) {
        set({ online: false })
        return 'wait'
      }
      if (error instanceof UnauthorizedError) {
        set({ status: 'unauthorized', error: error.message })
        return 'wait'
      }
      if (error instanceof StaleError) {
        if (item.stale >= 1) return dropOp(item, 'That changed somewhere else, so your edit was not saved.')
        item.stale += 1
        adoptCurrent(item.op, error.current) // retry the same patch on top of what is there now
        return 'again'
      }
      // A create the server already has: our earlier attempt landed and only the reply was lost.
      if (error instanceof ConflictError && isCreate(item.op)) return dropOp(item)
      if (error instanceof NotFoundError && (item.op.kind === 'trashTask' || item.op.kind === 'purgeTask' || item.op.kind === 'deleteList' || item.op.kind === 'deleteTag')) {
        return dropOp(item) // already gone: nothing to undo
      }
      if (error instanceof ConflictError || error instanceof NotFoundError || error instanceof InvalidError) {
        return dropOp(item, error.message)
      }
      // Anything else (a 5xx) may pass: try again a few times, then give up.
      item.attempts = (item.attempts ?? 0) + 1
      if (item.attempts >= MAX_ATTEMPTS) return dropOp(item, 'The server could not save that change.')
      return 'wait'
    }

    const scheduleRetry = (): void => {
      cancelRetry?.()
      cancelRetry = schedule(() => {
        cancelRetry = null
        void pump()
      }, retryDelay)
      retryDelay = Math.min(retryDelay * 2, 30_000)
    }

    /** Send queued operations one at a time, in order. */
    const pump = async (): Promise<void> => {
      if (running || get().status === 'unauthorized' || get().status === 'loading') return
      running = true
      try {
        while (queue.length) {
          const item = queue[0]!
          let outcome: Outcome
          try {
            const result = await runOp(api, item.op, etagOf)
            epoch += 1
            confirm(item.op, result) // base gains the op ...
            queue = queue.filter((q) => q !== item) // ... as it leaves the queue, with no await between
            saveQueue()
            recompute()
            retryDelay = 2000
            if (!get().online) set({ online: true })
            outcome = 'next'
          } catch (e) {
            outcome = handleFailure(e, item)
          }
          if (outcome === 'wait') {
            scheduleRetry()
            return
          }
        }
      } finally {
        running = false
      }
      await get().refresh() // reconcile with the server once the queue is empty
    }

    /** Applies one at a time, so each sees the previous one's effect in the view. */
    let enqueueChain: Promise<unknown> = Promise.resolve()
    const enqueue = (op: Op): Promise<void> => {
      const run = enqueueChain.then(() => enqueueNow(op))
      enqueueChain = run.catch(() => undefined) // a refused op must not block those behind it
      return run
    }

    /** Apply `op` optimistically, then queue it. Rejects (and changes nothing) if the rules refuse it. */
    const enqueueNow = async (op: Op): Promise<void> => {
      try {
        await runOp(view, op)
      } catch (e) {
        recompute() // a refused op may have half-applied; start clean
        throw e
      }
      const last = queue[queue.length - 1]
      const inFlight = running && queue.length === 1
      if (last && !inFlight && last.op.kind === 'updateTask' && op.kind === 'updateTask' && last.op.id === op.id) {
        last.op = { kind: 'updateTask', id: op.id, patch: { ...last.op.patch, ...op.patch } } // coalesce keystroke-rate edits
      } else {
        queue.push({ opId: newId(now()), op, stale: 0, attempts: 0 })
      }
      saveQueue()
      // Rebuild rather than publish the view we just changed: a refresh may have replaced
      // `view` while we awaited above, and only base + queue is guaranteed to include this op.
      recompute()
      void pump()
    }

    const task = (id: string): Task => {
      const t = get().tasks[id]
      if (!t) throw new NotFoundError('task not found')
      return t
    }

    return {
      status: 'loading',
      error: null,
      online: true,
      inboxId: '',
      lists: {},
      tasks: {},
      tags: {},
      pending: 0,
      toasts: [],

      async boot() {
        queue = loadQueue()
        set({ status: 'loading', error: null })
        try {
          base = await api.snapshot()
          epoch += 1
          set({ status: 'ready', online: true })
          recompute()
          saveCache()
          void pump()
        } catch (e) {
          if (e instanceof UnauthorizedError) {
            set({ status: 'unauthorized', error: e.message })
            return
          }
          const cached = e instanceof NetworkError ? loadCache() : null
          if (cached) {
            base = cached
            set({ status: 'ready', online: false })
            recompute()
            scheduleRetry()
            return
          }
          set({ status: 'error', error: e instanceof Error ? e.message : String(e) })
        }
      },

      async refresh() {
        if (queue.length || running || get().status === 'loading') return
        const startedAt = epoch
        try {
          const snapshot = await api.snapshot()
          if (epoch !== startedAt || queue.length) return // something changed while we asked: this copy may be stale
          base = snapshot
          if (!get().online) set({ online: true })
          recompute()
          saveCache()
        } catch (e) {
          if (e instanceof NetworkError) set({ online: false })
          else if (e instanceof UnauthorizedError) set({ status: 'unauthorized', error: e.message })
        }
      },

      retryNow() {
        cancelRetry?.()
        cancelRetry = null
        retryDelay = 2000
        if (get().status === 'unauthorized') set({ status: 'ready', error: null })
        if (get().status === 'error' || get().status === 'loading') void get().boot()
        else void pump()
      },

      async createTask(input) {
        const id = input.id ?? newId(now())
        await enqueue({ kind: 'createTask', input: { ...input, id } })
        return task(id)
      },
      updateTask: (id, patch) => enqueue({ kind: 'updateTask', id, patch }),
      async toggleDone(id) {
        const t = task(id)
        if (t.status === 'open' && t.repeatFlag) {
          const next = advanceDates(t, t.exDates)
          if (next) return enqueue({ kind: 'updateTask', id, patch: { dueMs: next.dueMs, startMs: next.startMs } })
        }
        return enqueue({ kind: 'updateTask', id, patch: { status: t.status === 'open' ? 'done' : 'open' } })
      },
      moveTask: (id, listId) => enqueue({ kind: 'updateTask', id, patch: { listId } }),
      reorderTask: (id, sortOrder) => enqueue({ kind: 'updateTask', id, patch: { sortOrder } }),
      trashTask: (id) => enqueue({ kind: 'trashTask', id }),
      restoreTask: (id) => enqueue({ kind: 'restoreTask', id }),
      purgeTask: (id) => enqueue({ kind: 'purgeTask', id }),
      emptyTrash: () => enqueue({ kind: 'emptyTrash' }),

      async createList(input) {
        const id = input.id ?? newId(now())
        await enqueue({ kind: 'createList', input: { ...input, id } })
        return get().lists[id]!
      },
      updateList: (id, patch) => enqueue({ kind: 'updateList', id, patch }),
      deleteList: (id) => enqueue({ kind: 'deleteList', id }),

      createTag: (label, color = null) => enqueue({ kind: 'createTag', label, color }),
      updateTag: (name, patch) => enqueue({ kind: 'updateTag', name, patch }),
      renameTag: (name, label) => enqueue({ kind: 'renameTag', name, label }),
      deleteTag: (name) => enqueue({ kind: 'deleteTag', name }),

      notify: toast,
      dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

      startBackgroundSync(intervalMs = 30_000) {
        const tick = (): void => void get().refresh()
        const onOnline = (): void => get().retryNow()
        const timer = setInterval(tick, intervalMs)
        const g = globalThis as { addEventListener?: typeof addEventListener; removeEventListener?: typeof removeEventListener }
        g.addEventListener?.('focus', tick)
        g.addEventListener?.('online', onOnline)
        return () => {
          clearInterval(timer)
          g.removeEventListener?.('focus', tick)
          g.removeEventListener?.('online', onOnline)
        }
      },
    }
  })
  return store
}

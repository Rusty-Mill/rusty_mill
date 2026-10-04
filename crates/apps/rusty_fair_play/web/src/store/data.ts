/**
 * The app's data: the server's snapshot (people, cards) and a memoised index
 * over it. Writes await the server and merge the returned records in; there
 * is no offline queue. The snapshot is refreshed on focus and on an interval.
 */
import { createStore, type StoreApi } from 'zustand/vanilla'
import type { ApiClient } from '@/api/client'
import { ApiError, NetworkError, UnauthorizedError } from '@/api/errors'
import type { Card, CardPatch, NewCard, Person, Snapshot, SplitInput, SplitResult } from '@/api/types'
import { indexCards, type CardIndex } from './derive'

export type Status = 'loading' | 'ready' | 'error' | 'unauthorized'

export interface Toast {
  id: string
  kind: 'error' | 'info'
  message: string
}

export interface DataState {
  status: Status
  error: string | null
  people: Person[]
  cards: Card[]
  index: CardIndex
  toasts: Toast[]
}

export interface DataActions {
  boot(): Promise<void>
  refresh(): Promise<void>
  seed(): Promise<void>

  createPerson(name: string): Promise<Person>
  renamePerson(id: string, name: string): Promise<void>

  updateCard(id: string, patch: CardPatch): Promise<Card>
  createCard(input: NewCard): Promise<Card>
  split(id: string, input: SplitInput): Promise<SplitResult>
  reset(id: string): Promise<Card>
  /** Swap two siblings' positions (two PUTs), then re-read both. */
  swapPositions(a: Card, b: Card): Promise<void>

  notify(kind: Toast['kind'], message: string): void
  dismissToast(id: string): void
  /** Refresh on an interval and when the window regains focus. */
  startBackgroundSync(intervalMs?: number): () => void
}

export type DataStore = StoreApi<DataState & DataActions>

export interface DataOptions {
  api: ApiClient
}

export function createDataStore({ api }: DataOptions): DataStore {
  let toastSeq = 0
  return createStore<DataState & DataActions>()((set, get) => {
    const adopt = (snap: Snapshot): void => set({ people: snap.people, cards: snap.cards, index: indexCards(snap.cards) })

    const mergeCards = (...cards: Card[]): void => {
      const next = [...get().cards]
      for (const card of cards) {
        const at = next.findIndex((c) => c.id === card.id)
        if (at >= 0) next[at] = card
        else next.push(card)
      }
      set({ cards: next, index: indexCards(next) })
    }

    const mergePerson = (person: Person): void => {
      const next = get().people.filter((p) => p.id !== person.id)
      next.push(person)
      next.sort((a, b) => a.player - b.player || a.name.localeCompare(b.name))
      set({ people: next })
    }

    /** Run a write; failures become a toast (and an unauthorized status) and are rethrown for the caller. */
    const write = async <T>(fn: () => Promise<T>): Promise<T> => {
      try {
        return await fn()
      } catch (e) {
        if (e instanceof UnauthorizedError) set({ status: 'unauthorized', error: e.message })
        else get().notify('error', describe(e))
        throw e
      }
    }

    return {
      status: 'loading',
      error: null,
      people: [],
      cards: [],
      index: indexCards([]),
      toasts: [],

      async boot() {
        set({ status: 'loading', error: null })
        try {
          adopt(await api.snapshot())
          set({ status: 'ready' })
        } catch (e) {
          set({ status: e instanceof UnauthorizedError ? 'unauthorized' : 'error', error: describe(e) })
        }
      },

      async refresh() {
        try {
          adopt(await api.snapshot())
          if (get().status !== 'ready') set({ status: 'ready', error: null })
        } catch (e) {
          if (e instanceof UnauthorizedError) set({ status: 'unauthorized', error: e.message })
          // Otherwise keep showing what we have; the next refresh may succeed.
        }
      },

      async seed() {
        await write(async () => {
          await api.seed()
          adopt(await api.snapshot())
        })
      },

      createPerson: (name) =>
        write(async () => {
          const person = await api.createPerson(name)
          mergePerson(person)
          return person
        }),
      renamePerson: (id, name) => write(async () => mergePerson(await api.renamePerson(id, name))),

      updateCard: (id, patch) =>
        write(async () => {
          const card = await api.updateCard(id, patch)
          mergeCards(card)
          return card
        }),
      createCard: (input) =>
        write(async () => {
          const card = await api.createCard(input)
          mergeCards(card)
          return card
        }),
      split: (id, input) =>
        write(async () => {
          const result = await api.split(id, input)
          mergeCards(result.parent, ...result.children)
          return result
        }),
      reset: (id) =>
        write(async () => {
          const card = await api.reset(id)
          mergeCards(card)
          return card
        }),
      swapPositions: (a, b) =>
        write(async () => {
          await api.setPosition(a.id, b.position)
          await api.setPosition(b.id, a.position)
          mergeCards({ ...a, position: b.position }, { ...b, position: a.position })
        }),

      notify(kind, message) {
        const id = `t${++toastSeq}`
        set((s) => ({ toasts: [...s.toasts, { id, kind, message }] }))
      },
      dismissToast(id) {
        set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }))
      },

      startBackgroundSync(intervalMs = 30_000) {
        const tick = (): void => void get().refresh()
        const timer = setInterval(tick, intervalMs)
        const onFocus = (): void => {
          if (document.visibilityState !== 'hidden') tick()
        }
        window.addEventListener('focus', onFocus)
        document.addEventListener('visibilitychange', onFocus)
        return () => {
          clearInterval(timer)
          window.removeEventListener('focus', onFocus)
          document.removeEventListener('visibilitychange', onFocus)
        }
      },
    }
  })
}

export function describe(e: unknown): string {
  if (e instanceof NetworkError) return 'The server could not be reached.'
  if (e instanceof ApiError) return e.message
  return e instanceof Error ? e.message : String(e)
}

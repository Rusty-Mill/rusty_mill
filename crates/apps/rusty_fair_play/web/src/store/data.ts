/**
 * The app's data: the server's snapshot (people, cards) and a memoised index
 * over it. Writes await the server and merge the returned records in; there
 * is no offline queue. The snapshot is refreshed on focus and on an interval.
 *
 * Every card write sends the version it was based on as `If-Match`: the draft's
 * own base for an edit (`updateCard`'s `base`), the card as shown for the rest
 * (its `etag`, or its `treeEtag` for unsplit and reorder). On 412 (`StaleError`)
 * the current card is merged in and a toast says so; the caller keeps its draft.
 *
 * A card's `treeEtag` moves when anything under it does, so a write to a card that
 * has an ancestor (or that reorders children) is followed by a re-read: the
 * ancestors' tags then match the server's, and the next unsplit or reorder is not
 * refused for a change this tab made itself.
 *
 * Reads and writes are ordered by a generation counter: a snapshot that was
 * requested before a write started, or that was still in flight when one
 * finished, is dropped, so an older read can never roll back an acknowledged
 * write (or make a freshly created card vanish).
 */

export const STALE_MESSAGE = 'This card changed elsewhere — reloaded, please re-apply your edit.'
import { createStore, type StoreApi } from 'zustand/vanilla'
import type { ApiClient } from '@/api/client'
import { ApiError, NetworkError, StaleError, UnauthorizedError } from '@/api/errors'
import type { Card, CardPatch, NewCard, Person, Snapshot, SplitInput, SplitResult, UnsplitResult } from '@/api/types'
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
  deletePerson(id: string): Promise<void>

  /** `base`: the card's `etag` as the edit was started from, not as the store has it now. */
  updateCard(id: string, patch: CardPatch, base: string): Promise<Card>
  createCard(input: NewCard): Promise<Card>
  split(id: string, input: SplitInput): Promise<SplitResult>
  reset(id: string): Promise<Card>
  deleteCard(id: string): Promise<void>
  /** Put cards in the family's deck or set them aside, one write each, stopping at the first that fails. */
  setInPlay(ids: string[], inPlay: boolean): Promise<void>
  unsplit(id: string): Promise<UnsplitResult>
  /** One request: `ids` is the new order of all of `parentId`'s children (validated whole, applied slot by slot). */
  reorderChildren(parentId: string, ids: string[]): Promise<void>

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

    const dropCards = (ids: string[]): void => {
      const gone = new Set(ids)
      const next = get().cards.filter((c) => !gone.has(c.id))
      set({ cards: next, index: indexCards(next) })
    }

    /** A card's version as this store shows it; absent for a card it does not know. */
    const etagOf = (id: string): string | undefined => get().index.byId.get(id)?.etag
    const treeEtagOf = (id: string): string | undefined => get().index.byId.get(id)?.treeEtag

    /** Bumped when a write starts and when it ends; see the header. */
    let generation = 0
    /** Reads are numbered too, so a slow older snapshot cannot replace a newer one. */
    let readSeq = 0
    let adoptedSeq = 0

    const mergePerson = (person: Person): void => {
      const next = get().people.filter((p) => p.id !== person.id)
      next.push(person)
      next.sort((a, b) => a.player - b.player || a.name.localeCompare(b.name))
      set({ people: next })
    }

    /** Run a write; failures become a toast (and an unauthorized status) and are rethrown for the caller. */
    const hasParent = (id: string): boolean => get().index.byId.get(id)?.parentCardId != null

    /** Run a write; `resync` re-reads the snapshot after it so ancestors' tree tags are current. */
    const write = async <T>(fn: () => Promise<T>, resync = false): Promise<T> => {
      generation++
      let stale = false
      try {
        const result = await fn()
        if (resync) await get().refresh()
        return result
      } catch (e) {
        if (e instanceof UnauthorizedError) set({ status: 'unauthorized', error: e.message })
        else if (e instanceof StaleError) {
          stale = true
          mergeCards(e.current)
          get().notify('info', STALE_MESSAGE)
        } else get().notify('error', describe(e))
        throw e
      } finally {
        generation++
        if (stale) void get().refresh() // after the bump, so its answer is kept
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
          const snap = await api.snapshot()
          adoptedSeq = ++readSeq // a refresh asked for earlier must not land over this
          adopt(snap)
          set({ status: 'ready' })
        } catch (e) {
          set({ status: e instanceof UnauthorizedError ? 'unauthorized' : 'error', error: describe(e) })
        }
      },

      async refresh() {
        const asked = generation
        const seq = ++readSeq
        try {
          const snap = await api.snapshot()
          if (generation !== asked) return // a write overtook this read; the next one will do
          if (seq < adoptedSeq) return // a newer read already landed
          adoptedSeq = seq
          adopt(snap)
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
      deletePerson: (id) =>
        write(async () => {
          await api.deletePerson(id)
          set({ people: get().people.filter((p) => p.id !== id) })
        }),

      updateCard: (id, patch, base) =>
        write(
          async () => {
            const card = await api.updateCard(id, patch, base)
            mergeCards(card)
            return card
          },
          hasParent(id) || patch.parentCardId != null,
        ),
      createCard: (input) =>
        write(
          async () => {
            const card = await api.createCard(input)
            mergeCards(card)
            return card
          },
          input.parentCardId != null,
        ),
      split: (id, input) =>
        write(async () => {
          const result = await api.split(id, input, etagOf(id))
          mergeCards(result.parent, ...result.children)
          return result
        }, hasParent(id)),
      reset: (id) =>
        write(async () => {
          const card = await api.reset(id, etagOf(id))
          mergeCards(card)
          return card
        }, hasParent(id)),
      async setInPlay(ids, inPlay) {
        for (const id of ids) {
          const card = get().index.byId.get(id)
          if (card && card.inPlay !== inPlay) await get().updateCard(id, { inPlay }, card.etag)
        }
      },
      deleteCard: (id) =>
        write(async () => {
          await api.deleteCard(id, etagOf(id))
          dropCards([id])
        }, hasParent(id)),
      unsplit: (id) =>
        write(async () => {
          const result = await api.unsplitCard(id, treeEtagOf(id))
          dropCards(result.deleted)
          mergeCards(result.parent)
          return result
        }, hasParent(id)),
      reorderChildren: (parentId, ids) =>
        write(async () => mergeCards(...(await api.reorderChildren(parentId, ids, treeEtagOf(parentId)))), true),

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

/**
 * Saved filters as `filter` client documents, held in a module-level store.
 * Edits apply at once; a failed save is reported and the change stays on screen.
 */
import { create } from 'zustand'
import type { ApiClient } from '@/api/client'
import { NotFoundError } from '@/api/errors'
import { newId } from '@/lib/id'
import { asFilterBody, type Filter, type FilterBody } from './logic'

type Notify = (kind: 'error' | 'info', message: string) => void

interface FiltersState {
  filters: Filter[]
  load(api: ApiClient, notify: Notify): Promise<void>
  add(input: FilterBody): string
  update(id: string, input: FilterBody): void
  remove(id: string): void
}

let api: ApiClient | null = null
let notify: Notify = () => undefined

export const useFilters = create<FiltersState>()((set, get) => ({
  filters: [],

  async load(client, notifyFn) {
    api = client
    notify = notifyFn
    try {
      const docs = await client.listDocs('filter')
      set({ filters: docs.flatMap((d) => { const body = asFilterBody(d.body); return body ? [{ id: d.id, ...body }] : [] }) })
    } catch {
      /* offline: keep what is shown */
    }
  },

  add(input) {
    const id = newId()
    set({ filters: [...get().filters, { id, ...input }] })
    api?.putDoc('filter', id, input).catch(() => notify('error', 'Could not save the filter.'))
    return id
  },

  update(id, input) {
    set({ filters: get().filters.map((f) => (f.id === id ? { id, ...input } : f)) })
    api?.putDoc('filter', id, input).catch(() => notify('error', 'Could not save the filter.'))
  },

  remove(id) {
    set({ filters: get().filters.filter((f) => f.id !== id) })
    api?.deleteDoc('filter', id).catch((e: unknown) => {
      if (!(e instanceof NotFoundError)) notify('error', 'Could not delete the filter.')
    })
  },
}))

/** For tests: forget everything held at module level. */
export function resetFiltersStore(): void {
  api = null
  notify = () => undefined
  useFilters.setState({ filters: [] })
}

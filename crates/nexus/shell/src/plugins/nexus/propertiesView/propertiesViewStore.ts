// RFC 0009 — row cache + filter state for the Properties table view.
// Pages append; the virtualized table reads `rows` and asks for
// `loadMore()` as the scroller nears the cached tail. The fetch itself
// goes through the kernel invoke handed in by the plugin's activate().

import { create } from 'zustand'
import { buildListArgs, decodePage, type PropertyViewRow } from './propertiesViewLogic'

const STORAGE_PLUGIN_ID = 'com.nexus.storage'

type Invoke = <T>(pluginId: string, command: string, args: unknown) => Promise<T>

let _invoke: Invoke | null = null

/** Installed once by the plugin's `activate`. */
export function setInvoke(invoke: Invoke): void {
  _invoke = invoke
}

export interface PropertiesViewState {
  columns: string[]
  rows: PropertyViewRow[]
  total: number
  loading: boolean
  error: string | null
  filterKey: string
  filterValue: string
  /** Monotonic tag so tests/components can observe a completed load. */
  version: number
  reload(): Promise<void>
  loadMore(): Promise<void>
  setFilter(key: string, value: string): Promise<void>
  clear(): void
}

async function fetchPage(offset: number, key: string, value: string) {
  if (!_invoke) throw new Error('[nexus.propertiesView] kernel invoke not installed')
  const raw = await _invoke<unknown>(STORAGE_PLUGIN_ID, 'properties_list', buildListArgs(key, value, offset))
  return decodePage(raw)
}

export const usePropertiesViewStore = create<PropertiesViewState>((set, get) => ({
  columns: [],
  rows: [],
  total: 0,
  loading: false,
  error: null,
  filterKey: '',
  filterValue: '',
  version: 0,

  clear: () =>
    set((s) => ({ columns: [], rows: [], total: 0, loading: false, error: null, version: s.version + 1 })),

  reload: async () => {
    const { loading, filterKey, filterValue } = get()
    if (loading) return
    set({ loading: true, error: null })
    try {
      const page = await fetchPage(0, filterKey, filterValue)
      set((s) => ({
        columns: page.columns,
        rows: page.rows,
        total: page.total,
        loading: false,
        version: s.version + 1,
      }))
    } catch (err) {
      set({ loading: false, error: err instanceof Error ? err.message : String(err) })
    }
  },

  loadMore: async () => {
    const { loading, rows, total, filterKey, filterValue } = get()
    if (loading || rows.length >= total) return
    set({ loading: true, error: null })
    try {
      const page = await fetchPage(rows.length, filterKey, filterValue)
      set((s) => ({
        columns: page.columns,
        rows: [...s.rows, ...page.rows],
        total: page.total,
        loading: false,
        version: s.version + 1,
      }))
    } catch (err) {
      set({ loading: false, error: err instanceof Error ? err.message : String(err) })
    }
  },

  setFilter: async (filterKey, filterValue) => {
    set({ filterKey, filterValue, rows: [], total: 0 })
    await get().reload()
  },
}))

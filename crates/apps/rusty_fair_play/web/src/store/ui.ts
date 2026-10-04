/**
 * View state that is not data: the board filters and what is collapsed.
 * Small, global, and partly remembered across reloads.
 */
import { create } from 'zustand'
import type { CardState, Suit } from '@/api/types'

const KEY = 'fair-play:ui:v1'

export type StateFilter = 'all' | CardState

export interface Filters {
  suits: Suit[]
  /** Person ids, plus `'unassigned'`. Empty = everyone. */
  owners: string[]
  state: StateFilter
  leavesOnly: boolean
  search: string
}

export const emptyFilters: Filters = { suits: [], owners: [], state: 'all', leavesOnly: false, search: '' }

interface Persisted {
  collapsedSuits: Record<string, boolean>
}

const load = (): Persisted => {
  try {
    const raw = localStorage.getItem(KEY)
    if (raw) return { collapsedSuits: {}, ...(JSON.parse(raw) as Partial<Persisted>) }
  } catch {
    /* unreadable: start fresh */
  }
  return { collapsedSuits: {} }
}

export interface UiState extends Persisted {
  filters: Filters
  /** Bumped to ask the search box to take focus (the `/` shortcut). */
  searchFocus: number
  setFilters(patch: Partial<Filters>): void
  toggleSuit(suit: Suit): void
  toggleOwner(id: string): void
  clearFilters(): void
  focusSearch(): void
  toggleSuitCollapsed(suit: Suit): void
}

export const useUi = create<UiState>()((set, get) => {
  const persist = (): void => {
    const { collapsedSuits } = get()
    try {
      localStorage.setItem(KEY, JSON.stringify({ collapsedSuits }))
    } catch {
      /* storage unavailable: the choice lasts until reload */
    }
  }
  const toggle = (xs: string[], x: string): string[] => (xs.includes(x) ? xs.filter((y) => y !== x) : [...xs, x])
  return {
    ...load(),
    filters: emptyFilters,
    searchFocus: 0,
    setFilters: (patch) => set((s) => ({ filters: { ...s.filters, ...patch } })),
    toggleSuit: (suit) => set((s) => ({ filters: { ...s.filters, suits: toggle(s.filters.suits, suit) as Suit[] } })),
    toggleOwner: (id) => set((s) => ({ filters: { ...s.filters, owners: toggle(s.filters.owners, id) } })),
    clearFilters: () => set({ filters: emptyFilters }),
    focusSearch: () => set((s) => ({ searchFocus: s.searchFocus + 1 })),
    toggleSuitCollapsed: (suit) => {
      set((s) => ({ collapsedSuits: { ...s.collapsedSuits, [suit]: !s.collapsedSuits[suit] } }))
      persist()
    },
  }
})

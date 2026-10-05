import type { Card } from '@/api/types'
import type { CardIndex } from '@/store/derive'
import type { Filters } from '@/store/ui'

export const UNASSIGNED = 'unassigned'

/**
 * The cards the board shows for `filters`. The family's deck by default; only the cards set aside
 * with `setAside`; every card while `choosing` (the mode for picking the deck).
 */
export function applyFilters(cards: Card[], index: CardIndex, filters: Filters, choosing = false): Card[] {
  const q = filters.search.trim().toLowerCase()
  return cards.filter((c) => {
    if (!choosing && c.inPlay === filters.setAside) return false
    if (filters.suits.length && !filters.suits.includes(c.suit)) return false
    if (filters.owners.length && !filters.owners.includes(c.ownerId ?? UNASSIGNED)) return false
    if (filters.state !== 'all' && c.state !== filters.state) return false
    if (filters.leavesOnly && !index.leafIds.has(c.id)) return false
    if (q && !c.name.toLowerCase().includes(q)) return false
    return true
  })
}

export const isFiltering = (f: Filters): boolean => f.suits.length > 0 || f.owners.length > 0 || f.state !== 'all' || f.leavesOnly || f.setAside || f.search.trim() !== ''

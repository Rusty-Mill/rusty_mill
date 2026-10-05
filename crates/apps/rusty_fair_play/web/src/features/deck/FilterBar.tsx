import { Plus, Search, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { SUITS } from '@/api/types'
import { useData } from '@/app/services'
import { initial, suitBg } from '@/components/badges'
import { useUi, type StateFilter } from '@/store/ui'
import { isFiltering, UNASSIGNED } from './filters'
import { NewCardDialog } from './NewCardDialog'

const STATES: { value: StateFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'original', label: 'Original' },
  { value: 'edited', label: 'Edited' },
  { value: 'custom', label: 'Custom' },
]

/** Suit, owner and state chips, the leaves-only toggle, and the search box. */
export function FilterBar() {
  const people = useData((s) => s.people)
  const asideCount = useData((s) => s.cards.filter((c) => !c.inPlay).length)
  const filters = useUi((s) => s.filters)
  const { toggleSuit, toggleOwner, setFilters, clearFilters } = useUi.getState()
  const searchFocus = useUi((s) => s.searchFocus)
  const searchRef = useRef<HTMLInputElement>(null)
  const [newOpen, setNewOpen] = useState(false)
  useEffect(() => {
    if (searchFocus > 0) searchRef.current?.focus()
  }, [searchFocus])

  return (
    <div className="flex flex-col gap-2 border-b border-line px-5 py-3" role="search" aria-label="Filter the deck">
      <div className="flex flex-wrap items-center gap-2">
        <label className="relative flex-1 basis-[200px]">
          <Search size={15} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-grey" aria-hidden />
          <input
            ref={searchRef}
            type="search"
            aria-label="Search cards"
            placeholder="Search cards  ( / )"
            value={filters.search}
            onChange={(e) => setFilters({ search: e.target.value })}
            className="field h-8 pl-8"
          />
        </label>
        <div className="flex items-center gap-1" role="group" aria-label="State">
          {STATES.map((s) => (
            <button key={s.value} type="button" aria-pressed={filters.state === s.value} onClick={() => setFilters({ state: s.value })} className={`chip ${filters.state === s.value ? 'chip-on' : ''}`}>
              {s.label}
            </button>
          ))}
        </div>
        <label className="chip cursor-pointer">
          <input type="checkbox" checked={filters.leavesOnly} onChange={(e) => setFilters({ leavesOnly: e.target.checked })} className="accent-primary" />
          Leaves only
        </label>
        {asideCount > 0 && (
          <button type="button" aria-pressed={filters.setAside} onClick={() => setFilters({ setAside: !filters.setAside })} className={`chip ${filters.setAside ? 'chip-on' : ''}`}>
            Set aside · {asideCount}
          </button>
        )}
        {isFiltering(filters) && (
          <button type="button" onClick={clearFilters} className="chip text-grey">
            <X size={13} aria-hidden /> Clear
          </button>
        )}
        <button type="button" onClick={() => setNewOpen(true)} className="btn-primary ml-auto h-7 px-2.5 text-s">
          <Plus size={14} aria-hidden /> New card…
        </button>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex flex-wrap items-center gap-1" role="group" aria-label="Suit">
          {SUITS.map((suit) => {
            const on = filters.suits.includes(suit)
            return (
              <button key={suit} type="button" aria-pressed={on} onClick={() => toggleSuit(suit)} className={`chip ${on ? 'chip-on' : ''}`}>
                <span aria-hidden className={`h-2.5 w-2.5 rounded-full ${suitBg[suit]}`} />
                {suit}
              </button>
            )
          })}
        </div>
        <span aria-hidden className="h-4 w-px bg-line" />
        <div className="flex flex-wrap items-center gap-1" role="group" aria-label="Owner">
          {people.map((p) => {
            const on = filters.owners.includes(p.id)
            return (
              <button key={p.id} type="button" aria-pressed={on} onClick={() => toggleOwner(p.id)} className={`chip ${on ? 'chip-on' : ''}`}>
                <span aria-hidden className="flex h-4 w-4 items-center justify-center rounded-full bg-primary text-[10px] font-semibold text-white">
                  {initial(p.name)}
                </span>
                {p.name}
              </button>
            )
          })}
          <button type="button" aria-pressed={filters.owners.includes(UNASSIGNED)} onClick={() => toggleOwner(UNASSIGNED)} className={`chip ${filters.owners.includes(UNASSIGNED) ? 'chip-on' : 'text-grey'}`}>
            Unassigned
          </button>
        </div>
      </div>
      {newOpen && <NewCardDialog open onClose={() => setNewOpen(false)} />}
    </div>
  )
}

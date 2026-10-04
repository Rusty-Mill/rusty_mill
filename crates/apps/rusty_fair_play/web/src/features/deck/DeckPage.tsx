import { ChevronDown, ChevronRight } from 'lucide-react'
import { useEffect, useMemo } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { SUITS } from '@/api/types'
import { PATHS } from '@/app/paths'
import { useActions, useData } from '@/app/services'
import { suitText } from '@/components/badges'
import { bySuit } from '@/store/derive'
import { useUi } from '@/store/ui'
import { CardTile } from './CardTile'
import { DetailPane } from './DetailPane'
import { FilterBar } from './FilterBar'
import { applyFilters, isFiltering } from './filters'

/** The board: six suit shelves, with the detail pane for the card in the URL. */
export function DeckPage() {
  const { cardId = null } = useParams()
  const navigate = useNavigate()
  const cards = useData((s) => s.cards)
  const people = useData((s) => s.people)
  const index = useData((s) => s.index)
  const filters = useUi((s) => s.filters)
  const collapsed = useUi((s) => s.collapsedSuits)
  const toggleSuitCollapsed = useUi((s) => s.toggleSuitCollapsed)
  const shelves = useMemo(() => bySuit(applyFilters(cards, index, filters)), [cards, index, filters])
  const totals = useMemo(() => bySuit(cards), [cards])

  // Escape closes the pane (dialogs stop the event before it gets here).
  useEffect(() => {
    if (!cardId) return
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape' && !document.querySelector('[role="dialog"]')) navigate(PATHS.deck)
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [cardId, navigate])

  if (cards.length === 0) return <EmptyDeck />

  const shown = SUITS.reduce((n, s) => n + shelves[s].length, 0)
  return (
    <>
      <main className="flex min-w-0 flex-1 flex-col" aria-label="Deck">
        <header className="flex items-center justify-between px-5 pt-4">
          <h1 className="text-h1 font-semibold">Deck</h1>
          <span className="text-s text-grey">
            {isFiltering(filters) ? `${shown} of ${cards.length} cards` : `${cards.length} cards`}
          </span>
        </header>
        <FilterBar />
        <div className="scroll-thin flex-1 overflow-y-auto px-5 pb-8">
          {shown === 0 && <p className="py-10 text-center text-grey">No cards match these filters.</p>}
          {SUITS.map((suit) => {
            const list = shelves[suit]
            if (list.length === 0) return null
            const open = !collapsed[suit]
            return (
              <section key={suit} aria-labelledby={`shelf-${suit}`} className="pt-5">
                <h2 id={`shelf-${suit}`} className="mb-2 flex items-center gap-1.5 text-base font-semibold">
                  <button type="button" aria-expanded={open} aria-label={`${open ? 'Collapse' : 'Expand'} ${suit}`} onClick={() => toggleSuitCollapsed(suit)} className="rounded p-0.5 text-grey hover:bg-hover">
                    {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
                  </button>
                  <span className={suitText[suit]}>{suit}</span>
                  <span className="font-normal text-grey">
                    · {isFiltering(filters) && list.length !== totals[suit].length ? `${list.length} of ${totals[suit].length}` : list.length}
                  </span>
                </h2>
                {open && (
                  <ul className="grid list-none grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-3 p-0" aria-label={`${suit} cards`}>
                    {list.map((card) => (
                      <li key={card.id} className="flex">
                        <CardTile card={card} people={people} childCount={index.children.get(card.id)?.length ?? 0} selected={card.id === cardId} />
                      </li>
                    ))}
                  </ul>
                )}
              </section>
            )
          })}
        </div>
      </main>
      <DetailPane cardId={cardId} />
    </>
  )
}

/** A data directory without a deck (the binary started with `--no-seed`): offer to load it. */
function EmptyDeck() {
  const { seed } = useActions()
  return (
    <main className="flex flex-1 flex-col items-center justify-center gap-3 text-center" aria-label="Deck">
      <h1 className="text-h1 font-semibold">No cards yet</h1>
      <p className="max-w-sm text-grey">The 100-card Fair Play deck ships with the app. Load it to start dealing.</p>
      <button type="button" className="btn-primary h-10 px-5" onClick={() => void seed().catch(() => undefined)}>
        Load the Fair Play deck
      </button>
    </main>
  )
}

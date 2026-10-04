import { ChevronDown, ChevronRight } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { SUITS, type Card } from '@/api/types'
import { PATHS } from '@/app/paths'
import { useActions, useData } from '@/app/services'
import { suitText } from '@/components/badges'
import { Confirm } from '@/components/Confirm'
import { bySuit, childrenOf } from '@/store/derive'
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
  const choosing = useUi((s) => s.choosing)
  const collapsed = useUi((s) => s.collapsedSuits)
  const toggleSuitCollapsed = useUi((s) => s.toggleSuitCollapsed)
  const { setInPlay, notify } = useActions()
  const shelves = useMemo(() => bySuit(applyFilters(cards, index, filters, choosing)), [cards, index, filters, choosing])
  // What the board is a view of: the family's deck, the cards set aside, or (choosing) everything.
  const base = useMemo(() => (choosing ? cards : cards.filter((c) => c.inPlay === !filters.setAside)), [cards, choosing, filters.setAside])
  const totals = useMemo(() => bySuit(base), [base])
  const inDeck = useMemo(() => cards.filter((c) => c.inPlay).length, [cards])
  const [asking, setAsking] = useState<Card[] | null>(null)

  // Nothing left set aside: leave the "set aside" view rather than show an empty board with no chip to undo it.
  const anyAside = cards.some((c) => !c.inPlay)
  useEffect(() => {
    if (!anyAside && filters.setAside) useUi.getState().setFilters({ setAside: false })
  }, [anyAside, filters.setAside])

  // Opening another card brings the folded detail pane back. Not on first render: a reload keeps it folded.
  const shownCard = useRef(cardId)
  useEffect(() => {
    if (cardId && cardId !== shownCard.current) useUi.getState().setPaneCollapsed(false)
    shownCard.current = cardId
  }, [cardId])

  /** Put cards in or out of the deck. Setting aside a dealt card takes it from its owner, so that asks first. */
  const choose = (list: Card[], inPlay: boolean): void => {
    const todo = list.filter((c) => c.inPlay !== inPlay)
    const split = inPlay ? [] : todo.filter((c) => childrenOf(index, c.id).length > 0)
    const ok = todo.filter((c) => !split.includes(c))
    if (split.length > 0) notify('info', `${split.length} split ${split.length === 1 ? 'card was' : 'cards were'} left in the deck: unsplit ${split.length === 1 ? 'it' : 'them'} first.`)
    if (ok.length === 0) return
    if (!inPlay && ok.some((c) => c.ownerId)) return setAsking(ok)
    void setInPlay(ok.map((c) => c.id), inPlay).catch(() => undefined)
  }

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
  const searching = isFiltering({ ...filters, setAside: false })
  const dealt = asking?.filter((c) => c.ownerId).length ?? 0
  return (
    <>
      <main className="flex min-w-0 flex-1 flex-col" aria-label="Deck">
        <header className="flex items-center justify-between px-5 pt-4">
          <h1 className="text-h1 font-semibold">Deck</h1>
          <div className="flex items-center gap-3 text-s text-grey">
            <span>{searching || filters.setAside ? `${shown} of ${base.length} cards` : `${inDeck} cards`}</span>
            {cards.length > inDeck && !filters.setAside && <span>· {cards.length - inDeck} set aside</span>}
            <button type="button" aria-pressed={choosing} onClick={() => useUi.getState().setChoosing(!choosing)} className={`btn h-7 px-2.5 text-s ${choosing ? 'border-primary text-primary' : ''}`}>
              {choosing ? 'Done choosing' : 'Choose cards'}
            </button>
          </div>
        </header>
        {choosing && <p className="px-5 pt-2 text-s text-grey">Pick the cards your family plays with. Click a card to put it in or take it out; a card set aside has no owner and stays out of the undealt list.</p>}
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
                    · {searching && list.length !== totals[suit].length ? `${list.length} of ${totals[suit].length}` : list.length}
                  </span>
                  {choosing && (
                    <span className="ml-2 flex gap-1 text-s font-normal">
                      <button type="button" aria-label={`Put every ${suit} card in the deck`} onClick={() => choose(list, true)} className="chip">
                        All in
                      </button>
                      <button type="button" aria-label={`Set aside every ${suit} card`} onClick={() => choose(list, false)} className="chip">
                        All out
                      </button>
                    </span>
                  )}
                </h2>
                {open && (
                  <ul className="grid list-none grid-cols-[repeat(auto-fill,minmax(208px,1fr))] gap-3 p-0" aria-label={`${suit} cards`}>
                    {list.map((card) => (
                      <li key={card.id} className="h-[92px]">
                        <CardTile card={card} people={people} childCount={index.children.get(card.id)?.length ?? 0} selected={card.id === cardId} choosing={choosing} onToggle={(c) => choose([c], !c.inPlay)} />
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
      <Confirm
        open={asking !== null}
        title={`Set aside ${asking?.length ?? 0} ${asking?.length === 1 ? 'card' : 'cards'}?`}
        message={`${dealt} of ${asking?.length === 1 ? 'it is' : 'them are'} dealt. Setting a card aside takes it back from its owner and leaves it out of the undealt list and the balance. You can add it back later.`}
        confirmLabel="Set aside"
        danger
        onConfirm={() => {
          const ids = (asking ?? []).map((c) => c.id)
          setAsking(null)
          void setInPlay(ids, false).catch(() => undefined)
        }}
        onCancel={() => setAsking(null)}
      />
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

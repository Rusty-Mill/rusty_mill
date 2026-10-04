import { ArrowDown, ArrowUp, ChevronRight, Scissors, X } from 'lucide-react'
import { useMemo, useRef, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import type { Card, CardPatch } from '@/api/types'
import { PATHS } from '@/app/paths'
import { useActions, useData } from '@/app/services'
import { OwnerChip, ownerOf, StateBadge, suitBg, suitText } from '@/components/badges'
import { chainToRoot, childrenOf } from '@/store/derive'
import { BaselineBlock } from './BaselineBlock'
import { SplitDialog } from './SplitDialog'
import { StandardsEditor } from './StandardsEditor'

const shell = 'flex w-[460px] shrink-0 flex-col border-l border-line bg-surface max-[1279px]:w-[400px] max-[999px]:absolute max-[999px]:inset-0 max-[999px]:z-20 max-[999px]:w-full max-[999px]:border-l-0'

/** The right-side pane for the card in the URL (a drawer over the board on narrow screens). */
export function DetailPane({ cardId }: { cardId: string | null }) {
  const card = useData((s) => (cardId ? s.index.byId.get(cardId) : undefined))
  if (!cardId) return <aside aria-label="Card details" className={`${shell} items-center justify-center text-grey max-[999px]:hidden`}>Pick a card</aside>
  if (!card) {
    return (
      <aside aria-label="Card details" className={`${shell} items-center justify-center gap-2 text-grey`}>
        <p>This card does not exist.</p>
        <Link to={PATHS.deck} className="text-primary underline">
          Back to the deck
        </Link>
      </aside>
    )
  }
  // Keyed by id so every draft starts from the card being opened.
  return <CardDetail key={card.id} card={card} />
}

interface Draft {
  conception: string
  planning: string
  execution: string
  minimumStandardOfCare: string[]
  notes: string
}

const draftOf = (c: Card): Draft => ({ conception: c.conception, planning: c.planning, execution: c.execution, minimumStandardOfCare: [...c.minimumStandardOfCare], notes: c.notes })

function CardDetail({ card }: { card: Card }) {
  const navigate = useNavigate()
  const people = useData((s) => s.people)
  const index = useData((s) => s.index)
  const { updateCard, swapPositions } = useActions()
  const [draft, setDraft] = useState<Draft>(() => draftOf(card))
  // When the card changes under the draft (a reset, a background refresh), fields the user has not touched follow it.
  const [base, setBase] = useState(card)
  if (base !== card) {
    const was = draftOf(base)
    const now = draftOf(card)
    setDraft({
      conception: draft.conception === was.conception ? now.conception : draft.conception,
      planning: draft.planning === was.planning ? now.planning : draft.planning,
      execution: draft.execution === was.execution ? now.execution : draft.execution,
      minimumStandardOfCare: draft.minimumStandardOfCare.join('\u0000') === was.minimumStandardOfCare.join('\u0000') ? now.minimumStandardOfCare : draft.minimumStandardOfCare,
      notes: draft.notes === was.notes ? now.notes : draft.notes,
    })
    setBase(card)
  }
  const [saving, setSaving] = useState(false)
  const [splitOpen, setSplitOpen] = useState(false)

  const chain = useMemo(() => chainToRoot(index, card.id), [index, card.id])
  const children = childrenOf(index, card.id)
  const owner = ownerOf(card, people)

  /** Only the fields that differ from the card go in the PATCH. */
  const changes = useMemo((): CardPatch => {
    const p: CardPatch = {}
    if (draft.conception !== card.conception) p.conception = draft.conception
    if (draft.planning !== card.planning) p.planning = draft.planning
    if (draft.execution !== card.execution) p.execution = draft.execution
    if (draft.minimumStandardOfCare.join('\u0000') !== card.minimumStandardOfCare.join('\u0000')) p.minimumStandardOfCare = draft.minimumStandardOfCare
    if (draft.notes !== card.notes) p.notes = draft.notes
    return p
  }, [draft, card])
  const dirty = Object.keys(changes).length > 0

  const save = async (): Promise<void> => {
    setSaving(true)
    try {
      const saved = await updateCard(card.id, changes)
      setDraft(draftOf(saved))
    } catch {
      /* toasted by the store; the draft stays for another try */
    } finally {
      setSaving(false)
    }
  }

  const deal = (value: string): void => {
    void updateCard(card.id, { ownerId: value || null }).catch(() => undefined)
  }

  const move = (i: number, delta: number): void => {
    const a = children[i]
    const b = children[i + delta]
    if (a && b) void swapPositions(a, b).catch(() => undefined)
  }

  return (
    <aside aria-label="Card details" className={shell}>
      <div className={`h-1.5 shrink-0 ${suitBg[card.suit]}`} aria-hidden />
      <div className="flex items-center gap-1 px-4 pt-3 text-s text-grey">
        <nav aria-label="Breadcrumb" className="flex min-w-0 flex-1 flex-wrap items-center gap-1">
          <Link to={PATHS.deck} className="hover:text-text">
            Deck
          </Link>
          {chain.map((c) => (
            <span key={c.id} className="flex items-center gap-1">
              <ChevronRight size={13} aria-hidden />
              {c.id === card.id ? <span className="text-text">{c.name}</span> : <Link to={PATHS.card(c.id)} className="hover:text-text">{c.name}</Link>}
            </span>
          ))}
        </nav>
        <button type="button" aria-label="Close" onClick={() => navigate(PATHS.deck)} className="rounded-row p-1.5 text-grey hover:bg-hover">
          <X size={18} />
        </button>
      </div>

      <div className="scroll-thin flex flex-1 flex-col gap-5 overflow-y-auto px-4 pb-6 pt-2">
        <div className="flex flex-col gap-1.5">
          <NameField card={card} />
          <div className="flex flex-wrap items-center gap-2 text-s">
            <span className={`font-medium ${suitText[card.suit]}`}>{card.suit}</span>
            {card.number !== null && <span className="text-grey">#{card.number}</span>}
            <StateBadge state={card.state} showOriginal />
            {card.origin === 'family' && <span className="text-grey">Custom card</span>}
          </div>
        </div>

        <label className="flex items-center gap-2">
          <span className="text-s text-grey">Deal to</span>
          <select aria-label="Deal to" value={card.ownerId ?? ''} onChange={(e) => deal(e.target.value)} className="field h-8 w-auto">
            <option value="">Unassigned</option>
            {people.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
          <OwnerChip owner={owner} />
        </label>

        {(['conception', 'planning', 'execution'] as const).map((field) => (
          <label key={field} className="flex flex-col gap-1">
            <span className="text-s font-semibold capitalize">{field}</span>
            <textarea aria-label={field[0]!.toUpperCase() + field.slice(1)} value={draft[field]} onChange={(e) => setDraft({ ...draft, [field]: e.target.value })} rows={3} className="field resize-y" />
          </label>
        ))}

        <section className="flex flex-col gap-1" aria-labelledby="msc-h">
          <h3 id="msc-h" className="m-0 text-s font-semibold">
            Minimum standard of care
          </h3>
          <StandardsEditor value={draft.minimumStandardOfCare} onChange={(minimumStandardOfCare) => setDraft({ ...draft, minimumStandardOfCare })} />
        </section>

        <label className="flex flex-col gap-1">
          <span className="text-s font-semibold">Notes</span>
          <textarea aria-label="Notes" value={draft.notes} onChange={(e) => setDraft({ ...draft, notes: e.target.value })} rows={2} placeholder="Notes never make a card edited." className="field resize-y" />
        </label>

        {dirty && (
          <div className="sticky bottom-0 flex items-center justify-end gap-2 border-t border-line bg-surface py-2">
            <button type="button" onClick={() => setDraft(draftOf(card))} className="btn border-transparent">
              Cancel
            </button>
            <button type="button" onClick={() => void save()} disabled={saving} className="btn-primary">
              Save
            </button>
          </div>
        )}

        {card.origin === 'deck' ? (
          <BaselineBlock card={card} />
        ) : (
          <p className="rounded-card border border-line px-3 py-2 text-s text-grey">Custom card: made by this family, with no deck original to compare against.</p>
        )}

        <section aria-labelledby="children-h" className="flex flex-col gap-2">
          <div className="flex items-center justify-between">
            <h3 id="children-h" className="m-0 text-base font-semibold">
              Children {children.length > 0 && <span className="font-normal text-grey">· {children.length}</span>}
            </h3>
            <button type="button" onClick={() => setSplitOpen(true)} className="btn">
              <Scissors size={14} /> Split…
            </button>
          </div>
          {children.length === 0 && <p className="text-s text-grey">Not split. Split it to deal parts of this card to different people.</p>}
          {children.length > 0 && (
            <ol className="flex list-none flex-col gap-1 p-0" aria-label="Child cards">
              {children.map((child, i) => (
                <li key={child.id} className="flex items-center gap-2 rounded-row border border-line px-2 py-1.5">
                  <Link to={PATHS.card(child.id)} className="flex min-w-0 flex-1 items-center gap-2 hover:underline">
                    <span className="truncate">{child.name}</span>
                    <OwnerChip owner={ownerOf(child, people)} />
                    {childrenOf(index, child.id).length > 0 && <span className="text-xs text-grey">split · {childrenOf(index, child.id).length}</span>}
                  </Link>
                  <button type="button" aria-label={`Move ${child.name} up`} disabled={i === 0} onClick={() => move(i, -1)} className="rounded p-1 text-grey hover:bg-hover disabled:opacity-30">
                    <ArrowUp size={14} />
                  </button>
                  <button type="button" aria-label={`Move ${child.name} down`} disabled={i === children.length - 1} onClick={() => move(i, 1)} className="rounded p-1 text-grey hover:bg-hover disabled:opacity-30">
                    <ArrowDown size={14} />
                  </button>
                </li>
              ))}
            </ol>
          )}
        </section>
      </div>
      {splitOpen && <SplitDialog card={card} open onClose={() => setSplitOpen(false)} />}
    </aside>
  )
}

/** The name, edited in place: Enter or blur saves, Escape reverts. */
function NameField({ card }: { card: Card }) {
  const { updateCard } = useActions()
  const [value, setValue] = useState(card.name)
  const [editing, setEditing] = useState(false)
  const cancelled = useRef(false)
  const commit = (): void => {
    setEditing(false)
    if (cancelled.current) {
      cancelled.current = false
      return setValue(card.name)
    }
    const next = value.trim()
    if (!next || next === card.name) return setValue(card.name)
    void updateCard(card.id, { name: next }).catch(() => setValue(card.name))
  }
  return (
    <input
      aria-label="Name"
      value={editing ? value : card.name}
      onFocus={() => {
        setValue(card.name)
        setEditing(true)
      }}
      onChange={(e) => setValue(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
        if (e.key === 'Escape') {
          e.stopPropagation()
          cancelled.current = true
          ;(e.target as HTMLInputElement).blur()
        }
      }}
      className="w-full rounded-row border border-transparent bg-transparent px-1 text-title font-semibold outline-none hover:border-line focus:border-primary"
    />
  )
}

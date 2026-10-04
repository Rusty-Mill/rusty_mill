import { ArrowDown, ArrowUp, ChevronRight, Merge, Scissors, Trash2, X } from 'lucide-react'
import { useMemo, useRef, useState } from 'react'
import { StaleError } from '@/api/errors'
import { Link, useNavigate } from 'react-router-dom'
import { SUITS, type Card, type CardPatch, type Suit } from '@/api/types'
import { PATHS } from '@/app/paths'
import { useActions, useData } from '@/app/services'
import { OwnerChip, ownerOf, StateBadge, suitBg, suitText } from '@/components/badges'
import { Confirm } from '@/components/Confirm'
import { Tooltip } from '@/components/Tooltip'
import { chainToRoot, childrenOf, subtree, type CardIndex } from '@/store/derive'
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
  suit: Suit
  parentCardId: string | null
  conception: string
  planning: string
  execution: string
  minimumStandardOfCare: string[]
  notes: string
}

const draftOf = (c: Card): Draft => ({ suit: c.suit, parentCardId: c.parentCardId, conception: c.conception, planning: c.planning, execution: c.execution, minimumStandardOfCare: [...c.minimumStandardOfCare], notes: c.notes })
type DraftField = keyof Draft
const draftFields: DraftField[] = ['suit', 'parentCardId', 'conception', 'planning', 'execution', 'minimumStandardOfCare', 'notes']
const sameDraftValue = (left: Draft[DraftField], right: Draft[DraftField]): boolean =>
  Array.isArray(left) && Array.isArray(right) ? left.join('\u0000') === right.join('\u0000') : left === right

const zeroGenerations = (): Record<DraftField, number> => ({ suit: 0, parentCardId: 0, conception: 0, planning: 0, execution: 0, minimumStandardOfCare: 0, notes: 0 })

function CardDetail({ card }: { card: Card }) {
  const navigate = useNavigate()
  const people = useData((s) => s.people)
  const index = useData((s) => s.index)
  const { updateCard, reorderChildren, deleteCard, unsplit } = useActions()
  const [draft, setDraft] = useState<Draft>(() => draftOf(card))
  // Equality with the previous server value cannot tell an untouched field from a real edit
  // back to that value. Generations make that distinction, including while Save is in flight.
  const editGenerations = useRef(zeroGenerations())
  const cleanGenerations = useRef(zeroGenerations())
  // When the card changes under the draft (a reset, a background refresh), fields the user has not touched follow it.
  const [base, setBase] = useState(card)
  if (base !== card) {
    const now = draftOf(card)
    const next = { ...draft }
    for (const field of draftFields) {
      if (editGenerations.current[field] === cleanGenerations.current[field]) Object.assign(next, { [field]: now[field] })
    }
    setDraft(next)
    setBase(card)
  }
  // The version the edit was started from. It stays put while the card moves on underneath (a
  // refresh, another tab), so a save is judged against what the user actually saw.
  const [draftBase, setDraftBase] = useState(card.etag)
  const [saving, setSaving] = useState(false)
  const [splitOpen, setSplitOpen] = useState(false)
  const [confirm, setConfirm] = useState<'delete' | 'unsplit' | null>(null)

  const chain = useMemo(() => chainToRoot(index, card.id), [index, card.id])
  const children = childrenOf(index, card.id)
  const below = subtree(index, card.id).length - 1
  const owner = ownerOf(card, people)
  const parents = useMemo(() => parentOptions(index, card.id), [index, card.id])

  /** Only the fields that differ from the card go in the PATCH. */
  const changes = useMemo((): CardPatch => {
    const p: CardPatch = {}
    if (draft.suit !== card.suit) p.suit = draft.suit
    if (draft.parentCardId !== card.parentCardId) p.parentCardId = draft.parentCardId
    if (draft.conception !== card.conception) p.conception = draft.conception
    if (draft.planning !== card.planning) p.planning = draft.planning
    if (draft.execution !== card.execution) p.execution = draft.execution
    if (draft.minimumStandardOfCare.join('\u0000') !== card.minimumStandardOfCare.join('\u0000')) p.minimumStandardOfCare = draft.minimumStandardOfCare
    if (draft.notes !== card.notes) p.notes = draft.notes
    return p
  }, [draft, card])
  const dirty = Object.keys(changes).length > 0
  /** Edits are pending and the card is no longer the one they were made against. */
  const conflicted = dirty && draftBase !== card.etag

  /** The first change of a clean draft records which version it is based on. */
  const edit = (next: Draft): void => {
    if (!dirty) setDraftBase(card.etag)
    for (const field of draftFields) {
      if (!sameDraftValue(draft[field], next[field])) {
        editGenerations.current[field]++
        // A manual revert with no request in flight is clean again and should follow a later
        // refresh. During a save, however, the same value can be a deliberate newer edit back
        // to the pre-save value, so its generation must remain outstanding.
        if (!saving && sameDraftValue(next[field], draftOf(card)[field])) {
          cleanGenerations.current[field] = editGenerations.current[field]
        }
      }
    }
    setDraft(next)
  }

  const save = async (base: string): Promise<void> => {
    const submittedChanges = changes
    const submittedGenerations = { ...editGenerations.current }
    setSaving(true)
    try {
      // The store merges the saved card in; the follow-the-card logic above then clears the
      // fields that were just saved and keeps anything typed while the request was in flight.
      const saved = await updateCard(card.id, submittedChanges, base)
      setDraft((current) => {
        const next = { ...current }
        for (const field of draftFields) {
          if (!(field in submittedChanges) || editGenerations.current[field] !== submittedGenerations[field]) continue
          Object.assign(next, { [field]: draftOf(saved)[field] })
          cleanGenerations.current[field] = submittedGenerations[field]
        }
        return next
      })
      setDraftBase(saved.etag)
    } catch (e) {
      // Toasted by the store. The draft stays; on a 412 the card is now the newer one, so the
      // conflict notice below offers to overwrite it or drop the edits.
      if (!(e instanceof StaleError)) return
    } finally {
      setSaving(false)
    }
  }

  const deal = (value: string): void => {
    void updateCard(card.id, { ownerId: value || null }, card.etag).catch(() => undefined)
  }

  /** Move child `i` by `delta` as one order request. */
  const move = (i: number, delta: number): void => {
    const ids = children.map((c) => c.id)
    const j = i + delta
    if (!ids[i] || !ids[j]) return
    ;[ids[i], ids[j]] = [ids[j]!, ids[i]!]
    void reorderChildren(card.id, ids).catch(() => undefined)
  }

  const doDelete = async (): Promise<void> => {
    setConfirm(null)
    try {
      await deleteCard(card.id)
      navigate(card.parentCardId ? PATHS.card(card.parentCardId) : PATHS.deck)
    } catch {
      /* toasted by the store */
    }
  }

  const doUnsplit = async (): Promise<void> => {
    setConfirm(null)
    await unsplit(card.id).catch(() => undefined)
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

        <div className="grid grid-cols-[auto_1fr] items-center gap-x-2 gap-y-1.5 text-s">
          <span className="text-grey">Suit</span>
          <select aria-label="Suit" value={draft.suit} onChange={(e) => edit({ ...draft, suit: e.target.value as Suit })} className="field h-8">
            {SUITS.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
          <span className="text-grey">Parent</span>
          <select aria-label="Parent" value={draft.parentCardId ?? ''} onChange={(e) => edit({ ...draft, parentCardId: e.target.value || null })} className="field h-8">
            <option value="">None (top level)</option>
            {parents.map((p) => (
              <option key={p.id} value={p.id}>
                {p.label}
              </option>
            ))}
          </select>
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
            <textarea aria-label={field[0]!.toUpperCase() + field.slice(1)} value={draft[field]} onChange={(e) => edit({ ...draft, [field]: e.target.value })} rows={3} className="field resize-y" />
          </label>
        ))}

        <section className="flex flex-col gap-1" aria-labelledby="msc-h">
          <h3 id="msc-h" className="m-0 text-s font-semibold">
            Minimum standard of care
          </h3>
          <StandardsEditor value={draft.minimumStandardOfCare} onChange={(minimumStandardOfCare) => edit({ ...draft, minimumStandardOfCare })} />
        </section>

        <label className="flex flex-col gap-1">
          <span className="text-s font-semibold">Notes</span>
          <textarea aria-label="Notes" value={draft.notes} onChange={(e) => edit({ ...draft, notes: e.target.value })} rows={2} placeholder="Notes never make a card edited." className="field resize-y" />
        </label>

        {dirty && (
          <div className="sticky bottom-0 flex flex-col gap-2 border-t border-line bg-surface py-2">
            {conflicted && (
              <p role="alert" className="m-0 text-s text-danger">
                This card changed elsewhere while you were editing. Your changes are kept; the fields you did not touch show the new version. Saving replaces the new version of the fields you changed.
              </p>
            )}
            <div className="flex items-center justify-end gap-2">
              <button type="button" onClick={() => {
                setDraft(draftOf(card))
                cleanGenerations.current = { ...editGenerations.current }
                setDraftBase(card.etag)
              }} className="btn border-transparent">
                {conflicted ? 'Discard mine' : 'Cancel'}
              </button>
              <button type="button" onClick={() => void save(conflicted ? card.etag : draftBase)} disabled={saving} className="btn-primary">
                {conflicted ? 'Overwrite' : 'Save'}
              </button>
            </div>
          </div>
        )}

        {card.origin === 'deck' ? (
          <BaselineBlock card={card} />
        ) : (
          <p className="rounded-card border border-line px-3 py-2 text-s text-grey">Custom card: made by this family, with no deck original to compare against.</p>
        )}

        <section aria-labelledby="children-h" className="flex flex-col gap-2">
          <div className="flex items-center justify-between gap-2">
            <h3 id="children-h" className="m-0 text-base font-semibold">
              Children {children.length > 0 && <span className="font-normal text-grey">· {children.length}</span>}
            </h3>
            <div className="flex items-center gap-1.5">
              {children.length > 0 && (
                <button type="button" onClick={() => setConfirm('unsplit')} className="btn">
                  <Merge size={14} /> Unsplit…
                </button>
              )}
              <button type="button" onClick={() => setSplitOpen(true)} className="btn">
                <Scissors size={14} /> Split…
              </button>
            </div>
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

        <div className="flex justify-end border-t border-line pt-3">
          <Tooltip label={children.length > 0 ? `Unsplit first: it has ${children.length} ${children.length === 1 ? 'child' : 'children'}` : 'Deletes this card for good'}>
            <button type="button" aria-label="Delete card" disabled={children.length > 0} onClick={() => setConfirm('delete')} className="btn text-danger">
              <Trash2 size={14} /> Delete card
            </button>
          </Tooltip>
        </div>
      </div>
      {splitOpen && <SplitDialog card={card} open onClose={() => setSplitOpen(false)} />}
      <Confirm
        open={confirm === 'delete'}
        title={`Delete "${card.name}"?`}
        message={card.origin === 'deck' ? 'The deck card goes from the board; its owner loses it. Load the deck again to bring it back.' : 'The card goes from the board; its owner loses it. This cannot be undone.'}
        confirmLabel="Delete"
        danger
        onConfirm={() => void doDelete()}
        onCancel={() => setConfirm(null)}
      />
      <Confirm
        open={confirm === 'unsplit'}
        title={`Unsplit "${card.name}"?`}
        message={`This removes ${below} ${below === 1 ? 'card' : 'cards'} under it (every child and grandchild, with their owners) and keeps "${card.name}" itself.`}
        confirmLabel={`Remove ${below} ${below === 1 ? 'card' : 'cards'}`}
        danger
        onConfirm={() => void doUnsplit()}
        onCancel={() => setConfirm(null)}
      />
    </aside>
  )
}

/** Every card that could become the parent: not the card itself nor anything under it. Labelled by its chain. */
function parentOptions(index: CardIndex, id: string): { id: string; label: string }[] {
  const excluded = new Set(subtree(index, id).map((c) => c.id))
  return [...index.byId.values()].filter((c) => !excluded.has(c.id)).map((c) => ({ id: c.id, label: chainToRoot(index, c.id).map((x) => (x.number !== null ? `#${x.number} ${x.name}` : x.name)).join(' › ') }))
}

/** The name, edited in place: Enter or blur saves, Escape reverts. */
function NameField({ card }: { card: Card }) {
  const { updateCard } = useActions()
  const [value, setValue] = useState(card.name)
  const [editing, setEditing] = useState(false)
  const [hasDraft, setHasDraft] = useState(false)
  const [conflicted, setConflicted] = useState(false)
  const [saving, setSaving] = useState(false)
  const editBase = useRef(card.etag)
  const submitting = useRef(false)
  const cancelled = useRef(false)
  const editGeneration = useRef(0)
  const commit = (base = editBase.current, overwrite = false): void => {
    setEditing(false)
    if (cancelled.current) {
      cancelled.current = false
      setHasDraft(false)
      return setValue(card.name)
    }
    if (submitting.current) return
    if (conflicted && !overwrite) return
    const next = value.trim()
    if (!next || next === card.name) {
      setHasDraft(false)
      return setValue(card.name)
    }
    submitting.current = true
    const submittedGeneration = editGeneration.current
    setSaving(true)
    void updateCard(card.id, { name: next }, base)
      .then((saved) => {
        // The name stays editable while the request is pending. Do not let its response erase
        // a newer draft, even when that draft deliberately equals the old server name.
        if (editGeneration.current === submittedGeneration) {
          setValue(saved.name)
          setHasDraft(false)
          setConflicted(false)
        }
        // A newer draft was still made against this request's result once it succeeds.
        // Advancing only its base preserves that draft without letting the old value replace it.
        editBase.current = saved.etag
      })
      .catch((error: unknown) => {
        // A discarded or superseded draft must not be brought back as a conflict by an
        // older request that happens to finish after it.
        if (error instanceof StaleError && editGeneration.current === submittedGeneration) setConflicted(true)
      })
      .finally(() => {
        submitting.current = false
        setSaving(false)
      })
  }
  return (
    <div className="flex flex-col gap-1.5">
      <input
        aria-label="Name"
        value={editing || hasDraft ? value : card.name}
        onFocus={() => {
          if (!hasDraft) {
            setValue(card.name)
            editBase.current = card.etag
          }
          setEditing(true)
        }}
        onChange={(e) => {
          editGeneration.current++
          setValue(e.target.value)
          setHasDraft(true)
        }}
        onBlur={() => commit()}
        onKeyDown={(e) => {
          if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
          if (e.key === 'Escape') {
            e.stopPropagation()
            // Invalidate an in-flight request before blur. commit consumes the cancellation
            // even while another submission is pending, so it cannot leak into the next edit.
            editGeneration.current++
            cancelled.current = true
            setConflicted(false)
            ;(e.target as HTMLInputElement).blur()
          }
        }}
        className="w-full rounded-row border border-transparent bg-transparent px-1 text-title font-semibold outline-none hover:border-line focus:border-primary"
      />
      {conflicted && (
        <div className="flex flex-col gap-1.5">
          <p role="alert" className="m-0 text-s text-danger">This name changed elsewhere. Your name is kept; overwrite the newer name or discard yours.</p>
          <div className="flex justify-end gap-2">
            <button type="button" className="btn border-transparent" onClick={() => {
              setValue(card.name)
              setHasDraft(false)
              setConflicted(false)
              editBase.current = card.etag
            }}>Discard mine</button>
            <button type="button" className="btn-primary" disabled={saving} onClick={() => {
              setConflicted(false)
              commit(card.etag, true)
            }}>Overwrite</button>
          </div>
        </div>
      )}
    </div>
  )
}

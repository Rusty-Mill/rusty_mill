import type { Card, CardState, Person, Suit } from '@/api/types'

/** Tailwind colour names per suit (see tokens.css). */
export const SUIT_COLOR: Record<Suit, string> = {
  Home: 'suit-home',
  Out: 'suit-out',
  Caregiving: 'suit-caregiving',
  Magic: 'suit-magic',
  Wild: 'suit-wild',
  'Unicorn Space': 'suit-unicorn',
}

export const suitBg: Record<Suit, string> = {
  Home: 'bg-suit-home',
  Out: 'bg-suit-out',
  Caregiving: 'bg-suit-caregiving',
  Magic: 'bg-suit-magic',
  Wild: 'bg-suit-wild',
  'Unicorn Space': 'bg-suit-unicorn',
}

export const suitText: Record<Suit, string> = {
  Home: 'text-suit-home',
  Out: 'text-suit-out',
  Caregiving: 'text-suit-caregiving',
  Magic: 'text-suit-magic',
  Wild: 'text-suit-wild',
  'Unicorn Space': 'text-suit-unicorn',
}

/** `edited` in amber, `custom` in violet; an original card shows nothing (or a quiet label when `showOriginal`). */
export function StateBadge({ state, showOriginal = false }: { state: CardState; showOriginal?: boolean }) {
  if (state === 'original') return showOriginal ? <span className="rounded-full border border-line px-1.5 text-xs text-grey">original</span> : null
  const cls = state === 'edited' ? 'bg-amber/15 text-amber' : 'bg-violet/15 text-violet'
  return <span className={`rounded-full px-1.5 text-xs font-medium ${cls}`}>{state}</span>
}

export function SplitBadge({ count }: { count: number }) {
  if (count === 0) return null
  return <span className="rounded-full bg-selected px-1.5 text-xs text-grey">split · {count}</span>
}

export const initial = (name: string): string => (name.trim()[0] ?? '?').toUpperCase()

/** Who holds the card: an initial and the name, or "Unassigned" in a muted style. */
export function OwnerChip({ owner, size = 's' }: { owner: Person | null | undefined; size?: 's' | 'base' }) {
  const text = size === 's' ? 'text-xs' : 'text-s'
  if (!owner) return <span data-testid="owner" className={`inline-flex items-center gap-1 rounded-full border border-dashed border-line px-1.5 ${text} text-grey`}>Unassigned</span>
  return (
    <span data-testid="owner" className={`inline-flex items-center gap-1 rounded-full bg-selected pl-0.5 pr-1.5 ${text}`}>
      <span aria-hidden className="flex h-4 w-4 items-center justify-center rounded-full bg-primary text-[10px] font-semibold text-white">
        {initial(owner.name)}
      </span>
      {owner.name}
    </span>
  )
}

export const ownerOf = (card: Card, people: Person[]): Person | null => (card.ownerId ? (people.find((p) => p.id === card.ownerId) ?? null) : null)

import { Check } from 'lucide-react'
import { Link } from 'react-router-dom'
import type { Card, Person } from '@/api/types'
import { PATHS } from '@/app/paths'
import { OwnerChip, ownerOf, SplitBadge, StateBadge, suitBg } from '@/components/badges'

interface Props {
  card: Card
  people: Person[]
  childCount: number
  selected: boolean
  /** Picking the deck: the tile is a checkbox for "in our deck" instead of a link. */
  choosing?: boolean
  onToggle?: (card: Card) => void
}

/** Every tile is the same size whatever the name or badges: the board is a grid of equal cards. */
const frame = 'relative flex h-full w-full flex-col overflow-hidden rounded-card border bg-surface text-left shadow-card transition-colors hover:bg-hover'

/** One card on the board: suit stripe, number, name, owner, badges. */
export function CardTile({ card, people, childCount, selected, choosing = false, onToggle }: Props) {
  const body = (
    <>
      <span aria-hidden className={`h-1.5 shrink-0 ${suitBg[card.suit]}`} />
      <span className="flex min-h-0 flex-1 flex-col gap-1.5 p-3">
        <span className="flex items-start justify-between gap-2">
          <span className="line-clamp-2 font-medium leading-5">{card.name}</span>
          {choosing ? (
            <span aria-hidden className={`flex h-5 w-5 shrink-0 items-center justify-center rounded border ${card.inPlay ? 'border-primary bg-primary text-white' : 'border-line'}`}>
              {card.inPlay && <Check size={14} />}
            </span>
          ) : (
            card.number !== null && <span className="shrink-0 text-xs text-grey">#{card.number}</span>
          )}
        </span>
        <span className="mt-auto flex flex-wrap items-center gap-1.5">
          {card.inPlay ? (
            <>
              <OwnerChip owner={ownerOf(card, people)} />
              <StateBadge state={card.state} />
              <SplitBadge count={childCount} />
            </>
          ) : (
            <span className="rounded-full border border-dashed border-line px-1.5 text-xs text-grey">set aside</span>
          )}
        </span>
      </span>
    </>
  )
  const tone = card.inPlay ? '' : 'opacity-60'
  if (choosing) {
    return (
      <button type="button" role="checkbox" aria-checked={card.inPlay} aria-label={`${card.name} is in our deck`} data-testid="card-tile" onClick={() => onToggle?.(card)} className={`${frame} ${tone} ${card.inPlay ? 'border-primary/60' : 'border-line'}`}>
        {body}
      </button>
    )
  }
  return (
    <Link to={PATHS.card(card.id)} aria-current={selected ? 'true' : undefined} data-testid="card-tile" className={`${frame} ${tone} ${selected ? 'border-primary ring-1 ring-primary' : 'border-line'}`}>
      {body}
    </Link>
  )
}

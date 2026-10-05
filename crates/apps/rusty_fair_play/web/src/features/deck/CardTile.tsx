import { Check } from 'lucide-react'
import { Link } from 'react-router-dom'
import type { Card, Person } from '@/api/types'
import { PATHS } from '@/app/paths'
import { OwnerChip, ownerOf, SplitBadge, StateBadge, suitText } from '@/components/badges'

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
const frame = 'relative flex h-full w-full flex-col overflow-hidden rounded-[14px] border bg-paper py-2.5 pl-8 pr-3 text-left shadow-card transition hover:brightness-95 [[data-theme=dark]_&]:hover:brightness-125'

/**
 * One card on the board, after the printed deck: a cream card, the name in spaced capitals, and the
 * suit written up the left edge in its colour. (The look, not the artwork.)
 */
export function CardTile({ card, people, childCount, selected, choosing = false, onToggle }: Props) {
  const body = (
    <>
      <span className="flex items-start justify-between gap-2">
        <span className="line-clamp-2 font-serif text-[12.5px] font-semibold uppercase leading-4 tracking-[0.06em]">{card.name}</span>
        {choosing ? (
          <span aria-hidden className={`flex h-5 w-5 shrink-0 items-center justify-center rounded border ${card.inPlay ? 'border-primary bg-primary text-white' : 'border-grey/60'}`}>
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
          <span className="rounded-full border border-dashed border-grey/60 px-1.5 text-xs text-grey">set aside</span>
        )}
      </span>
      {/* After the name in the DOM, so the tile's text still starts with the card; placed on the left edge. */}
      <span aria-hidden className={`absolute inset-y-0 left-0 flex w-7 items-center justify-center overflow-hidden py-2 text-[9px] font-semibold uppercase tracking-[0.16em] [writing-mode:vertical-rl] rotate-180 ${suitText[card.suit]}`}>
        <span className="overflow-hidden text-ellipsis whitespace-nowrap">{card.suit}</span>
      </span>
    </>
  )
  const tone = card.inPlay ? '' : 'opacity-60'
  if (choosing) {
    return (
      <button type="button" role="checkbox" aria-checked={card.inPlay} aria-label={`${card.name} is in our deck`} data-testid="card-tile" title={card.name} onClick={() => onToggle?.(card)} className={`${frame} ${tone} ${card.inPlay ? 'border-primary/60' : 'border-line'}`}>
        {body}
      </button>
    )
  }
  return (
    <Link to={PATHS.card(card.id)} aria-current={selected ? 'true' : undefined} data-testid="card-tile" title={card.name} className={`${frame} ${tone} ${selected ? 'border-primary ring-2 ring-primary' : 'border-line'}`}>
      {body}
    </Link>
  )
}

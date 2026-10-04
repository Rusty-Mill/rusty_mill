import { Link } from 'react-router-dom'
import type { Card, Person } from '@/api/types'
import { PATHS } from '@/app/paths'
import { OwnerChip, ownerOf, SplitBadge, StateBadge, suitBg } from '@/components/badges'

interface Props {
  card: Card
  people: Person[]
  childCount: number
  selected: boolean
}

/** One card on the board: suit stripe, number, name, owner, badges. */
export function CardTile({ card, people, childCount, selected }: Props) {
  return (
    <Link
      to={PATHS.card(card.id)}
      aria-current={selected ? 'true' : undefined}
      data-testid="card-tile"
      className={`flex flex-col overflow-hidden rounded-card border bg-surface shadow-card transition-colors hover:bg-hover ${selected ? 'border-primary ring-1 ring-primary' : 'border-line'}`}
    >
      <span aria-hidden className={`h-1.5 ${suitBg[card.suit]}`} />
      <span className="flex flex-1 flex-col gap-1.5 p-3">
        <span className="flex items-start justify-between gap-2">
          <span className="font-medium leading-5">{card.name}</span>
          {card.number !== null && <span className="shrink-0 text-xs text-grey">#{card.number}</span>}
        </span>
        <span className="mt-auto flex flex-wrap items-center gap-1.5">
          <OwnerChip owner={ownerOf(card, people)} />
          <StateBadge state={card.state} />
          <SplitBadge count={childCount} />
        </span>
      </span>
    </Link>
  )
}

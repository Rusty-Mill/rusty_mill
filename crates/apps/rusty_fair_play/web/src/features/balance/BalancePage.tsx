import { useMemo } from 'react'
import { Link } from 'react-router-dom'
import { SUITS } from '@/api/types'
import { PATHS } from '@/app/paths'
import { useActions, useData } from '@/app/services'
import { initial, suitBg, suitText } from '@/components/badges'
import { balance, bySuit, undealt } from '@/store/derive'

/** Who holds how much, all cards and leaf cards only, and what is still undealt. */
export function BalancePage() {
  const people = useData((s) => s.people)
  const cards = useData((s) => s.cards)
  const index = useData((s) => s.index)
  const { updateCard } = useActions()
  const rows = useMemo(() => balance(index, cards, people), [index, cards, people])
  const left = useMemo(() => bySuit(undealt(index, cards)), [index, cards])
  const leftCount = SUITS.reduce((n, s) => n + left[s].length, 0)
  const maxAll = Math.max(1, ...rows.map((r) => r.all))

  return (
    <main className="flex min-w-0 flex-1 flex-col" aria-label="Balance">
      <header className="px-5 pt-4">
        <h1 className="text-h1 font-semibold">Balance</h1>
        <p className="text-s text-grey">Leaf cards count the work once: a split parent and its children are not both counted.</p>
      </header>
      <div className="scroll-thin flex-1 overflow-y-auto px-5 pb-8 pt-4">
        {people.length === 0 ? (
          <p className="text-grey">
            No people yet.{' '}
            <Link to={PATHS.players} className="text-primary underline">
              Add players
            </Link>{' '}
            to start dealing.
          </p>
        ) : (
          <ul className="flex max-w-3xl list-none flex-col gap-3 p-0" aria-label="Balance per person">
            {rows.map((row) => (
              <li key={row.person.id} className="rounded-card border border-line p-3" data-testid="balance-row">
                <div className="mb-2 flex items-center gap-2">
                  <span aria-hidden className="flex h-6 w-6 items-center justify-center rounded-full bg-primary text-xs font-semibold text-white">
                    {initial(row.person.name)}
                  </span>
                  <span className="font-semibold">{row.person.name}</span>
                </div>
                <Bar label="All cards" value={row.all} max={maxAll} />
                <Bar label="Leaf cards" value={row.leaves} max={maxAll} muted />
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-xs text-grey" aria-label={`${row.person.name} by suit`}>
                  {SUITS.map((s) => (
                    <span key={s} className="inline-flex items-center gap-1">
                      <span aria-hidden className={`h-2 w-2 rounded-full ${suitBg[s]}`} />
                      {s} {row.bySuit[s].all}
                      {row.bySuit[s].leaves !== row.bySuit[s].all && <span>({row.bySuit[s].leaves})</span>}
                    </span>
                  ))}
                </div>
              </li>
            ))}
          </ul>
        )}

        <section aria-labelledby="undealt-h" className="mt-8 max-w-3xl">
          <h2 id="undealt-h" className="text-title font-semibold">
            Still undealt <span className="font-normal text-grey">· {leftCount}</span>
          </h2>
          <p className="mb-3 text-s text-grey">Unassigned leaf cards: the ones nobody holds yet.</p>
          {leftCount === 0 && <p className="text-grey">Everything is dealt.</p>}
          {SUITS.map((suit) =>
            left[suit].length === 0 ? null : (
              <div key={suit} className="mb-4">
                <h3 className={`mb-1 text-base font-semibold ${suitText[suit]}`}>
                  {suit} <span className="font-normal text-grey">· {left[suit].length}</span>
                </h3>
                <ul className="flex list-none flex-col gap-1 p-0" aria-label={`Undealt ${suit} cards`}>
                  {left[suit].map((card) => (
                    <li key={card.id} className="flex items-center gap-2 rounded-row border border-line px-2 py-1">
                      <Link to={PATHS.card(card.id)} className="min-w-0 flex-1 truncate hover:underline">
                        {card.number !== null && <span className="mr-1 text-xs text-grey">#{card.number}</span>}
                        {card.name}
                      </Link>
                      <select aria-label={`Deal ${card.name} to`} value="" onChange={(e) => e.target.value && void updateCard(card.id, { ownerId: e.target.value }, card.etag).catch(() => undefined)} className="field h-7 w-auto py-0 text-s" disabled={people.length === 0}>
                        <option value="">Deal to…</option>
                        {people.map((p) => (
                          <option key={p.id} value={p.id}>
                            {p.name}
                          </option>
                        ))}
                      </select>
                    </li>
                  ))}
                </ul>
              </div>
            ),
          )}
        </section>
      </div>
    </main>
  )
}

function Bar({ label, value, max, muted = false }: { label: string; value: number; max: number; muted?: boolean }) {
  return (
    <div className="flex items-center gap-2 text-s">
      <span className="w-20 shrink-0 text-grey">{label}</span>
      <div className="h-3 flex-1 overflow-hidden rounded-full bg-selected" role="img" aria-label={`${label}: ${value}`}>
        <div className={`h-full rounded-full ${muted ? 'bg-primary/40' : 'bg-primary'}`} style={{ width: `${(value / max) * 100}%` }} />
      </div>
      <span className="w-8 shrink-0 text-right font-medium" data-testid={label === 'All cards' ? 'count-all' : 'count-leaves'}>
        {value}
      </span>
    </div>
  )
}

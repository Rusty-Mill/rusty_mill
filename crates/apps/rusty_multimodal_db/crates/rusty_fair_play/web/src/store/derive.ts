/**
 * Everything the pages need that is not stored: the tree (children, leaves,
 * the chain to the root), per-person balance in both variants, and counts.
 * Pure functions over the snapshot's arrays; the store memoises the index.
 */
import { SUITS, type Card, type CardState, type Person, type Suit } from '@/api/types'

export interface CardIndex {
  byId: Map<string, Card>
  /** Children of each card, ordered by position. */
  children: Map<string, Card[]>
  /** Cards with no children. */
  leafIds: Set<string>
  /** Top-level cards, in deck order. */
  roots: Card[]
}

export function indexCards(cards: Card[]): CardIndex {
  const byId = new Map(cards.map((c) => [c.id, c]))
  const children = new Map<string, Card[]>()
  const roots: Card[] = []
  for (const c of cards) {
    if (c.parentCardId && byId.has(c.parentCardId)) {
      const list = children.get(c.parentCardId) ?? []
      list.push(c)
      children.set(c.parentCardId, list)
    } else roots.push(c)
  }
  for (const list of children.values()) list.sort((a, b) => a.position - b.position || a.id.localeCompare(b.id))
  const leafIds = new Set(cards.filter((c) => !children.has(c.id)).map((c) => c.id))
  return { byId, children, leafIds, roots }
}

export const childrenOf = (index: CardIndex, id: string): Card[] => index.children.get(id) ?? []
export const isLeaf = (index: CardIndex, id: string): boolean => index.leafIds.has(id)

/** The card's ancestors from the top-level card down to the card itself. Stops on a cycle. */
export function chainToRoot(index: CardIndex, id: string): Card[] {
  const chain: Card[] = []
  const seen = new Set<string>()
  let at: string | null = id
  while (at && !seen.has(at)) {
    seen.add(at)
    const card = index.byId.get(at)
    if (!card) break
    chain.unshift(card)
    at = card.parentCardId
  }
  return chain
}

/** The card and everything under it, depth first. */
export function subtree(index: CardIndex, id: string): Card[] {
  const out: Card[] = []
  const seen = new Set<string>()
  const walk = (cardId: string): void => {
    if (seen.has(cardId)) return
    seen.add(cardId)
    const card = index.byId.get(cardId)
    if (!card) return
    out.push(card)
    for (const child of childrenOf(index, cardId)) walk(child.id)
  }
  walk(id)
  return out
}

/** The unowned leaf cards: what is really still undealt. */
export const undealt = (index: CardIndex, cards: Card[]): Card[] => cards.filter((c) => c.ownerId === null && index.leafIds.has(c.id))

export interface Balance {
  person: Person
  all: number
  leaves: number
  bySuit: Record<Suit, { all: number; leaves: number }>
}

const emptySuits = (): Record<Suit, { all: number; leaves: number }> =>
  Object.fromEntries(SUITS.map((s) => [s, { all: 0, leaves: 0 }])) as Record<Suit, { all: number; leaves: number }>

/** Cards held per person: every card, and leaf cards only (a split parent plus its children would double-count). */
export function balance(index: CardIndex, cards: Card[], people: Person[]): Balance[] {
  const rows = new Map(people.map((p) => [p.id, { person: p, all: 0, leaves: 0, bySuit: emptySuits() } satisfies Balance]))
  for (const c of cards) {
    const row = c.ownerId ? rows.get(c.ownerId) : undefined
    if (!row) continue
    row.all++
    row.bySuit[c.suit].all++
    if (index.leafIds.has(c.id)) {
      row.leaves++
      row.bySuit[c.suit].leaves++
    }
  }
  return [...rows.values()]
}

export const stateCounts = (cards: Card[]): Record<CardState, number> => {
  const counts: Record<CardState, number> = { original: 0, edited: 0, custom: 0 }
  for (const c of cards) counts[c.state]++
  return counts
}

export const bySuit = (cards: Card[]): Record<Suit, Card[]> => {
  const groups = Object.fromEntries(SUITS.map((s) => [s, [] as Card[]])) as Record<Suit, Card[]>
  for (const c of cards) groups[c.suit].push(c)
  return groups
}

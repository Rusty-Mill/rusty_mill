/** The wire shapes of `rusty_fair_play`'s JSON API (see the crate README). */

export const SUITS = ['Home', 'Out', 'Caregiving', 'Magic', 'Wild', 'Unicorn Space'] as const
export type Suit = (typeof SUITS)[number]

export type CardState = 'original' | 'edited' | 'custom'
export type Origin = 'deck' | 'family'

export interface Person {
  id: string
  name: string
  player: number
}

export interface Card {
  id: string
  /** 1..100 for a deck card, null for a family-made one. */
  number: number | null
  name: string
  suit: Suit
  /** The tree: a split-off card's parent. */
  parentCardId: string | null
  /** Sibling order under `parentCardId`. */
  position: number
  /** null = unassigned. */
  ownerId: string | null
  conception: string
  planning: string
  execution: string
  minimumStandardOfCare: string[]
  notes: string
  origin: Origin
  baselineId: string | null
  /** Derived against the baseline, never stored. */
  state: CardState
  /** Changes whenever the card changes; sent back as `If-Match` on writes to the card. */
  etag: string
  /** Like `etag` but over the card and everything under it; guards `unsplitCard` and `reorderChildren`. */
  treeEtag: string
}

export interface Baseline {
  id: string
  number: number
  name: string
  suit: Suit
  conception: string
  planning: string
  execution: string
  minimumStandardOfCare: string[]
}

/** The six comparable fields, by their wire names. */
export type DiffField = 'name' | 'suit' | 'conception' | 'planning' | 'execution' | 'minimum_standard_of_care'

export interface FieldDiff {
  field: DiffField
  card: string
  baseline: string
}

export interface BaselineResponse {
  /** null for a family card. */
  baseline: Baseline | null
  diff: FieldDiff[]
}

export interface Snapshot {
  people: Person[]
  cards: Card[]
}

/** Absent = keep; `ownerId`/`parentCardId` null = clear. */
export interface CardPatch {
  name?: string
  suit?: Suit
  conception?: string
  planning?: string
  execution?: string
  minimumStandardOfCare?: string[]
  notes?: string
  ownerId?: string | null
  parentCardId?: string | null
  position?: number
}

export interface NewCard {
  id?: string
  name: string
  suit: Suit
  parentCardId?: string | null
  ownerId?: string | null
  conception?: string
  planning?: string
  execution?: string
  minimumStandardOfCare?: string[]
  notes?: string
}

export interface SplitChild {
  id?: string
  name: string
  ownerId?: string | null
  conception?: string
  planning?: string
  execution?: string
  minimumStandardOfCare?: string[]
  notes?: string
}

export interface SplitInput {
  children: SplitChild[]
  /** Absent = the parent keeps its owner; null = the parent becomes unassigned. */
  ownerId?: string | null
  notes?: string
}

export interface SplitResult {
  parent: Card
  children: Card[]
}

export interface UnsplitResult {
  parent: Card
  /** The subtree that went, deepest first. */
  deleted: string[]
}

export interface Tally {
  created: number
  existing: number
}

export interface SeedResult {
  cardDefaults: Tally
  cards: Tally
}

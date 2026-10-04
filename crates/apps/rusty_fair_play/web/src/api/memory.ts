/**
 * An in-browser backend with the same rules as `rusty_fair_play`'s service
 * (ADR-0137): state derived against a kept baseline, splits with positions,
 * reset, unknown-owner and cycle refusals, 409 on a duplicate name.
 * Used for the demo (`?adapter=memory`) and the unit tests; the contract
 * suite holds it to the real server's behaviour.
 */
import type { ApiClient } from './client'
import { DEMO_DECK } from './demoDeck'
import { ConflictError, InvalidError, NotFoundError } from './errors'
import { SUITS, type Baseline, type BaselineResponse, type Card, type CardPatch, type CardState, type DiffField, type FieldDiff, type NewCard, type Person, type SeedResult, type Snapshot, type SplitInput, type SplitResult, type Suit } from './types'
import { newId } from '@/lib/id'

const MAX_TEXT_LEN = 10_000
const MAX_STANDARDS = 50

/** A card as stored: everything but the derived `state`. */
type Stored = Omit<Card, 'state'>

interface Data {
  people: Person[]
  baselines: Baseline[]
  cards: Stored[]
}

export interface MemoryOptions {
  /** Persist across reloads (the demo); omit for tests. */
  storage?: Storage | null
  key?: string
  /** The baselines `seed()` loads. Defaults to the dozen in `demoDeck.ts`. */
  deck?: Baseline[]
}

const invalid = (message: string): InvalidError => new InvalidError(422, message)

export class MemoryAdapter implements ApiClient {
  private data: Data = { people: [], baselines: [], cards: [] }
  private readonly storage: Storage | null
  private readonly key: string
  private readonly deck: Baseline[]

  constructor(options: MemoryOptions = {}) {
    this.storage = options.storage ?? null
    this.key = options.key ?? 'fair-play:memory:v1'
    this.deck = options.deck ?? DEMO_DECK
    this.load()
  }

  // ---- persistence ---------------------------------------------------

  private load(): void {
    try {
      const raw = this.storage?.getItem(this.key)
      if (raw) this.data = JSON.parse(raw) as Data
    } catch {
      /* unreadable: start empty */
    }
  }

  private save(): void {
    try {
      this.storage?.setItem(this.key, JSON.stringify(this.data))
    } catch {
      /* storage unavailable: the data lasts until reload */
    }
  }

  // ---- helpers -------------------------------------------------------

  private stored(id: string): Stored {
    const card = this.data.cards.find((c) => c.id === id)
    if (!card) throw new NotFoundError('card not found')
    return card
  }

  private baselineOf(card: Stored): Baseline | null {
    if (!card.baselineId) return null
    const b = this.data.baselines.find((x) => x.id === card.baselineId)
    if (!b) throw new Error(`baseline ${card.baselineId} missing for ${card.id}`)
    return b
  }

  private view(card: Stored): Card {
    const baseline = this.baselineOf(card)
    const state: CardState = card.origin === 'family' ? 'custom' : diff(card, baseline!).length === 0 ? 'original' : 'edited'
    return { ...clone(card), state }
  }

  private checkOwner(owner: string | null | undefined): void {
    if (owner && !this.data.people.some((p) => p.id === owner)) throw invalid('unknown owner')
  }

  private checkParent(card: Stored): void {
    if (!card.parentCardId) return
    if (card.parentCardId === card.id) throw invalid('a card cannot be its own parent')
    if (!this.data.cards.some((c) => c.id === card.parentCardId)) throw invalid('parent card not found')
    // Walk up from the parent; reaching the card again would make a cycle.
    const seen = new Set<string>()
    let at: string | null = card.parentCardId
    while (at) {
      if (at === card.id) throw invalid('a card cannot become its own ancestor')
      if (seen.has(at)) break
      seen.add(at)
      at = this.data.cards.find((c) => c.id === at)?.parentCardId ?? null
    }
  }

  private nextPosition(parentId: string | null): number {
    const siblings = this.data.cards.filter((c) => c.parentCardId === parentId)
    return siblings.length === 0 ? 0 : Math.max(...siblings.map((c) => c.position)) + 1
  }

  // ---- reads ---------------------------------------------------------

  async snapshot(): Promise<Snapshot> {
    const people = [...this.data.people].sort((a, b) => a.player - b.player || a.name.localeCompare(b.name))
    const cards = [...this.data.cards]
      .sort((a, b) => (a.number ?? Infinity) - (b.number ?? Infinity) || a.position - b.position || a.id.localeCompare(b.id))
      .map((c) => this.view(c))
    return { people: people.map(clone), cards }
  }

  async getCard(id: string): Promise<Card> {
    return this.view(this.stored(id))
  }

  async baseline(id: string): Promise<BaselineResponse> {
    const card = this.stored(id)
    const baseline = this.baselineOf(card)
    return { baseline: baseline ? clone(baseline) : null, diff: baseline ? diff(card, baseline) : [] }
  }

  // ---- seed ----------------------------------------------------------

  async seed(): Promise<SeedResult> {
    const result: SeedResult = { cardDefaults: { created: 0, existing: 0 }, cards: { created: 0, existing: 0 } }
    for (const b of this.deck) {
      if (this.data.baselines.some((x) => x.id === b.id)) result.cardDefaults.existing++
      else {
        this.data.baselines.push(clone(b))
        result.cardDefaults.created++
      }
      if (this.data.cards.some((c) => c.baselineId === b.id)) result.cards.existing++
      else {
        this.data.cards.push({
          id: deckCardId(b.number),
          number: b.number,
          name: b.name,
          suit: b.suit,
          parentCardId: null,
          position: b.number,
          ownerId: null,
          conception: b.conception,
          planning: b.planning,
          execution: b.execution,
          minimumStandardOfCare: [...b.minimumStandardOfCare],
          notes: '',
          origin: 'deck',
          baselineId: b.id,
        })
        result.cards.created++
      }
    }
    this.save()
    return result
  }

  // ---- people --------------------------------------------------------

  async createPerson(name: string): Promise<Person> {
    const clean = cleanName(name)
    if (this.data.people.some((p) => p.name === clean)) throw new ConflictError(`${clean} already exists`)
    const player = this.data.people.reduce((m, p) => Math.max(m, p.player), 0) + 1
    const person: Person = { id: newId(), name: clean, player }
    this.data.people.push(person)
    this.save()
    return clone(person)
  }

  async renamePerson(id: string, name: string): Promise<Person> {
    const person = this.data.people.find((p) => p.id === id)
    if (!person) throw new NotFoundError('person not found')
    person.name = cleanName(name)
    this.save()
    return clone(person)
  }

  // ---- cards ---------------------------------------------------------

  async updateCard(id: string, patch: CardPatch): Promise<Card> {
    const card = this.stored(id)
    const next: Stored = clone(card)
    if (patch.name !== undefined) next.name = cleanName(patch.name)
    if (patch.suit !== undefined) next.suit = cleanSuit(patch.suit)
    if (patch.conception !== undefined) next.conception = cleanText(patch.conception)
    if (patch.planning !== undefined) next.planning = cleanText(patch.planning)
    if (patch.execution !== undefined) next.execution = cleanText(patch.execution)
    if (patch.minimumStandardOfCare !== undefined) next.minimumStandardOfCare = cleanStandards(patch.minimumStandardOfCare)
    if (patch.notes !== undefined) next.notes = cleanText(patch.notes)
    if (patch.ownerId !== undefined) {
      this.checkOwner(patch.ownerId)
      next.ownerId = patch.ownerId
    }
    if (patch.parentCardId !== undefined) next.parentCardId = patch.parentCardId
    if (patch.position !== undefined) next.position = patch.position
    this.checkParent(next)
    Object.assign(card, next)
    this.save()
    return this.view(card)
  }

  async createCard(input: NewCard): Promise<Card> {
    this.checkOwner(input.ownerId)
    const card: Stored = {
      id: input.id ?? newId(),
      number: null,
      name: cleanName(input.name),
      suit: cleanSuit(input.suit),
      parentCardId: input.parentCardId ?? null,
      position: this.nextPosition(input.parentCardId ?? null),
      ownerId: input.ownerId ?? null,
      conception: cleanText(input.conception ?? ''),
      planning: cleanText(input.planning ?? ''),
      execution: cleanText(input.execution ?? ''),
      minimumStandardOfCare: cleanStandards(input.minimumStandardOfCare ?? []),
      notes: cleanText(input.notes ?? ''),
      origin: 'family',
      baselineId: null,
    }
    if (this.data.cards.some((c) => c.id === card.id)) throw new ConflictError('a card with that id already exists')
    this.checkParent(card)
    this.data.cards.push(card)
    this.save()
    return this.view(card)
  }

  async split(id: string, input: SplitInput): Promise<SplitResult> {
    if (input.children.length === 0) throw invalid('a split needs at least one child')
    const parent = this.stored(id)
    // Validate everything before writing anything, as the server does.
    const specs: Stored[] = []
    let position = this.nextPosition(id)
    for (const child of input.children) {
      this.checkOwner(child.ownerId)
      specs.push({
        id: child.id ?? newId(),
        number: null,
        name: cleanName(child.name),
        suit: parent.suit,
        parentCardId: id,
        position: position++,
        ownerId: child.ownerId ?? null,
        conception: cleanText(child.conception ?? ''),
        planning: cleanText(child.planning ?? ''),
        execution: cleanText(child.execution ?? ''),
        minimumStandardOfCare: cleanStandards(child.minimumStandardOfCare ?? []),
        notes: cleanText(child.notes ?? ''),
        origin: 'family',
        baselineId: null,
      })
    }
    if (input.ownerId !== undefined) this.checkOwner(input.ownerId)
    const notes = input.notes !== undefined ? cleanText(input.notes) : undefined
    for (const spec of specs) {
      if (this.data.cards.some((c) => c.id === spec.id)) throw new ConflictError('a card with that id already exists')
      this.data.cards.push(spec)
    }
    if (input.ownerId !== undefined) parent.ownerId = input.ownerId
    if (notes !== undefined) parent.notes = notes
    this.save()
    return { parent: this.view(parent), children: specs.map((c) => this.view(c)) }
  }

  async reset(id: string): Promise<Card> {
    const card = this.stored(id)
    const baseline = this.baselineOf(card)
    if (!baseline) throw invalid('not a deck card')
    card.name = baseline.name
    card.suit = baseline.suit
    card.conception = baseline.conception
    card.planning = baseline.planning
    card.execution = baseline.execution
    card.minimumStandardOfCare = [...baseline.minimumStandardOfCare]
    this.save()
    return this.view(card)
  }

  async setPosition(id: string, position: number): Promise<void> {
    this.stored(id).position = position
    this.save()
  }
}

// ---- rules shared with the server ---------------------------------------

export const deckCardId = (number: number): string => `00000000-0000-4000-8000-1000${String(number).padStart(8, '0')}`

const FIELDS: DiffField[] = ['name', 'suit', 'conception', 'planning', 'execution', 'minimum_standard_of_care']

function text(card: Stored | Baseline, field: DiffField): string {
  switch (field) {
    case 'name':
      return card.name
    case 'suit':
      return card.suit
    case 'conception':
      return card.conception
    case 'planning':
      return card.planning
    case 'execution':
      return card.execution
    case 'minimum_standard_of_care':
      return card.minimumStandardOfCare.join('|')
  }
}

/** The fields on which `card` differs from `baseline`, in the server's order. */
export function diff(card: Stored, baseline: Baseline): FieldDiff[] {
  return FIELDS.flatMap((field) => {
    const c = text(card, field)
    const b = text(baseline, field)
    return c === b ? [] : [{ field, card: c, baseline: b }]
  })
}

function cleanName(name: string): string {
  const clean = name.trim()
  if (!clean) throw invalid('name must not be blank')
  if ([...clean].length > 200) throw invalid('name is too long')
  return clean
}

function cleanText(value: string): string {
  if ([...value].length > MAX_TEXT_LEN) throw invalid(`text is longer than ${MAX_TEXT_LEN} characters`)
  return value.trim()
}

function cleanStandards(list: string[]): string[] {
  if (list.length > MAX_STANDARDS) throw invalid(`at most ${MAX_STANDARDS} standards`)
  return list.map(cleanText).filter((s) => s !== '')
}

function cleanSuit(suit: string): Suit {
  if (!(SUITS as readonly string[]).includes(suit)) throw invalid('suit must be one of Home, Out, Caregiving, Magic, Wild, Unicorn Space')
  return suit as Suit
}

const clone = <T>(x: T): T => JSON.parse(JSON.stringify(x)) as T

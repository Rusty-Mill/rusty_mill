/**
 * An in-browser backend with the same rules as `rusty_fair_play`'s service
 * (ADR-0137): state derived against a kept baseline, splits with positions,
 * reset, unknown-owner and cycle refusals, 409 on a duplicate name.
 * Used for the demo (`?adapter=memory`) and the unit tests; the contract
 * suite holds it to the real server's behaviour.
 */
import type { ApiClient } from './client'
import { DEMO_DECK } from './demoDeck'
import { ConflictError, InvalidError, NotFoundError, StaleError } from './errors'
import { SUITS, type Baseline, type BaselineResponse, type Card, type CardPatch, type CardState, type DiffField, type FieldDiff, type NewCard, type Person, type SeedResult, type Snapshot, type SplitInput, type SplitResult, type Suit, type UnsplitResult } from './types'
import { newId } from '@/lib/id'

const MAX_TEXT_LEN = 10_000
const MAX_STANDARDS = 50

/** A card as stored: everything but the derived `state`, `etag` and `treeEtag`. */
type Stored = Omit<Card, 'state' | 'etag' | 'treeEtag'>

interface Data {
  people: Person[]
  baselines: Baseline[]
  cards: Stored[]
}

export interface MemoryOptions {
  /** Persist across reloads (the demo); omit for tests. */
  storage?: Storage | null
  key?: string
  /** The baselines `seed()` loads. Defaults to the full deck in `demoDeck.ts`. */
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
      if (raw) {
        const data = JSON.parse(raw) as Data
        // Older saves predate deck membership: every card was in the deck.
        this.data = { ...data, cards: data.cards.map((c) => ({ ...c, inPlay: c.inPlay ?? true })) }
      }
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
    return { ...clone(card), state, etag: etagOf(card), treeEtag: this.treeEtagOf(card) }
  }

  /** Over the card and its whole subtree in tree order, as the server's. */
  private treeEtagOf(card: Stored): string {
    const tags: string[] = []
    const walk = (c: Stored): void => {
      tags.push(etagOf(c))
      for (const child of this.childrenOf(c.id)) walk(child)
    }
    walk(card)
    return fnv(tags.join(''))
  }

  /** The server's `If-Match`: absent or `*` always passes; anything else must be the card's tag now (its tree tag for `tree`). */
  private check(card: Stored, etag: string | undefined, tree = false): void {
    if (etag === undefined) return
    const tag = etag.trim()
    const current = tree ? this.treeEtagOf(card) : etagOf(card)
    if (tag === '*' || tag.replace(/^W\//, '').replace(/^"|"$/g, '') === current) return
    throw new StaleError(this.view(card))
  }

  private checkOwner(owner: string | null | undefined): void {
    if (owner && !this.data.people.some((p) => p.id === owner)) throw invalid('unknown owner')
  }

  private checkParent(card: Stored): void {
    if (!card.parentCardId) return
    if (card.parentCardId === card.id) throw invalid('a card cannot be its own parent')
    const parent = this.data.cards.find((c) => c.id === card.parentCardId)
    if (!parent) throw invalid('parent card not found')
    if (!parent.inPlay) throw invalid('a card that is set aside cannot have cards under it; add it back to the deck first')
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

  private childrenOf(parentId: string): Stored[] {
    return this.data.cards.filter((c) => c.parentCardId === parentId).sort((a, b) => a.position - b.position || a.id.localeCompare(b.id))
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
          inPlay: true,
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

  async deletePerson(id: string): Promise<void> {
    const at = this.data.people.findIndex((p) => p.id === id)
    if (at < 0) throw new NotFoundError('person not found')
    const held = this.data.cards.filter((c) => c.ownerId === id).length
    if (held > 0) throw new ConflictError(`holds ${held} ${held === 1 ? 'card' : 'cards'}`)
    this.data.people.splice(at, 1)
    this.save()
  }

  // ---- cards ---------------------------------------------------------

  async updateCard(id: string, patch: CardPatch, etag?: string): Promise<Card> {
    const card = this.stored(id)
    this.check(card, etag)
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
      if (patch.ownerId && patch.inPlay === false) throw invalid('a card that is set aside cannot be dealt')
      if (patch.ownerId && patch.inPlay !== true && !card.inPlay) throw invalid('a card that is set aside cannot be dealt; add it back to the deck first')
      next.ownerId = patch.ownerId
    }
    if (patch.parentCardId !== undefined) next.parentCardId = patch.parentCardId
    if (patch.position !== undefined) next.position = patch.position
    if (patch.inPlay === false) {
      if (this.data.cards.some((c) => c.parentCardId === id)) throw new ConflictError('the card is split; unsplit it before setting it aside')
      next.ownerId = null // taken back from its owner in the same write
      next.inPlay = false
    } else if (patch.inPlay === true) next.inPlay = true
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
      inPlay: true,
    }
    if (this.data.cards.some((c) => c.id === card.id)) throw new ConflictError('a card with that id already exists')
    this.checkParent(card)
    this.data.cards.push(card)
    this.save()
    return this.view(card)
  }

  async split(id: string, input: SplitInput, etag?: string): Promise<SplitResult> {
    const parent = this.stored(id)
    this.check(parent, etag)
    if (!parent.inPlay) throw invalid('a card that is set aside cannot be split; add it back to the deck first')
    if (input.children.length === 0) throw invalid('a split needs at least one child')
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
        inPlay: true,
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

  async reset(id: string, etag?: string): Promise<Card> {
    const card = this.stored(id)
    this.check(card, etag)
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

  async setPosition(id: string, position: number, etag?: string): Promise<void> {
    const card = this.stored(id)
    this.check(card, etag)
    card.position = position
    this.save()
  }

  async deleteCard(id: string, etag?: string): Promise<void> {
    const card = this.stored(id)
    this.check(card, etag)
    if (this.data.cards.some((c) => c.parentCardId === id)) throw new ConflictError('the card has children; unsplit it first')
    this.data.cards = this.data.cards.filter((c) => c.id !== id)
    this.save()
  }

  async unsplitCard(id: string, treeEtag?: string): Promise<UnsplitResult> {
    const card = this.stored(id)
    this.check(card, treeEtag, true)
    const deleted: string[] = []
    const walk = (parentId: string): void => {
      for (const child of this.childrenOf(parentId)) {
        walk(child.id)
        deleted.push(child.id)
      }
    }
    walk(id)
    const gone = new Set(deleted)
    this.data.cards = this.data.cards.filter((c) => !gone.has(c.id))
    this.save()
    return { parent: this.view(card), deleted }
  }

  async reorderChildren(parentId: string, ids: string[], treeEtag?: string): Promise<Card[]> {
    const parent = this.stored(parentId)
    this.check(parent, treeEtag, true)
    const children = this.childrenOf(parentId)
    const want = new Set(ids)
    if (want.size !== ids.length || children.length !== ids.length || !children.every((c) => want.has(c.id))) throw invalid('ids must be exactly the current children, each once')
    const byId = new Map(children.map((c) => [c.id, c]))
    const ordered = ids.map((cid, position) => Object.assign(byId.get(cid)!, { position }))
    this.save()
    return ordered.map((c) => this.view(c))
  }
}

// ---- rules shared with the server ---------------------------------------

/** FNV-1a over the stored record: 16 hex chars, like the server's SHA-256 prefix; new whenever the card changes. */
function etagOf(card: Stored): string {
  return fnv(JSON.stringify(card))
}

function fnv(text: string): string {
  let h = 0xcbf29ce484222325n
  for (const byte of new TextEncoder().encode(text)) h = ((h ^ BigInt(byte)) * 0x100000001b3n) & 0xffffffffffffffffn
  return h.toString(16).padStart(16, '0')
}

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

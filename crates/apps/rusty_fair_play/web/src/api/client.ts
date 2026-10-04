import type { BaselineResponse, Card, CardPatch, NewCard, Person, SeedResult, Snapshot, SplitInput, SplitResult, UnsplitResult } from './types'

/**
 * What the UI needs from a backend. `HttpAdapter` talks to `rusty_fair_play`;
 * `MemoryAdapter` implements the same rules in the browser so the UI runs
 * standalone (`?adapter=memory`) and in tests. `contract.ts` holds the shared
 * behaviour tests that both must pass.
 *
 * Every card write takes the version it was based on as `If-Match`: the card's
 * `etag` for a write to the card, its `treeEtag` for the two writes that act on
 * the children too (`unsplitCard`, `reorderChildren`). When given and the card
 * has changed since, the write is refused with a `StaleError` carrying the
 * current card. Omit it (or pass `*`) to write regardless.
 */
export interface ApiClient {
  /** Everything needed to boot: people and every card with its state. */
  snapshot(): Promise<Snapshot>
  /** Load the deck that ships with the app; idempotent. */
  seed(): Promise<SeedResult>

  createPerson(name: string): Promise<Person>
  renamePerson(id: string, name: string): Promise<Person>
  /** 409 while the person still holds a card. */
  deletePerson(id: string): Promise<void>

  getCard(id: string): Promise<Card>
  updateCard(id: string, patch: CardPatch, etag?: string): Promise<Card>
  createCard(input: NewCard): Promise<Card>
  split(id: string, input: SplitInput, etag?: string): Promise<SplitResult>
  /** The six text fields back to the baseline; owner, parent, position and notes kept. */
  reset(id: string, etag?: string): Promise<Card>
  baseline(id: string): Promise<BaselineResponse>
  setPosition(id: string, position: number, etag?: string): Promise<void>
  /** A leaf card only: 409 while it has children. */
  deleteCard(id: string, etag?: string): Promise<void>
  /** Delete everything under the card (deepest first); the card stays. Guarded by `treeEtag`. */
  unsplitCard(id: string, treeEtag?: string): Promise<UnsplitResult>
  /**
   * `ids` must be exactly the current children (422 otherwise); positions follow the list.
   * One request, validated before anything is written; the server then writes the slots one
   * by one, so a crash mid-request can leave some positions applied (the order falls back to id).
   * Guarded by `treeEtag`.
   */
  reorderChildren(parentId: string, ids: string[], treeEtag?: string): Promise<Card[]>
}

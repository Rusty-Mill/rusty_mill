import type { BaselineResponse, Card, CardPatch, NewCard, Person, SeedResult, Snapshot, SplitInput, SplitResult } from './types'

/**
 * What the UI needs from a backend. `HttpAdapter` talks to `rusty_fair_play`;
 * `MemoryAdapter` implements the same rules in the browser so the UI runs
 * standalone (`?adapter=memory`) and in tests. `contract.ts` holds the shared
 * behaviour tests that both must pass.
 */
export interface ApiClient {
  /** Everything needed to boot: people and every card with its state. */
  snapshot(): Promise<Snapshot>
  /** Load the deck that ships with the app; idempotent. */
  seed(): Promise<SeedResult>

  createPerson(name: string): Promise<Person>
  renamePerson(id: string, name: string): Promise<Person>

  getCard(id: string): Promise<Card>
  updateCard(id: string, patch: CardPatch): Promise<Card>
  createCard(input: NewCard): Promise<Card>
  split(id: string, input: SplitInput): Promise<SplitResult>
  /** The six text fields back to the baseline; owner, parent, position and notes kept. */
  reset(id: string): Promise<Card>
  baseline(id: string): Promise<BaselineResponse>
  setPosition(id: string, position: number): Promise<void>
}

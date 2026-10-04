import { describe, expect, it } from 'vitest'
import { baselineId, DEMO_DECK, parseDeck } from './demoDeck'

describe('the demo deck', () => {
  it('is the whole deck from the CSV: 100 cards, the six suit counts, deck ids', () => {
    expect(DEMO_DECK).toHaveLength(100)
    expect(DEMO_DECK.map((b) => b.number)).toEqual(Array.from({ length: 100 }, (_, i) => i + 1))
    const counts = Object.fromEntries(DEMO_DECK.map((b) => [b.suit, DEMO_DECK.filter((x) => x.suit === b.suit).length]))
    expect(counts).toEqual({ Home: 22, Out: 22, Caregiving: 22, Magic: 22, Wild: 10, 'Unicorn Space': 2 })
    expect(DEMO_DECK[1]).toMatchObject({ id: '00000000-0000-4000-8000-000000000002', name: 'Cleaning', suit: 'Home' })
    expect(DEMO_DECK[0]!.minimumStandardOfCare).toHaveLength(3)
    expect(DEMO_DECK[0]!.conception).toContain('sitters, daycare, relatives')
    expect(baselineId(100)).toBe('00000000-0000-4000-8000-000000000100')
  })

  it('refuses a row with an unknown suit or number', () => {
    expect(() => parseDeck('number,name,suit,conception,planning,execution,minimum_standard_of_care,notes\n1,x,Kitchen,,,,,')).toThrow(/bad deck row/)
    expect(() => parseDeck('number,name\n1,x')).toThrow(/lacks a suit/)
  })
})

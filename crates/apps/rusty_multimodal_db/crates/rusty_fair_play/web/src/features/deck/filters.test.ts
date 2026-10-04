import { describe, expect, it } from 'vitest'
import { MemoryAdapter } from '@/api/memory'
import { indexCards } from '@/store/derive'
import { emptyFilters } from '@/store/ui'
import { applyFilters, isFiltering, UNASSIGNED } from './filters'

async function fixture() {
  const api = new MemoryAdapter()
  await api.seed()
  const ada = await api.createPerson('Ada')
  const cards = (await api.snapshot()).cards
  await api.updateCard(cards[0]!.id, { ownerId: ada.id, execution: 'x' })
  await api.split(cards[1]!.id, { children: [{ name: 'Floors' }] })
  const snap = await api.snapshot()
  return { cards: snap.cards, index: indexCards(snap.cards), ada }
}

describe('board filters', () => {
  it('shows everything by default', async () => {
    const { cards, index } = await fixture()
    expect(applyFilters(cards, index, emptyFilters)).toHaveLength(13)
    expect(isFiltering(emptyFilters)).toBe(false)
  })

  it('filters by suit, owner, state, leaves and search, combined', async () => {
    const { cards, index, ada } = await fixture()
    expect(applyFilters(cards, index, { ...emptyFilters, suits: ['Home'] })).toHaveLength(4)
    expect(applyFilters(cards, index, { ...emptyFilters, suits: ['Wild', 'Unicorn Space'] })).toHaveLength(3)
    expect(applyFilters(cards, index, { ...emptyFilters, owners: [ada.id] }).map((c) => c.number)).toEqual([1])
    expect(applyFilters(cards, index, { ...emptyFilters, owners: [UNASSIGNED] })).toHaveLength(12)
    expect(applyFilters(cards, index, { ...emptyFilters, state: 'edited' }).map((c) => c.number)).toEqual([1])
    expect(applyFilters(cards, index, { ...emptyFilters, state: 'custom' }).map((c) => c.name)).toEqual(['Floors'])
    expect(applyFilters(cards, index, { ...emptyFilters, state: 'original' })).toHaveLength(11)
    expect(applyFilters(cards, index, { ...emptyFilters, leavesOnly: true })).toHaveLength(12) // Cleaning is a split parent
    expect(applyFilters(cards, index, { ...emptyFilters, search: '  FLOOR ' }).map((c) => c.name)).toEqual(['Floors'])
    expect(applyFilters(cards, index, { ...emptyFilters, suits: ['Home'], state: 'original', leavesOnly: true }).map((c) => c.name)).toEqual(['Dishes'])
    expect(isFiltering({ ...emptyFilters, search: ' ' })).toBe(false)
    expect(isFiltering({ ...emptyFilters, leavesOnly: true })).toBe(true)
  })
})

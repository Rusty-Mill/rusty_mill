import { describe, expect, it } from 'vitest'
import { MemoryAdapter } from '@/api/memory'
import { balance, bySuit, chainToRoot, childrenOf, indexCards, isLeaf, stateCounts, subtree, undealt } from './derive'

/** The deck, Ada and Bob, Cleaning split into Floors (Bob) and Bathrooms (Ada), Floors split into Mopping (unowned). */
async function fixture() {
  const api = new MemoryAdapter()
  await api.seed()
  const ada = await api.createPerson('Ada')
  const bob = await api.createPerson('Bob')
  const cleaning = (await api.snapshot()).cards.find((c) => c.number === 2)!
  const dishes = (await api.snapshot()).cards.find((c) => c.number === 3)!
  await api.updateCard(cleaning.id, { ownerId: ada.id })
  await api.updateCard(dishes.id, { ownerId: ada.id, execution: 'changed' })
  const { children } = await api.split(cleaning.id, { children: [{ name: 'Floors', ownerId: bob.id }, { name: 'Bathrooms', ownerId: ada.id }] })
  const floors = children[0]!
  const { children: grand } = await api.split(floors.id, { children: [{ name: 'Mopping' }] })
  const snap = await api.snapshot()
  return { snap, index: indexCards(snap.cards), ada, bob, cleaning, dishes, floors, bathrooms: children[1]!, mopping: grand[0]! }
}

describe('derived views', () => {
  it('indexes children in position order and tells leaves from split parents', async () => {
    const { index, cleaning, floors, bathrooms, mopping } = await fixture()
    expect(childrenOf(index, cleaning.id).map((c) => c.name)).toEqual(['Floors', 'Bathrooms'])
    expect(isLeaf(index, cleaning.id)).toBe(false)
    expect(isLeaf(index, floors.id)).toBe(false)
    expect(isLeaf(index, bathrooms.id)).toBe(true)
    expect(isLeaf(index, mopping.id)).toBe(true)
    expect(index.roots).toHaveLength(100)
    expect(index.leafIds.size).toBe(100 - 2 + 3) // 100 roots minus Cleaning and Floors, plus Floors' and Cleaning's leaves
  })

  it('walks the chain to the root and the subtree down', async () => {
    const { index, cleaning, floors, mopping } = await fixture()
    expect(chainToRoot(index, mopping.id).map((c) => c.name)).toEqual(['Cleaning', 'Floors', 'Mopping'])
    expect(chainToRoot(index, cleaning.id).map((c) => c.name)).toEqual(['Cleaning'])
    expect(subtree(index, cleaning.id).map((c) => c.name)).toEqual(['Cleaning', 'Floors', 'Mopping', 'Bathrooms'])
    expect(subtree(index, floors.id)).toHaveLength(2)
  })

  it('stops on a cycle instead of looping', () => {
    const base = { number: null, suit: 'Home' as const, position: 0, ownerId: null, conception: '', planning: '', execution: '', minimumStandardOfCare: [], notes: '', origin: 'family' as const, baselineId: null, state: 'custom' as const, etag: '0', treeEtag: '0' }
    const index = indexCards([
      { ...base, id: 'a', name: 'A', parentCardId: 'b' },
      { ...base, id: 'b', name: 'B', parentCardId: 'a' },
    ])
    expect(chainToRoot(index, 'a').map((c) => c.id)).toEqual(['b', 'a'])
    expect(subtree(index, 'a').map((c) => c.id)).toEqual(['a', 'b'])
  })

  it('counts balance in both variants: all cards, and leaves only', async () => {
    const { snap, index } = await fixture()
    const rows = balance(index, snap.cards, snap.people)
    const ada = rows.find((r) => r.person.name === 'Ada')!
    const bob = rows.find((r) => r.person.name === 'Bob')!
    expect([ada.all, ada.leaves]).toEqual([3, 2]) // Cleaning (split), Dishes, Bathrooms
    expect([bob.all, bob.leaves]).toEqual([1, 0]) // Floors, itself split
    expect(ada.bySuit.Home).toEqual({ all: 3, leaves: 2 })
    expect(ada.bySuit.Out).toEqual({ all: 0, leaves: 0 })
    expect(bob.bySuit.Home).toEqual({ all: 1, leaves: 0 })
  })

  it('lists the undealt as unowned leaves only', async () => {
    const { snap, index } = await fixture()
    const left = undealt(index, snap.cards)
    expect(left).toHaveLength(98 + 1) // the untouched deck cards plus Mopping; neither split parent
    expect(left.some((c) => c.name === 'Mopping')).toBe(true)
    expect(left.some((c) => c.name === 'Cleaning' || c.name === 'Floors')).toBe(false)
    expect(bySuit(left).Home).toHaveLength(21)
    expect(bySuit(left).Home.map((c) => c.name)).toEqual(expect.arrayContaining(['Childcare Helpers (Kids)', 'Mopping']))
  })

  it('counts states', async () => {
    const { snap } = await fixture()
    expect(stateCounts(snap.cards)).toEqual({ original: 99, edited: 1, custom: 3 })
  })
})

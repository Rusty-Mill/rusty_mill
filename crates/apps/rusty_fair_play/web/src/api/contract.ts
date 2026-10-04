/**
 * Behaviour every `ApiClient` must have. Run against `MemoryAdapter` in the
 * unit tests and against `HttpAdapter` + the real `rusty_fair_play` binary in
 * `npm run test:integration`, so the in-browser fallback cannot drift from the
 * server it stands in for.
 */
import { describe, expect, it } from 'vitest'
import type { ApiClient } from './client'
import { ConflictError, InvalidError, NotFoundError, StaleError } from './errors'
import type { Card } from './types'

export function runApiContract(label: string, make: () => Promise<ApiClient>): void {
  describe(`ApiClient contract: ${label}`, () => {
    /** A seeded backend, two people, and deck card 2 (Cleaning, Home). */
    const setup = async () => {
      const api = await make()
      await api.seed()
      const ada = await api.createPerson('Ada')
      const bob = await api.createPerson('Bob')
      const snap = await api.snapshot()
      const cleaning = snap.cards.find((c) => c.number === 2)
      if (!cleaning) throw new Error('deck card 2 missing')
      return { api, ada, bob, cleaning }
    }
    const card = async (api: ApiClient, id: string): Promise<Card> => api.getCard(id)

    it('seeds the deck once: original deck cards, numbered and unowned', async () => {
      const api = await make()
      const first = await api.seed()
      const snap = await api.snapshot()
      expect(snap.cards).toHaveLength(100)
      expect(first.cards.created + first.cards.existing).toBe(snap.cards.length)
      for (const c of snap.cards) {
        expect(c).toMatchObject({ origin: 'deck', state: 'original', ownerId: null, parentCardId: null })
        expect(c.number).toBeGreaterThanOrEqual(1)
        expect(c.baselineId).not.toBeNull()
        expect(c.etag).toMatch(/^[0-9a-f]{16}$/)
      }
      expect(snap.cards.map((c) => c.number)).toEqual([...snap.cards.map((c) => c.number)].sort((a, b) => a! - b!))
      const again = await api.seed()
      expect(again.cards.created).toBe(0)
      expect(again.cards.existing).toBe(snap.cards.length)
    })

    it('adds people in player order, trims names, and refuses blanks and duplicates', async () => {
      const api = await make()
      const ada = await api.createPerson(' Ada ')
      const bob = await api.createPerson('Bob')
      expect([ada.name, ada.player, bob.player]).toEqual(['Ada', 1, 2])
      await expect(api.createPerson('Ada')).rejects.toBeInstanceOf(ConflictError)
      await expect(api.createPerson('   ')).rejects.toBeInstanceOf(InvalidError)
      const renamed = await api.renamePerson(bob.id, ' Robert ')
      expect(renamed).toMatchObject({ id: bob.id, name: 'Robert', player: 2 })
      await expect(api.renamePerson(bob.id, '')).rejects.toBeInstanceOf(InvalidError)
      expect((await api.snapshot()).people.map((p) => p.name)).toEqual(['Ada', 'Robert'])
    })

    it('deals a card without editing it, and refuses an unknown owner', async () => {
      const { api, ada, cleaning } = await setup()
      const dealt = await api.updateCard(cleaning.id, { ownerId: ada.id })
      expect(dealt).toMatchObject({ ownerId: ada.id, state: 'original' })
      await expect(api.updateCard(cleaning.id, { ownerId: '00000000-0000-4000-8000-00000000dead' })).rejects.toBeInstanceOf(InvalidError)
      const back = await api.updateCard(cleaning.id, { ownerId: null })
      expect(back.ownerId).toBeNull()
      const noted = await api.updateCard(cleaning.id, { notes: ' kept as is ' })
      expect(noted).toMatchObject({ notes: 'kept as is', state: 'original' })
    })

    it('marks a deck card edited by its six fields, shows the diff, and resets it', async () => {
      const { api, ada, cleaning } = await setup()
      await api.updateCard(cleaning.id, { ownerId: ada.id, notes: 'ours' })
      const edited = await api.updateCard(cleaning.id, { execution: '  our way  ' })
      expect(edited).toMatchObject({ execution: 'our way', state: 'edited' })
      const { baseline, diff } = await api.baseline(cleaning.id)
      expect(baseline).toMatchObject({ number: 2, name: cleaning.name, suit: 'Home' })
      expect(diff).toEqual([{ field: 'execution', card: 'our way', baseline: cleaning.execution }])
      const more = await api.updateCard(cleaning.id, { name: 'Tidying', minimumStandardOfCare: ['weekly', '  ', 'daily'] })
      expect(more.minimumStandardOfCare).toEqual(['weekly', 'daily'])
      expect((await api.baseline(cleaning.id)).diff.map((d) => d.field)).toEqual(['name', 'execution', 'minimum_standard_of_care'])
      await expect(api.updateCard(cleaning.id, { name: '  ' })).rejects.toBeInstanceOf(InvalidError)
      await expect(api.updateCard(cleaning.id, { suit: 'Kitchen' as never })).rejects.toBeInstanceOf(InvalidError)
      const reset = await api.reset(cleaning.id)
      expect(reset).toMatchObject({ name: cleaning.name, execution: cleaning.execution, minimumStandardOfCare: cleaning.minimumStandardOfCare, state: 'original', ownerId: ada.id, notes: 'ours' })
      expect((await api.baseline(cleaning.id)).diff).toEqual([])
    })

    it('splits a card into custom children of the same suit, in order', async () => {
      const { api, ada, bob, cleaning } = await setup()
      await api.updateCard(cleaning.id, { ownerId: ada.id })
      const kept = await api.split(cleaning.id, { children: [{ name: 'Bathrooms', ownerId: ada.id }] })
      expect(kept.parent.ownerId).toBe(ada.id) // absent ownerId: the parent keeps its owner
      const split = await api.split(cleaning.id, { children: [{ name: 'Floors', ownerId: bob.id }, { name: ' Dusting ' }], ownerId: null, notes: 'split three ways' })
      expect(split.parent).toMatchObject({ id: cleaning.id, ownerId: null, notes: 'split three ways', state: 'original' })
      expect(split.children.map((c) => [c.name, c.ownerId, c.suit, c.parentCardId, c.origin, c.state, c.number])).toEqual([
        ['Floors', bob.id, 'Home', cleaning.id, 'family', 'custom', null],
        ['Dusting', null, 'Home', cleaning.id, 'family', 'custom', null],
      ])
      const children = (await api.snapshot()).cards.filter((c) => c.parentCardId === cleaning.id).sort((a, b) => a.position - b.position)
      expect(children.map((c) => c.name)).toEqual(['Bathrooms', 'Floors', 'Dusting'])
      expect(children.map((c) => c.position)).toEqual([children[0]!.position, children[0]!.position + 1, children[0]!.position + 2])
      await expect(api.split(cleaning.id, { children: [] })).rejects.toBeInstanceOf(InvalidError)
      await expect(api.split(cleaning.id, { children: [{ name: 'x', ownerId: '00000000-0000-4000-8000-00000000dead' }] })).rejects.toBeInstanceOf(InvalidError)
      await expect(api.split(cleaning.id, { children: [{ name: '  ' }] })).rejects.toBeInstanceOf(InvalidError)
      await expect(api.split('00000000-0000-4000-8000-00000000dead', { children: [{ name: 'x' }] })).rejects.toBeInstanceOf(NotFoundError)
      // A split child cannot be reset: it has no baseline.
      await expect(api.reset(split.children[0]!.id)).rejects.toBeInstanceOf(InvalidError)
      expect(await api.baseline(split.children[0]!.id)).toEqual({ baseline: null, diff: [] })
    })

    it('creates custom cards and refuses cycles and unknown parents', async () => {
      const { api, bob, cleaning } = await setup()
      const dog = await api.createCard({ name: 'Dog walking', suit: 'Out', ownerId: bob.id, minimumStandardOfCare: ['twice a day', ' '] })
      expect(dog).toMatchObject({ state: 'custom', origin: 'family', number: null, baselineId: null, suit: 'Out', minimumStandardOfCare: ['twice a day'] })
      await expect(api.createCard({ name: 'x', suit: 'Out', ownerId: '00000000-0000-4000-8000-00000000dead' })).rejects.toBeInstanceOf(InvalidError)
      await expect(api.createCard({ name: 'x', suit: 'Out', parentCardId: '00000000-0000-4000-8000-00000000dead' })).rejects.toBeInstanceOf(InvalidError)
      const { children } = await api.split(cleaning.id, { children: [{ name: 'Floors' }] })
      const floors = children[0]!
      const { children: grand } = await api.split(floors.id, { children: [{ name: 'Mopping' }] })
      const mopping = grand[0]!
      await expect(api.updateCard(cleaning.id, { parentCardId: mopping.id })).rejects.toBeInstanceOf(InvalidError)
      await expect(api.updateCard(floors.id, { parentCardId: floors.id })).rejects.toBeInstanceOf(InvalidError)
      expect((await card(api, cleaning.id)).parentCardId).toBeNull()
      const moved = await api.updateCard(mopping.id, { parentCardId: dog.id })
      expect(moved.parentCardId).toBe(dog.id)
    })

    it('moves a card among its siblings with one position write', async () => {
      const { api, cleaning } = await setup()
      const { children } = await api.split(cleaning.id, { children: [{ name: 'A' }, { name: 'B' }] })
      const [a, b] = children as [Card, Card]
      await api.setPosition(a.id, b.position)
      await api.setPosition(b.id, a.position)
      const now = (await api.snapshot()).cards.filter((c) => c.parentCardId === cleaning.id).sort((x, y) => x.position - y.position)
      expect(now.map((c) => c.name)).toEqual(['B', 'A'])
      expect((await card(api, a.id)).state).toBe('custom')
      await expect(api.setPosition('00000000-0000-4000-8000-00000000dead', 1)).rejects.toBeInstanceOf(NotFoundError)
      await expect(card(api, '00000000-0000-4000-8000-00000000dead')).rejects.toBeInstanceOf(NotFoundError)
    })

    it('reorders children in one call, and refuses anything but the exact set', async () => {
      const { api, cleaning } = await setup()
      const { parent, children } = await api.split(cleaning.id, { children: [{ name: 'A' }, { name: 'B' }, { name: 'C' }] })
      const [a, b, c] = children.map((x) => x.id) as [string, string, string]
      const back = await api.reorderChildren(cleaning.id, [c, a, b], (await card(api, parent.id)).treeEtag)
      expect(back.map((x) => [x.id, x.position])).toEqual([
        [c, 0],
        [a, 1],
        [b, 2],
      ])
      expect(back[0]!.etag).not.toBe(children[2]!.etag) // a moved child is a changed card
      const now = (await api.snapshot()).cards.filter((x) => x.parentCardId === cleaning.id).sort((x, y) => x.position - y.position)
      expect(now.map((x) => x.name)).toEqual(['C', 'A', 'B'])
      await expect(api.reorderChildren(cleaning.id, [a])).rejects.toBeInstanceOf(InvalidError)
      await expect(api.reorderChildren(cleaning.id, [a, a, b])).rejects.toBeInstanceOf(InvalidError)
      await expect(api.reorderChildren(cleaning.id, [a, b, cleaning.id])).rejects.toBeInstanceOf(InvalidError)
      await expect(api.reorderChildren('00000000-0000-4000-8000-00000000dead', [a])).rejects.toBeInstanceOf(NotFoundError)
      expect((await api.snapshot()).cards.filter((x) => x.parentCardId === cleaning.id).map((x) => x.position).sort()).toEqual([0, 1, 2])
    })

    it('deletes a leaf card, refuses a split parent until it is unsplit, and removes a person who holds nothing', async () => {
      const { api, ada, cleaning } = await setup()
      const { children } = await api.split(cleaning.id, { children: [{ name: 'Floors' }, { name: 'Walls' }] })
      const floors = children[0]!
      const { children: grand } = await api.split(floors.id, { children: [{ name: 'Mopping' }] })
      await expect(api.deleteCard(cleaning.id)).rejects.toBeInstanceOf(ConflictError)
      await expect(api.deleteCard('00000000-0000-4000-8000-00000000dead')).rejects.toBeInstanceOf(NotFoundError)
      await api.deleteCard(children[1]!.id)
      await expect(card(api, children[1]!.id)).rejects.toBeInstanceOf(NotFoundError)
      expect(await api.unsplitCard(grand[0]!.id)).toMatchObject({ deleted: [] }) // a leaf: nothing under it
      const { parent, deleted } = await api.unsplitCard(cleaning.id)
      expect(deleted).toEqual([grand[0]!.id, floors.id]) // deepest first
      expect(parent).toMatchObject({ id: cleaning.id, state: 'original' })
      await expect(api.unsplitCard('00000000-0000-4000-8000-00000000dead')).rejects.toBeInstanceOf(NotFoundError)
      expect((await api.snapshot()).cards).toHaveLength(100)
      await api.deleteCard(cleaning.id) // a deck card goes once it is a leaf
      expect((await api.snapshot()).cards).toHaveLength(99)

      const dishes = (await api.snapshot()).cards.find((x) => x.number === 3)!
      await api.updateCard(dishes.id, { ownerId: ada.id })
      await expect(api.deletePerson(ada.id)).rejects.toBeInstanceOf(ConflictError)
      await api.updateCard(dishes.id, { ownerId: null })
      await api.deletePerson(ada.id)
      await expect(api.deletePerson(ada.id)).rejects.toBeInstanceOf(NotFoundError)
      expect((await api.snapshot()).people.map((p) => p.name)).toEqual(['Bob'])
    })

    it('guards the subtree writes with the tree tag, which moves when any descendant does', async () => {
      const { api, cleaning } = await setup()
      const { children } = await api.split(cleaning.id, { children: [{ name: 'A' }, { name: 'B' }] })
      const [a, b] = children.map((x) => x.id) as [string, string]
      const read = await card(api, cleaning.id)
      await api.updateCard(a, { notes: 'edited in another tab' }) // a child, not the card
      const after = await card(api, cleaning.id)
      expect(after.etag).toBe(read.etag)
      expect(after.treeEtag).not.toBe(read.treeEtag)
      const stale = api.reorderChildren(cleaning.id, [b, a], read.treeEtag)
      await expect(stale).rejects.toBeInstanceOf(StaleError)
      expect(((await stale.catch((x: unknown) => x)) as StaleError).current.treeEtag).toBe(after.treeEtag)
      await expect(api.unsplitCard(cleaning.id, read.treeEtag)).rejects.toBeInstanceOf(StaleError)
      expect((await api.snapshot()).cards.filter((x) => x.parentCardId === cleaning.id)).toHaveLength(2) // nothing ran
      // A child added elsewhere moves it too, and the card's own tag does not guard a subtree write.
      await api.reorderChildren(cleaning.id, [b, a], after.treeEtag)
      const moved = await card(api, cleaning.id)
      expect(moved.treeEtag).not.toBe(after.treeEtag)
      await expect(api.unsplitCard(cleaning.id, after.etag)).rejects.toBeInstanceOf(StaleError)
      const { deleted } = await api.unsplitCard(cleaning.id, moved.treeEtag)
      expect(deleted).toHaveLength(2)
    })

    it('refuses a write with a stale etag (412) and hands back the current card', async () => {
      const { api, cleaning } = await setup()
      const stale = api.updateCard(cleaning.id, { notes: 'x' }, '0000000000000000')
      await expect(stale).rejects.toBeInstanceOf(StaleError)
      const e = (await stale.catch((x: unknown) => x)) as StaleError
      expect(e.current).toMatchObject({ id: cleaning.id, etag: cleaning.etag })
      const noted = await api.updateCard(cleaning.id, { notes: 'x' }, cleaning.etag)
      expect(noted.etag).not.toBe(cleaning.etag)
      expect(noted.etag).toBe((await card(api, cleaning.id)).etag)
      await api.updateCard(cleaning.id, { notes: 'y' }, '*') // a wildcard always matches
      const old = cleaning.etag
      await expect(api.split(cleaning.id, { children: [{ name: 'A' }] }, old)).rejects.toBeInstanceOf(StaleError)
      await expect(api.reset(cleaning.id, old)).rejects.toBeInstanceOf(StaleError)
      await expect(api.setPosition(cleaning.id, 5, old)).rejects.toBeInstanceOf(StaleError)
      await expect(api.deleteCard(cleaning.id, old)).rejects.toBeInstanceOf(StaleError)
      await expect(api.unsplitCard(cleaning.id, old)).rejects.toBeInstanceOf(StaleError)
      await expect(api.reorderChildren(cleaning.id, [], old)).rejects.toBeInstanceOf(StaleError)
      const fresh = (await card(api, cleaning.id)).etag
      const { parent } = await api.split(cleaning.id, { children: [{ name: 'A' }] }, fresh)
      expect(parent.etag).toBe(fresh) // the parent itself did not change
      await api.deleteCard((await api.snapshot()).cards.find((x) => x.name === 'A')!.id, undefined)
    })
  })
}

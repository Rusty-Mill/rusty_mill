import { describe, expect, it } from 'vitest'
import type { List, Task } from '@/api/types'
import { startOfDay } from '@/lib/date'
import { buildQuickAdd, defaultListId, type QuickAddContext } from './quickAddParse'

const NOW = new Date(2026, 8, 29, 12).getTime()
const INBOX = 'inbox'
const list = (id: string, name: string, archived = false): List => ({ id, name, color: null, archived, viewMode: 'list', sortType: '', sortOrder: 0, updatedMs: 0, etag: '1' })
const lists = [list(INBOX, 'Inbox'), list('w', 'Work'), list('h', 'Home Reno'), list('old', 'Old', true)]
const cx = (over: Partial<QuickAddContext> = {}): QuickAddContext => ({
  now: NOW, lists, inboxId: INBOX, view: { kind: 'inbox' }, siblings: () => [], defaultReminder: 'TRIGGER:PT0S', ...over,
})
const ok = (text: string, over?: Partial<QuickAddContext>) => {
  const r = buildQuickAdd(text, cx(over))
  if (!r.ok) throw new Error('expected a task')
  return r.input
}

describe('target list', () => {
  it('defaults to the Inbox, or the list being viewed', () => {
    expect(ok('buy milk').listId).toBe(INBOX)
    expect(ok('buy milk', { view: { kind: 'list', id: 'w' } }).listId).toBe('w')
    expect(defaultListId({ kind: 'today' }, INBOX)).toBe(INBOX)
  })
  it('~list overrides it, case-insensitively, quoted or not', () => {
    expect(ok('paint ~work', { view: { kind: 'list', id: 'h' } }).listId).toBe('w')
    expect(ok('paint ~"home reno"').listId).toBe('h')
  })
  it('an unknown or archived ~list stays in the title and files nowhere special', () => {
    const t = ok('call ~nowhere today')
    expect(t.listId).toBe(INBOX)
    expect(t.title).toBe('call ~nowhere')
    expect(ok('call ~old').title).toBe('call ~old')
  })
})

describe('fields from the text', () => {
  it('takes date, priority and tags', () => {
    const t = ok('call mum tomorrow 5pm !high #family')
    expect(t).toMatchObject({ title: 'call mum', priority: 5, tags: ['family'], isAllDay: false })
    expect(t.dueMs).toBe(new Date(2026, 8, 30, 17).getTime())
  })
  it('an all-day date gets no reminder; a timed one gets the default', () => {
    expect(ok('a tomorrow').reminders).toBeUndefined()
    expect(ok('a tomorrow 9am').reminders).toEqual(['TRIGGER:PT0S'])
    expect(ok('a tomorrow 9am', { defaultReminder: '' }).reminders).toBeUndefined()
  })
  it('has no date fields when the text has no date', () => {
    const t = ok('plain')
    expect(t).not.toHaveProperty('dueMs')
    expect(t).not.toHaveProperty('priority')
  })
})

describe('what the current view implies', () => {
  it('Today and Next 7 Days add for today', () => {
    for (const kind of ['today', 'week'] as const) {
      const t = ok('x', { view: { kind } })
      expect(t).toMatchObject({ dueMs: startOfDay(NOW), isAllDay: true })
    }
  })
  it('but an explicit date wins', () => {
    expect(ok('x tomorrow', { view: { kind: 'today' } }).dueMs).toBe(startOfDay(NOW) + 86_400_000)
  })
  it('a tag view adds the tag, without duplicating a typed one', () => {
    expect(ok('x', { view: { kind: 'tag', name: 'family' } }).tags).toEqual(['family'])
    expect(ok('x #Family #b', { view: { kind: 'tag', name: 'family' } }).tags).toEqual(['Family', 'b'])
  })
  it('All and the Inbox add nothing extra', () => {
    expect(ok('x', { view: { kind: 'all' } })).not.toHaveProperty('dueMs')
  })
})

describe('position and rejection', () => {
  it('goes above what is already in the list', () => {
    const existing = [{ sortOrder: 10 }, { sortOrder: 500 }] as Task[]
    expect(ok('x', { siblings: () => existing }).sortOrder).toBe(10 - 1024)
    expect(ok('x').sortOrder).toBe(0)
  })
  it('refuses a line with nothing left for a title', () => {
    expect(buildQuickAdd('tomorrow #a !high', cx())).toEqual({ ok: false, reason: 'empty' })
    expect(buildQuickAdd('   ', cx())).toEqual({ ok: false, reason: 'empty' })
  })
})

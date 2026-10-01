import { describe, expect, it } from 'vitest'
import { calendarPath, parseView, railSection, settingsHref, taskPath, viewPath } from './paths'

const INBOX = '00000000-0000-7000-8000-000000000001'

describe('parseView', () => {
  it('reads smart lists, the Inbox, lists and tags', () => {
    expect(parseView({ smart: 'all' }, INBOX)).toEqual({ kind: 'all' })
    expect(parseView({ smart: 'today' }, INBOX)).toEqual({ kind: 'today' })
    expect(parseView({ smart: 'week' }, INBOX)).toEqual({ kind: 'week' })
    expect(parseView({ listId: 'inbox' }, INBOX)).toEqual({ kind: 'inbox' })
    expect(parseView({ listId: INBOX }, INBOX)).toEqual({ kind: 'inbox' })
    expect(parseView({ listId: 'abc' }, INBOX)).toEqual({ kind: 'list', id: 'abc' })
    expect(parseView({ tag: 'home office' }, INBOX)).toEqual({ kind: 'tag', name: 'home office' })
  })
  it('rejects unknown smart lists so the caller can redirect', () => {
    expect(parseView({ smart: 'someday' }, INBOX)).toBeNull()
    expect(parseView({}, INBOX)).toBeNull()
  })
})

describe('paths', () => {
  it('round-trips through parseView', () => {
    const specs = [{ kind: 'all' }, { kind: 'today' }, { kind: 'week' }, { kind: 'inbox' }, { kind: 'list', id: 'abc' }] as const
    for (const spec of specs) {
      const [, section, id] = viewPath(spec).split('/')
      const params = section === 'q' ? { smart: id } : { listId: id }
      expect(parseView(params, INBOX)).toEqual(spec)
    }
  })
  it('matches the reference URLs', () => {
    expect(viewPath({ kind: 'all' })).toBe('/q/all/tasks')
    expect(viewPath({ kind: 'inbox' })).toBe('/p/inbox/tasks')
    expect(taskPath({ kind: 'today' }, 'abc')).toBe('/q/today/tasks/abc')
    expect(calendarPath()).toBe('/c/all/calendar/m')
    expect(calendarPath('w')).toBe('/c/all/calendar/w')
    expect(settingsHref()).toBe('?modalType=settings&tabs=account')
    expect(settingsHref('date-time')).toBe('?modalType=settings&tabs=date-time')
  })
  it('encodes tag names', () => {
    expect(viewPath({ kind: 'tag', name: 'a/b c' })).toBe('/t/a%2Fb%20c/tasks')
  })
  it('names the rail section for a path', () => {
    expect(railSection('/q/all/tasks')).toBe('tasks')
    expect(railSection('/p/inbox/tasks/x')).toBe('tasks')
    expect(railSection('/c/all/calendar/m')).toBe('calendar')
    expect(railSection('/focus')).toBe('focus')
    expect(railSection('/q/all/habit')).toBe('habit')
    expect(railSection('/q/all/completed')).toBe('tasks')
  })
})

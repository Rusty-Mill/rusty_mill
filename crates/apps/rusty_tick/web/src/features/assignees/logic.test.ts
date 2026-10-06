import { describe, expect, it } from 'vitest'
import { asAssigneeBody, assigneeMap, cleanName, knownNames } from './logic'

describe('assignee names', () => {
  it('cleans whitespace and length', () => {
    expect(cleanName('  Ada   Lovelace ')).toBe('Ada Lovelace')
    expect(cleanName('x'.repeat(100))).toHaveLength(60)
    expect(cleanName('   ')).toBe('')
  })
  it('parses a stored body, or refuses it', () => {
    expect(asAssigneeBody({ taskId: 't', name: ' Bo ' })).toEqual({ taskId: 't', name: 'Bo' })
    expect(asAssigneeBody({ taskId: 't', name: ' ' })).toBeNull()
    expect(asAssigneeBody({ name: 'Bo' })).toBeNull()
    expect(asAssigneeBody(null)).toBeNull()
  })
  it('maps tasks to names and lists each name once', () => {
    const items = [{ taskId: 'a', name: 'Bo' }, { taskId: 'b', name: 'ada' }, { taskId: 'c', name: 'Ada' }]
    expect(assigneeMap(items)).toEqual({ a: 'Bo', b: 'ada', c: 'Ada' })
    expect(knownNames(items)).toEqual(['ada', 'Bo'])
  })
})

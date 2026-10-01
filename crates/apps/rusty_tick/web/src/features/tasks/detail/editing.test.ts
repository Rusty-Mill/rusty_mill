import { describe, expect, it } from 'vitest'
import { applyFormat, applySlash, detectSlash, itemsToNotes, matchCommands, notesToItems, SLASH_COMMANDS } from './editing'

describe('detectSlash', () => {
  it('finds a slash word before the caret, at line start or after a space', () => {
    expect(detectSlash('/he', 3)).toEqual({ start: 0, query: 'he' })
    expect(detectSlash('note\n/', 6)).toEqual({ start: 5, query: '' })
    expect(detectSlash('see /todo', 9)).toEqual({ start: 4, query: 'todo' })
  })
  it('ignores slashes inside words or urls, and text after the caret', () => {
    expect(detectSlash('and/or', 6)).toBeNull()
    expect(detectSlash('https://x.co', 12)).toBeNull()
    expect(detectSlash('/he llo', 7)).toBeNull()
    expect(detectSlash('/head', 2)).toEqual({ start: 0, query: 'h' }) // only what is before the caret counts
  })
})

describe('matchCommands', () => {
  it('lists everything for an empty query and filters by label or id', () => {
    expect(matchCommands('')).toHaveLength(SLASH_COMMANDS.length)
    expect(matchCommands('head').map((c) => c.id)).toEqual(['h1', 'h2', 'h3'])
    expect(matchCommands('todo').map((c) => c.id)).toEqual(['todo'])
    expect(matchCommands('zzz')).toEqual([])
  })
})

describe('applySlash', () => {
  const cmd = (id: string) => SLASH_COMMANDS.find((c) => c.id === id)!
  it('replaces the /query and places the caret', () => {
    expect(applySlash('/he', { start: 0, query: 'he' }, 3, cmd('h2'))).toEqual({ text: '## ', start: 3, end: 3 })
    expect(applySlash('a\n/todo more', { start: 2, query: 'todo' }, 7, cmd('todo'))).toEqual({ text: 'a\n- [ ]  more', start: 8, end: 8 })
  })
  it('puts the caret inside paired markers', () => {
    expect(applySlash('/code', { start: 0, query: 'code' }, 5, cmd('code'))).toEqual({ text: '``', start: 1, end: 1 })
  })
})

describe('applyFormat', () => {
  it('wraps the selection and keeps it selected', () => {
    expect(applyFormat('hello world', 6, 11, 'bold')).toEqual({ text: 'hello **world**', start: 8, end: 13 })
    expect(applyFormat('a', 0, 1, 'code')).toEqual({ text: '`a`', start: 1, end: 2 })
  })
  it('unwraps when already wrapped', () => {
    expect(applyFormat('hello **world**', 8, 13, 'bold')).toEqual({ text: 'hello world', start: 6, end: 11 })
  })
  it('an empty selection inserts a marker pair around the caret', () => {
    expect(applyFormat('ab', 1, 1, 'italic')).toEqual({ text: 'a**b', start: 2, end: 2 })
  })
  it('toggles a line prefix on every selected line', () => {
    expect(applyFormat('a\nb\nc', 0, 3, 'bullet')).toEqual({ text: '- a\n- b\nc', start: 0, end: 7 })
    expect(applyFormat('- a\n- b', 0, 7, 'bullet')).toEqual({ text: 'a\nb', start: 0, end: 3 })
    expect(applyFormat('a\n- b', 0, 5, 'bullet')).toEqual({ text: '- a\n- b', start: 0, end: 7 }) // mixed: fill in the gaps
  })
  it('acts on the caret line when nothing is selected', () => {
    expect(applyFormat('one\ntwo', 5, 5, 'quote')).toEqual({ text: 'one\n> two', start: 4, end: 9 })
    expect(applyFormat('title', 2, 2, 'heading')).toEqual({ text: '## title', start: 0, end: 8 })
  })
})

describe('checklist <-> notes', () => {
  let n = 0
  const id = () => `id${++n}`
  it('turns lines into items, dropping markers and reading boxes', () => {
    const items = notesToItems('- [x] done\n- [ ] todo\n* bullet\n1. numbered\n\nplain', id)
    expect(items.map((i) => [i.title, i.done])).toEqual([['done', true], ['todo', false], ['bullet', false], ['numbered', false], ['plain', false]])
    expect(items.map((i) => i.sortOrder)).toEqual([0, 1024, 2048, 3072, 4096])
    expect(new Set(items.map((i) => i.id)).size).toBe(5)
  })
  it('turns items back into checkbox lines, in order', () => {
    const items = [
      { id: 'b', title: 'second', done: true, sortOrder: 2 },
      { id: 'a', title: 'first', done: false, sortOrder: 1 },
    ]
    expect(itemsToNotes(items)).toBe('- [ ] first\n- [x] second')
  })
  it('round-trips', () => {
    const text = '- [ ] a\n- [x] b'
    expect(itemsToNotes(notesToItems(text, id))).toBe(text)
    expect(notesToItems('', id)).toEqual([])
    expect(itemsToNotes([])).toBe('')
  })
})

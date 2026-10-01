import { describe, expect, it } from 'vitest'
import { parseBlocks, parseInline, toggleCheckbox } from './markdown'

const t = (text: string) => ({ type: 'text', text })

describe('parseInline', () => {
  it('leaves plain text alone', () => {
    expect(parseInline('hello world')).toEqual([t('hello world')])
    expect(parseInline('')).toEqual([])
  })
  it('reads bold, italic, strike and code', () => {
    expect(parseInline('a **b** c')).toEqual([t('a '), { type: 'bold', children: [t('b')] }, t(' c')])
    expect(parseInline('*i*')).toEqual([{ type: 'italic', children: [t('i')] }])
    expect(parseInline('~~s~~')).toEqual([{ type: 'strike', children: [t('s')] }])
    expect(parseInline('use `npm test`')).toEqual([t('use '), { type: 'code', text: 'npm test' }])
  })
  it('nests', () => {
    expect(parseInline('**bold *and italic***')).toEqual([{ type: 'bold', children: [t('bold '), { type: 'italic', children: [t('and italic')] }] }])
  })
  it('does not interpret markup inside code', () => {
    expect(parseInline('`**not bold**`')).toEqual([{ type: 'code', text: '**not bold**' }])
  })
  it('keeps unmatched markers as text', () => {
    expect(parseInline('2 * 3 = 6')).toEqual([t('2 * 3 = 6')])
    expect(parseInline('**unclosed')).toEqual([t('**unclosed')])
  })
  it('makes links only for safe schemes', () => {
    expect(parseInline('[docs](https://example.com)')).toEqual([{ type: 'link', href: 'https://example.com', children: [t('docs')] }])
    expect(parseInline('[me](mailto:a@b.co)')[0]).toMatchObject({ type: 'link' })
    expect(parseInline('[x](javascript:alert(1))')).toEqual([t('[x](javascript:alert(1))')])
    expect(parseInline('[x](data:text/html,hi)')).toEqual([t('[x](data:text/html,hi)')])
    expect(parseInline('[x](//evil.com)')).toEqual([t('[x](//evil.com)')])
  })
  it('never produces anything but the node types above (no raw HTML passthrough)', () => {
    const nodes = parseInline('<img src=x onerror=alert(1)> **b**')
    expect(nodes[0]).toEqual(t('<img src=x onerror=alert(1)> '))
    expect(JSON.stringify(nodes)).not.toContain('"html"')
  })
})

describe('parseBlocks', () => {
  it('recognises each block type', () => {
    const blocks = parseBlocks(['# Title', '- item', '1. first', '- [ ] todo', '- [x] done', '> quote', '---', 'plain', ''].join('\n'))
    expect(blocks.map((b) => b.type)).toEqual(['heading', 'bullet', 'number', 'check', 'check', 'quote', 'rule', 'paragraph', 'blank'])
    expect(blocks[0]).toMatchObject({ level: 1 })
    expect(blocks[3]).toMatchObject({ checked: false, line: 3 })
    expect(blocks[4]).toMatchObject({ checked: true, line: 4 })
    expect(blocks[2]).toMatchObject({ n: 1 })
  })
  it('heading levels', () => {
    expect(parseBlocks('### h3')[0]).toMatchObject({ level: 3 })
    expect(parseBlocks('#### too deep')[0]).toMatchObject({ type: 'paragraph' })
    expect(parseBlocks('#nospace')[0]).toMatchObject({ type: 'paragraph' })
  })
  it('inline markup applies inside blocks', () => {
    expect(parseBlocks('- **bold** item')[0]).toEqual({ type: 'bullet', children: [{ type: 'bold', children: [t('bold')] }, t(' item')] })
  })
})

describe('toggleCheckbox', () => {
  const text = 'intro\n- [ ] a\n- [x] b\n- plain'
  it('flips an unchecked box and a checked one', () => {
    expect(toggleCheckbox(text, 1)).toBe('intro\n- [x] a\n- [x] b\n- plain')
    expect(toggleCheckbox(text, 2)).toBe('intro\n- [ ] a\n- [ ] b\n- plain')
  })
  it('leaves other lines and bad indexes alone', () => {
    expect(toggleCheckbox(text, 0)).toBe(text)
    expect(toggleCheckbox(text, 3)).toBe(text)
    expect(toggleCheckbox(text, 99)).toBe(text)
  })
})

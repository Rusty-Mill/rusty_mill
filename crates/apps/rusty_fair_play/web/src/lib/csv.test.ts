import { describe, expect, it } from 'vitest'
import { parseCsv, parseStandards } from './csv'

describe('parseCsv', () => {
  it('handles quoted commas, doubled quotes, newlines in quotes, CRLF and a missing final newline', () => {
    const text = 'a,b,c\r\n1,"x, y","say ""hi"""\n2,"two\nlines",\n\n3,,last'
    expect(parseCsv(text)).toEqual([
      ['a', 'b', 'c'],
      ['1', 'x, y', 'say "hi"'],
      ['2', 'two\nlines', ''],
      ['3', '', 'last'],
    ])
    expect(parseCsv('')).toEqual([])
    expect(() => parseCsv('a,"open')).toThrow(/unterminated/)
  })

  it('splits standards on | and drops blanks', () => {
    expect(parseStandards(' daily | weekly ||')).toEqual(['daily', 'weekly'])
    expect(parseStandards('')).toEqual([])
  })
})

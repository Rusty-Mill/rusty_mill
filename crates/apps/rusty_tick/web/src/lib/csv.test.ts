import { describe, expect, it } from 'vitest'
import { parseCsv } from './csv'

describe('parseCsv', () => {
  it('reads plain and quoted fields, doubled quotes and line breaks inside quotes', () => {
    expect(parseCsv('a,b,c\r\n"x, y","say ""hi""","two\nlines"\n')).toEqual([
      ['a', 'b', 'c'],
      ['x, y', 'say "hi"', 'two\nlines'],
    ])
  })

  it('drops a byte-order mark and blank lines, keeps empty fields and a last line with no newline', () => {
    expect(parseCsv('﻿a,,b\n\n,\nlast')).toEqual([['a', '', 'b'], ['', ''], ['last']])
  })

  it('keeps a stray quote inside an unquoted field', () => {
    expect(parseCsv('5" screen,ok')).toEqual([['5" screen', 'ok']])
  })

  it('reads nothing from empty text', () => {
    expect(parseCsv('')).toEqual([])
  })
})

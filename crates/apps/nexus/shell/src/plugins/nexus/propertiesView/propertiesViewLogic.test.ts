import { test } from 'node:test'
import assert from 'node:assert/strict'
import { buildListArgs, decodePage, formatCell, OVERSCAN, PAGE_SIZE, shouldLoadMore } from './propertiesViewLogic'

test('decodePage keeps well-formed rows and defaults the rest', () => {
  const page = decodePage({
    columns: ['a', 3, 'b'],
    rows: [
      { path: 'x.md', title: 'X', properties: { a: 1 } },
      { path: 'y.md' },
      { title: 'no path' },
      null,
    ],
    total: 7,
  })
  assert.deepEqual(page.columns, ['a', 'b'])
  assert.equal(page.rows.length, 2)
  assert.deepEqual(page.rows[1], { path: 'y.md', title: 'y.md', properties: {} })
  assert.equal(page.total, 7)
  assert.deepEqual(decodePage('junk'), { columns: [], rows: [], total: 0 })
  assert.equal(decodePage({ rows: [{ path: 'z.md' }] }).total, 1, 'total falls back to row count')
})

test('formatCell joins arrays and stringifies objects', () => {
  assert.equal(formatCell(['x', 2]), 'x, 2')
  assert.equal(formatCell({ a: 1 }), '{"a":1}')
  assert.equal(formatCell(null), '')
  assert.equal(formatCell(3), '3')
})

test('shouldLoadMore triggers only near the tail with more rows remaining', () => {
  assert.equal(shouldLoadMore(0, 0, 500, false), false, 'nothing cached yet')
  assert.equal(shouldLoadMore(99, 100, 100, false), false, 'everything loaded')
  assert.equal(shouldLoadMore(99, 100, 500, true), false, 'already loading')
  assert.equal(shouldLoadMore(100 - OVERSCAN - 1, 100, 500, false), false, 'not near tail')
  assert.equal(shouldLoadMore(100 - OVERSCAN, 100, 500, false), true)
})

test('buildListArgs omits empty filters and never sends value without key', () => {
  assert.deepEqual(buildListArgs('', 'x', 0), { limit: PAGE_SIZE, offset: 0 })
  assert.deepEqual(buildListArgs(' status ', '', 100), { key: 'status', limit: PAGE_SIZE, offset: 100 })
  assert.deepEqual(buildListArgs('status', ' draft ', 0), {
    key: 'status',
    value: 'draft',
    limit: PAGE_SIZE,
    offset: 0,
  })
})

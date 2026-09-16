import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  decodeRow,
  formatCell,
  inputToValue,
  isPropertyType,
  labelForType,
  PROPERTY_TYPES,
  valueToInput,
} from './propertyEditors'

test('every property type has a label and round-trips the guard', () => {
  for (const t of PROPERTY_TYPES) {
    assert.ok(labelForType(t).length > 0)
    assert.ok(isPropertyType(t))
  }
  assert.equal(isPropertyType('nope'), false)
  assert.equal(labelForType('date_time'), 'Date & time')
})

test('decodeRow tolerates unknown types and rejects bad shapes', () => {
  assert.deepEqual(decodeRow({ key: 'k', property_type: 'weird', value: 1 }), {
    key: 'k',
    property_type: 'text',
    value: 1,
  })
  assert.equal(decodeRow({ property_type: 'text' }), null)
  assert.equal(decodeRow(null), null)
})

test('valueToInput renders lists, datetimes, and scalars', () => {
  assert.equal(valueToInput('tags', ['a', 'b']), 'a, b')
  assert.equal(valueToInput('list', 'solo'), 'solo')
  assert.equal(valueToInput('date_time', '2026-09-08 10:30:00'), '2026-09-08T10:30')
  assert.equal(valueToInput('number', 3), '3')
  assert.equal(valueToInput('text', null), '')
})

test('inputToValue coerces per type and signals removal with null', () => {
  assert.equal(inputToValue('number', ' 42 '), 42)
  assert.equal(inputToValue('number', 'x'), null)
  assert.equal(inputToValue('number', ''), null)
  assert.deepEqual(inputToValue('tags', 'a, ,b '), ['a', 'b'])
  assert.deepEqual(inputToValue('list', ''), [])
  assert.equal(inputToValue('boolean', 'true'), true)
  assert.equal(inputToValue('text', '  '), null)
  assert.equal(inputToValue('date', '2026-09-08'), '2026-09-08')
})

test('formatCell joins arrays and stringifies objects', () => {
  assert.equal(formatCell(['x', 2]), 'x, 2')
  assert.equal(formatCell({ a: 1 }), '{"a":1}')
  assert.equal(formatCell(undefined), '')
  assert.equal(formatCell(false), 'false')
})

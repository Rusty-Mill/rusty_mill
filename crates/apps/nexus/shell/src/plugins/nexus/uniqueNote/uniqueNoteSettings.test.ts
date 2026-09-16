import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  basename,
  buildCreateArgs,
  CONFIG_KEY_FILE_LOCATION,
  CONFIG_KEY_ID_FORMAT,
  CONFIG_KEY_SEPARATOR,
  DEFAULT_ID_FORMAT,
  DEFAULT_SEPARATOR,
  readUniqueNoteSettings,
} from './uniqueNoteSettings'

function getValueFrom(values: Record<string, unknown>) {
  return <T>(key: string, defaultValue: T): T => (key in values ? (values[key] as T) : defaultValue)
}

test('readUniqueNoteSettings falls back to engine defaults on blanks', () => {
  const settings = readUniqueNoteSettings(
    getValueFrom({
      [CONFIG_KEY_ID_FORMAT]: '   ',
      [CONFIG_KEY_SEPARATOR]: '',
      [CONFIG_KEY_FILE_LOCATION]: '  ',
    }),
  )
  assert.deepEqual(settings, { idFormat: DEFAULT_ID_FORMAT, separator: DEFAULT_SEPARATOR, fileLocation: '' })
})

test('readUniqueNoteSettings trims folder slashes but keeps separator verbatim', () => {
  const settings = readUniqueNoteSettings(
    getValueFrom({
      [CONFIG_KEY_ID_FORMAT]: ' %Y-%m-%d ',
      [CONFIG_KEY_SEPARATOR]: ' - ',
      [CONFIG_KEY_FILE_LOCATION]: '/zettel/inbox/',
    }),
  )
  assert.deepEqual(settings, { idFormat: '%Y-%m-%d', separator: ' - ', fileLocation: 'zettel/inbox' })
})

test('buildCreateArgs omits values equal to the engine defaults', () => {
  const args = buildCreateArgs('  Hello  ', {
    idFormat: DEFAULT_ID_FORMAT,
    separator: DEFAULT_SEPARATOR,
    fileLocation: '',
  })
  assert.deepEqual(args, { title: 'Hello' })
})

test('buildCreateArgs forwards overridden values', () => {
  const args = buildCreateArgs('x', { idFormat: '%Y', separator: '_', fileLocation: 'z' })
  assert.deepEqual(args, { title: 'x', id_format: '%Y', separator: '_', folder: 'z' })
})

test('basename strips forge-relative directories', () => {
  assert.equal(basename('a/b/c.md'), 'c.md')
  assert.equal(basename('c.md'), 'c.md')
})

import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  CONFIG_KEY_CONFIRM_MERGE,
  CONFIG_KEY_DELETED_FILES_DESTINATION,
  CONFIG_KEY_LINKS_AUTO_UPDATE,
  CONFIG_KEY_TEXT_AFTER_EXTRACTION,
  mergeTargetCandidates,
  readNoteComposerSettings,
  replacementForExtraction,
  stemOf,
} from './noteComposerLogic'

function getValueFrom(values: Record<string, unknown>) {
  return <T>(key: string, defaultValue: T): T => (key in values ? (values[key] as T) : defaultValue)
}

test('readNoteComposerSettings defaults: link, confirm, update links, system trash', () => {
  const settings = readNoteComposerSettings(getValueFrom({}))
  assert.deepEqual(settings, {
    textAfterExtraction: 'link',
    confirmMerge: true,
    updateLinks: true,
    destination: 'system',
  })
})

test('readNoteComposerSettings honours stored values and rejects unknown ones', () => {
  const settings = readNoteComposerSettings(
    getValueFrom({
      [CONFIG_KEY_TEXT_AFTER_EXTRACTION]: 'embed',
      [CONFIG_KEY_CONFIRM_MERGE]: false,
      [CONFIG_KEY_LINKS_AUTO_UPDATE]: false,
      [CONFIG_KEY_DELETED_FILES_DESTINATION]: 'bogus',
    }),
  )
  assert.deepEqual(settings, {
    textAfterExtraction: 'embed',
    confirmMerge: false,
    updateLinks: false,
    destination: 'forge',
  })
  assert.equal(
    readNoteComposerSettings(getValueFrom({ [CONFIG_KEY_TEXT_AFTER_EXTRACTION]: 'weird' })).textAfterExtraction,
    'link',
  )
})

test('stemOf strips folder and .md extension only', () => {
  assert.equal(stemOf('notes/My Idea.md'), 'My Idea')
  assert.equal(stemOf('Top.MD'), 'Top')
  assert.equal(stemOf('notes/data.csv'), 'data.csv')
})

test('replacementForExtraction follows the textAfterExtraction setting', () => {
  assert.equal(replacementForExtraction('link', 'inbox/Part.md'), '[[Part]]')
  assert.equal(replacementForExtraction('embed', 'inbox/Part.md'), '![[Part]]')
  assert.equal(replacementForExtraction('nothing', 'inbox/Part.md'), '')
})

test('mergeTargetCandidates excludes the source and non-markdown, sorted', () => {
  const out = mergeTargetCandidates(['z.md', 'a.md', 'src.md', 'img.png', 'b/c.md'], 'src.md')
  assert.deepEqual(out, ['a.md', 'b/c.md', 'z.md'])
})

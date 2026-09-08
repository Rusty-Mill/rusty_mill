// shell/src/plugins/nexus/noteComposer/index.ts
//
// RFC 0009 — note composer, ported from nexus_forge's
// forge-plugin-note-composer. Three commands:
//
//   * Merge current note into…  — pick a target, `note_merge` appends the
//     active note to it, redirects inbound links, trashes the source
//     (per the files "deleted files" setting), then opens the target.
//   * Extract selection to new note — prompt for a title,
//     `note_create_from_title` writes the selection as the body, and the
//     selection in the still-open source buffer is replaced with a link /
//     embed / nothing per `nexus.settings.noteComposer.textAfterExtraction`.
//     The replacement is a plain CM6 dispatch (same path as templates'
//     insert-at-cursor, #367) because the shell owns the open buffer.
//   * Create note from title — prompt, create, open.
//
// Wires the previously dead `nexus.settings.noteComposer.{textAfterExtraction,
// confirmMerge}` keys (SettingsStubPages.tsx). `templateLocation` stays
// unwired for the same reason dailyNotes left its template key alone: the
// templates engine has no caller-supplied `{{content}}` / `{{fromTitle}}`
// substitution yet, so honouring it faithfully is a templates-engine change.

import type { Plugin, PluginAPI } from '../../../types/plugin'
import { getActiveCmView } from '../editor/runtime'
import { useEditorStore } from '../editor/editorStore'
import {
  basename,
  mergeTargetCandidates,
  readNoteComposerSettings,
  replacementForExtraction,
  type NoteComposerSettings,
} from './noteComposerLogic'

const STORAGE_PLUGIN_ID = 'com.nexus.storage'
const EVENT_FILE_OPEN = 'files:open'

const COMMAND_MERGE = 'nexus.noteComposer.mergeInto'
const COMMAND_EXTRACT = 'nexus.noteComposer.extractSelection'
const COMMAND_CREATE = 'nexus.noteComposer.createFromTitle'

interface NoteMergeReply {
  target: string
  files_rewritten: number
  links_updated: number
  trash_id: string | null
}

interface NoteCreateFromTitleReply {
  path: string
}

function settings(api: PluginAPI): NoteComposerSettings {
  return readNoteComposerSettings((key, fallback) => api.configuration.getValue(key, fallback))
}

function openNote(api: PluginAPI, relpath: string): void {
  api.events.emit(EVENT_FILE_OPEN, { relpath, name: basename(relpath) })
}

function fail(api: PluginAPI, what: string, e: unknown): void {
  api.notifications.show({
    message: `${what}: ${e instanceof Error ? e.message : String(e)}`,
    type: 'error',
    duration: 8000,
  })
}

async function markdownPaths(api: PluginAPI): Promise<string[]> {
  const raw = await api.kernel.invoke<unknown>(STORAGE_PLUGIN_ID, 'query_files', {
    file_type: 'markdown',
  })
  if (!Array.isArray(raw)) return []
  return raw
    .map((row) => (row && typeof row === 'object' ? (row as { path?: unknown }).path : undefined))
    .filter((p): p is string => typeof p === 'string')
}

async function mergeInto(api: PluginAPI): Promise<void> {
  const source = useEditorStore.getState().activeRelpath
  if (!source || !source.toLowerCase().endsWith('.md')) {
    api.notifications.show({ message: 'Merge requires an active markdown note.', type: 'warning' })
    return
  }

  let candidates: string[]
  try {
    candidates = mergeTargetCandidates(await markdownPaths(api), source)
  } catch (e) {
    fail(api, 'Merge: listing notes failed', e)
    return
  }
  if (candidates.length === 0) {
    api.notifications.show({ message: 'No other notes to merge into.', type: 'info' })
    return
  }

  const target = await api.input.pick<string>(
    candidates.map((p) => ({ label: basename(p), description: p, value: p })),
    { title: `Merge "${basename(source)}" into…`, placeholder: 'Target note' },
  )
  if (!target) return

  const cfg = settings(api)
  if (cfg.confirmMerge) {
    const ok = await api.input.confirm(
      `Merge "${basename(source)}" into "${basename(target)}"? The source note will be ` +
        (cfg.destination === 'permanent' ? 'deleted permanently.' : 'moved to the trash.'),
    )
    if (!ok) return
  }

  let reply: NoteMergeReply
  try {
    reply = await api.kernel.invoke<NoteMergeReply>(STORAGE_PLUGIN_ID, 'note_merge', {
      source,
      target,
      update_links: cfg.updateLinks,
      destination: cfg.destination,
    })
  } catch (e) {
    fail(api, 'Merge failed', e)
    return
  }

  useEditorStore.getState().closeTab(source)
  openNote(api, reply.target)
  const links =
    reply.links_updated > 0
      ? ` ${reply.links_updated} link${reply.links_updated === 1 ? '' : 's'} redirected.`
      : ''
  api.notifications.show({
    message: `Merged "${basename(source)}" into "${basename(reply.target)}".${links}`,
    type: 'info',
  })
}

async function extractSelection(api: PluginAPI): Promise<void> {
  const view = getActiveCmView()
  const source = useEditorStore.getState().activeRelpath
  if (!view || !source) {
    api.notifications.show({ message: 'Extract requires an active editor tab.', type: 'warning' })
    return
  }
  const sel = view.state.selection.main
  const selected = view.state.sliceDoc(sel.from, sel.to)
  if (selected.trim().length === 0) {
    api.notifications.show({ message: 'Select the text to extract first.', type: 'warning' })
    return
  }

  const title = await api.input.prompt('New note title', basename(source).replace(/\.md$/i, ''))
  if (title === null || title.trim() === '') return

  let reply: NoteCreateFromTitleReply
  try {
    reply = await api.kernel.invoke<NoteCreateFromTitleReply>(
      STORAGE_PLUGIN_ID,
      'note_create_from_title',
      { title, content: selected.endsWith('\n') ? selected : `${selected}\n` },
    )
  } catch (e) {
    fail(api, 'Extract failed', e)
    return
  }

  // Re-check the view: the prompt was async and the tab may have changed.
  const current = getActiveCmView()
  if (current !== view || useEditorStore.getState().activeRelpath !== source) {
    api.notifications.show({
      message: `Created "${reply.path}", but the source tab changed so the selection was left in place.`,
      type: 'warning',
    })
    openNote(api, reply.path)
    return
  }
  const replacement = replacementForExtraction(settings(api).textAfterExtraction, reply.path)
  current.dispatch({
    changes: { from: sel.from, to: sel.to, insert: replacement },
    scrollIntoView: true,
  })
  openNote(api, reply.path)
}

async function createFromTitle(api: PluginAPI): Promise<void> {
  const title = await api.input.prompt('New note title')
  if (title === null || title.trim() === '') return
  let reply: NoteCreateFromTitleReply
  try {
    reply = await api.kernel.invoke<NoteCreateFromTitleReply>(
      STORAGE_PLUGIN_ID,
      'note_create_from_title',
      { title },
    )
  } catch (e) {
    fail(api, 'Create note failed', e)
    return
  }
  openNote(api, reply.path)
}

export const noteComposerPlugin: Plugin = {
  manifest: {
    id: 'nexus.noteComposer',
    name: 'Note Composer',
    version: '0.1.0',
    core: false,
    activationEvents: ['onStartup'],
    dependsOn: ['com.nexus.storage', 'nexus.editor'],
    contributes: {
      commands: [
        { id: COMMAND_MERGE, title: 'Merge Current Note Into…', category: 'Note Composer' },
        { id: COMMAND_EXTRACT, title: 'Extract Selection to New Note', category: 'Note Composer' },
        { id: COMMAND_CREATE, title: 'Create Note From Title', category: 'Note Composer' },
      ],
    },
  },

  activate(api: PluginAPI) {
    api.commands.register(COMMAND_MERGE, () => void mergeInto(api))
    api.commands.register(COMMAND_EXTRACT, () => void extractSelection(api))
    api.commands.register(COMMAND_CREATE, () => void createFromTitle(api))
  },
}

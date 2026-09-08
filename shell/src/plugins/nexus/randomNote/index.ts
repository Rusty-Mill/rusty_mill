// shell/src/plugins/nexus/randomNote/index.ts
//
// RFC 0009 — "Open random note", ported from nexus_forge's
// forge-plugin-random-note. The draw lives engine-side
// (`com.nexus.storage::note_random`) so CLI/TUI/MCP share it; the shell
// only supplies the active note to exclude and opens the result.

import type { Plugin, PluginAPI } from '../../../types/plugin'
import { useEditorStore } from '../editor/editorStore'

const STORAGE_PLUGIN_ID = 'com.nexus.storage'
const EVENT_FILE_OPEN = 'files:open'

const COMMAND_OPEN = 'nexus.randomNote.open'

interface NoteRandomReply {
  path: string | null
}

/** Basename of a forge-relative path. Forward-slash only. */
function basename(relpath: string): string {
  const i = relpath.lastIndexOf('/')
  return i === -1 ? relpath : relpath.slice(i + 1)
}

async function openRandomNote(api: PluginAPI): Promise<void> {
  const exclude = useEditorStore.getState().activeRelpath ?? undefined

  let reply: NoteRandomReply
  try {
    reply = await api.kernel.invoke<NoteRandomReply>(STORAGE_PLUGIN_ID, 'note_random', { exclude })
  } catch (e) {
    api.notifications.show({ message: `Failed to pick a random note: ${String(e)}`, type: 'error' })
    return
  }

  if (!reply.path) {
    api.notifications.show({ message: 'No other notes to open.', type: 'info' })
    return
  }
  api.events.emit(EVENT_FILE_OPEN, { relpath: reply.path, name: basename(reply.path) })
}

export const randomNotePlugin: Plugin = {
  manifest: {
    id: 'nexus.randomNote',
    name: 'Random Note',
    version: '0.1.0',
    core: false,
    activationEvents: ['onStartup'],
    dependsOn: ['com.nexus.storage', 'nexus.editor'],
    contributes: {
      commands: [{ id: COMMAND_OPEN, title: 'Open Random Note', category: 'Random Note' }],
    },
  },

  activate(api: PluginAPI) {
    api.commands.register(COMMAND_OPEN, () => void openRandomNote(api))
  },
}

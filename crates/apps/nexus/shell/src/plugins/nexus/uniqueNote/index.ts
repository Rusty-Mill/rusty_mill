// shell/src/plugins/nexus/uniqueNote/index.ts
//
// RFC 0009 — Zettelkasten-style "unique note" creation, ported from
// nexus_forge's forge-plugin-unique-note. One command: prompt for a
// title, ask `com.nexus.storage::note_create_unique` to create
// `{id}{separator}{title}.md` (id = chrono-formatted local time, `-N`
// suffix on collision), then open it. Naming is driven by the
// `nexus.settings.uniqueNote.*` keys (Settings → Core plugins → Unique
// note); blanks fall back to the engine defaults.

import type { Plugin, PluginAPI } from '../../../types/plugin'
import { basename, buildCreateArgs, readUniqueNoteSettings } from './uniqueNoteSettings'

const STORAGE_PLUGIN_ID = 'com.nexus.storage'
const EVENT_FILE_OPEN = 'files:open'

const COMMAND_CREATE = 'nexus.uniqueNote.create'

interface NoteCreateUniqueReply {
  path: string
}

async function createUniqueNote(api: PluginAPI): Promise<void> {
  const title = await api.input.prompt('Unique note title (optional)', 'Leave empty for id only')
  if (title === null) return

  const settings = readUniqueNoteSettings((key, fallback) => api.configuration.getValue(key, fallback))
  const args = buildCreateArgs(title, settings)

  let reply: NoteCreateUniqueReply
  try {
    reply = await api.kernel.invoke<NoteCreateUniqueReply>(STORAGE_PLUGIN_ID, 'note_create_unique', args)
  } catch (e) {
    api.notifications.show({
      message: `Failed to create unique note: ${String(e)}`,
      type: 'error',
    })
    return
  }

  api.events.emit(EVENT_FILE_OPEN, { relpath: reply.path, name: basename(reply.path) })
}

export const uniqueNotePlugin: Plugin = {
  manifest: {
    id: 'nexus.uniqueNote',
    name: 'Unique Note',
    version: '0.1.0',
    core: false,
    activationEvents: ['onStartup'],
    dependsOn: ['com.nexus.storage'],
    contributes: {
      commands: [{ id: COMMAND_CREATE, title: 'Create Unique Note', category: 'Unique Note' }],
    },
  },

  activate(api: PluginAPI) {
    api.commands.register(COMMAND_CREATE, () => void createUniqueNote(api))
  },
}

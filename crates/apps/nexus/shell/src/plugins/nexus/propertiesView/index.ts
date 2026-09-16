// shell/src/plugins/nexus/propertiesView/index.ts
//
// RFC 0009 — Properties table view, ported from nexus_forge's
// forge-plugin-properties-view: a main-pane, virtualized, read-only table
// of every indexed note's frontmatter, paged through
// `com.nexus.storage::properties_list` with a stable column set (the
// inferred ∪ declared schema). Editing stays in the File Properties
// panel (nexus.fileProperties).

import { createElement } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import type { Plugin, PluginAPI } from '../../../types/plugin'
import { ViewBase, workspace, type Leaf } from '../../../workspace'
import { PropertiesTableView } from './PropertiesTableView'
import { setInvoke, usePropertiesViewStore } from './propertiesViewStore'
import type { PropertyViewRow } from './propertiesViewLogic'

const VIEW_TYPE = 'properties-view'
const COMMAND_OPEN = 'nexus.propertiesView.open'
const COMMAND_REFRESH = 'nexus.propertiesView.refresh'
const EVENT_FILE_OPEN = 'files:open'
const TOPIC_FILE_MODIFIED = 'com.nexus.storage.file_modified'
const TOPIC_FILE_DELETED = 'com.nexus.storage.file_deleted'

function basename(relpath: string): string {
  const i = relpath.lastIndexOf('/')
  return i === -1 ? relpath : relpath.slice(i + 1)
}

class PropertiesPaneView extends ViewBase {
  readonly viewType = VIEW_TYPE
  private root: Root | null = null
  private readonly openRow: (row: PropertyViewRow) => void

  constructor(leaf: Leaf, openRow: (row: PropertyViewRow) => void) {
    super(leaf)
    this.openRow = openRow
  }

  getIcon(): string {
    return 'table'
  }

  async onOpen(containerEl: HTMLElement): Promise<void> {
    this.root = createRoot(containerEl)
    this.root.render(createElement(PropertiesTableView, { onOpen: this.openRow }))
  }

  async onClose(): Promise<void> {
    this.root?.unmount()
    this.root = null
  }
}

export const propertiesViewPlugin: Plugin = {
  manifest: {
    id: 'nexus.propertiesView',
    name: 'Properties View',
    version: '0.1.0',
    core: false,
    activationEvents: ['onStartup'],
    dependsOn: ['com.nexus.storage', 'nexus.workspace'],
    contributes: {
      commands: [
        { id: COMMAND_OPEN, title: 'Open Properties Table', category: 'Properties' },
        { id: COMMAND_REFRESH, title: 'Refresh Properties Table', category: 'Properties' },
      ],
    },
  },

  activate(api: PluginAPI) {
    setInvoke((pluginId, command, args) => api.kernel.invoke(pluginId, command, args))

    const openRow = (row: PropertyViewRow) => {
      api.events.emit(EVENT_FILE_OPEN, { relpath: row.path, name: basename(row.path) })
    }
    api.viewRegistry.register(VIEW_TYPE, (leaf) => new PropertiesPaneView(leaf, openRow))

    api.commands.register(COMMAND_OPEN, async () => {
      const leaf = await workspace.ensureLeafOfType(VIEW_TYPE, 'main')
      workspace.revealLeaf(leaf)
    })
    api.commands.register(COMMAND_REFRESH, () => void usePropertiesViewStore.getState().reload())

    // Keep the table honest under external edits: a modified or deleted
    // file invalidates the cached pages. Cheap because reload() is a
    // single first-page fetch and no-ops while one is in flight.
    const invalidate = () => {
      const s = usePropertiesViewStore.getState()
      if (s.rows.length > 0 || s.columns.length > 0) void s.reload()
    }
    void api.kernel.on(TOPIC_FILE_MODIFIED, invalidate)
    void api.kernel.on(TOPIC_FILE_DELETED, invalidate)
  },
}

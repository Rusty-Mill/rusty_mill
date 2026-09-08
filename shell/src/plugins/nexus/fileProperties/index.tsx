// shell/src/plugins/nexus/fileProperties/index.tsx
//
// RFC 0009 — File Properties panel: the active note's frontmatter as a
// typed, editable form (ported from nexus_forge's properties-panel),
// above the file metadata rows this view always showed.
//
// Reads through `com.nexus.storage::properties_get` (effective type per
// key: app.toml override → index-inferred → value shape), writes through
// `properties_set`, and lets the user declare a key's type with a
// per-row picker backed by `properties_set_override`. Writes re-serialise
// the YAML block Brain-side, so comments inside it are not preserved —
// the banner says so.

import { createRoot, type Root } from 'react-dom/client'
import { createElement, useCallback, useEffect, useState } from 'react'
import type { Plugin, PluginAPI } from '../../../types/plugin'
import { ViewBase, workspace, type Leaf } from '../../../workspace'
import { useEditorStore } from '../editor/editorStore'
import { getKernel } from '../files/kernelClient'
import {
  decodeRow,
  inputToValue,
  labelForType,
  PROPERTY_TYPES,
  valueToInput,
  type PropertyRow,
  type PropertyType,
} from './propertyEditors'

const VIEW_TYPE = 'file-properties'
const COMMAND_FOCUS = 'nexus.fileProperties.focus'
const STORAGE_PLUGIN_ID = 'com.nexus.storage'
const TOPIC_FILE_MODIFIED = 'com.nexus.storage.file_modified'

/** Kernel `file_modified` events poke every mounted panel to refetch. */
const _versionListeners = new Set<() => void>()
function bumpExternalVersion(): void {
  for (const l of _versionListeners) l()
}

interface FileRecord {
  path?: unknown
  file_type?: unknown
  size_bytes?: unknown
  created_at?: unknown
  modified_at?: unknown
}

function basename(relpath: string): string {
  const i = relpath.lastIndexOf('/')
  return i === -1 ? relpath : relpath.slice(i + 1)
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / (1024 * 1024)).toFixed(2)} MB`
}

function formatTimestamp(secs: number): string {
  if (!Number.isFinite(secs) || secs <= 0) return '—'
  try {
    return new Date(secs * 1000).toLocaleString()
  } catch {
    return '—'
  }
}

const cellLabel: React.CSSProperties = {
  padding: '4px 8px',
  color: 'var(--text-muted)',
  verticalAlign: 'top',
  width: '40%',
}
const cellValue: React.CSSProperties = {
  padding: '4px 8px',
  color: 'var(--text-normal)',
  wordBreak: 'break-word',
}
const inputStyle: React.CSSProperties = {
  width: '100%',
  boxSizing: 'border-box',
  font: 'inherit',
  padding: '2px 4px',
  background: 'var(--background-primary)',
  color: 'var(--text-normal)',
  border: '1px solid var(--background-modifier-border)',
  borderRadius: 3,
}
const selectStyle: React.CSSProperties = { ...inputStyle, width: 'auto', padding: '1px 2px' }

function MetaRow({ label, value }: { label: string; value: string }) {
  return (
    <tr>
      <td style={cellLabel}>{label}</td>
      <td style={cellValue}>{value || <span style={{ color: 'var(--text-faint)' }}>—</span>}</td>
    </tr>
  )
}

interface PropertyRowProps {
  row: PropertyRow
  onValue: (key: string, value: unknown, type: PropertyType) => void
  onType: (key: string, type: PropertyType) => void
  onRemove: (key: string) => void
}

function PropertyEditorRow({ row, onValue, onType, onRemove }: PropertyRowProps) {
  const [draft, setDraft] = useState(() => valueToInput(row.property_type, row.value))
  useEffect(() => {
    setDraft(valueToInput(row.property_type, row.value))
  }, [row.property_type, row.value])

  const commit = () => {
    const next = inputToValue(row.property_type, draft)
    if (next === null) return
    if (valueToInput(row.property_type, next) === valueToInput(row.property_type, row.value)) return
    onValue(row.key, next, row.property_type)
  }

  let editor: React.ReactNode
  switch (row.property_type) {
    case 'boolean':
      editor = (
        <input
          type="checkbox"
          aria-label={row.key}
          checked={row.value === true}
          onChange={(e) => onValue(row.key, e.target.checked, 'boolean')}
        />
      )
      break
    case 'number':
    case 'date':
    case 'date_time':
      editor = (
        <input
          style={inputStyle}
          aria-label={row.key}
          type={row.property_type === 'number' ? 'number' : row.property_type === 'date' ? 'date' : 'datetime-local'}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === 'Enter') commit()
          }}
        />
      )
      break
    default:
      editor = (
        <input
          style={inputStyle}
          aria-label={row.key}
          type="text"
          value={draft}
          placeholder={row.property_type === 'list' || row.property_type === 'tags' ? 'a, b, c' : ''}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === 'Enter') commit()
          }}
        />
      )
  }

  return (
    <tr>
      <td style={cellLabel} title={labelForType(row.property_type)}>
        {row.key}
      </td>
      <td style={cellValue}>
        <div style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
          <div style={{ flex: 1 }}>{editor}</div>
          <select
            style={selectStyle}
            aria-label={`Type of ${row.key}`}
            title="Declared type (saved to app.toml)"
            value={row.property_type}
            onChange={(e) => onType(row.key, e.target.value as PropertyType)}
          >
            {PROPERTY_TYPES.map((t) => (
              <option key={t} value={t}>
                {labelForType(t)}
              </option>
            ))}
          </select>
          <button
            type="button"
            aria-label={`Remove ${row.key}`}
            title="Remove property"
            onClick={() => onRemove(row.key)}
            style={{ ...selectStyle, cursor: 'pointer' }}
          >
            ×
          </button>
        </div>
      </td>
    </tr>
  )
}

function FilePropertiesView() {
  const activeRelpath = useEditorStore((s) => s.activeRelpath)
  const [rows, setRows] = useState<PropertyRow[]>([])
  const [meta, setMeta] = useState<FileRecord | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [newKey, setNewKey] = useState('')
  const [newValue, setNewValue] = useState('')
  const [version, setVersion] = useState(0)

  useEffect(() => {
    const l = () => setVersion((v) => v + 1)
    _versionListeners.add(l)
    return () => {
      _versionListeners.delete(l)
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    if (!activeRelpath) {
      setRows([])
      setMeta(null)
      setError(null)
      setLoading(false)
      return
    }
    const kernel = getKernel()
    if (!kernel) {
      setError('Kernel not ready.')
      return
    }
    setLoading(true)
    setError(null)
    Promise.all([
      kernel.invoke<{ rows?: unknown }>(STORAGE_PLUGIN_ID, 'properties_get', { path: activeRelpath }),
      kernel.invoke<unknown>(STORAGE_PLUGIN_ID, 'query_files', {
        prefix: activeRelpath,
        include_deleted: false,
      }),
    ])
      .then(([props, rawFiles]) => {
        if (cancelled) return
        const decoded = Array.isArray(props?.rows)
          ? props.rows.map(decodeRow).filter((r): r is PropertyRow => r !== null)
          : []
        setRows(decoded)
        setMeta(
          Array.isArray(rawFiles)
            ? ((rawFiles as FileRecord[]).find((r) => r.path === activeRelpath) ?? null)
            : null,
        )
        setLoading(false)
      })
      .catch((err: unknown) => {
        if (cancelled) return
        setError(err instanceof Error ? err.message : String(err))
        setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [activeRelpath, version])

  const refresh = useCallback(() => setVersion((v) => v + 1), [])

  const write = useCallback(
    async (command: string, args: Record<string, unknown>) => {
      const kernel = getKernel()
      if (!kernel) return
      try {
        await kernel.invoke<unknown>(STORAGE_PLUGIN_ID, command, args)
        refresh()
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      }
    },
    [refresh],
  )

  const onValue = useCallback(
    (key: string, value: unknown, type: PropertyType) =>
      void write('properties_set', { path: activeRelpath, key, value, property_type: type }),
    [activeRelpath, write],
  )
  const onType = useCallback(
    (key: string, type: PropertyType) => void write('properties_set_override', { key, property_type: type }),
    [write],
  )
  const onRemove = useCallback(
    (key: string) => void write('properties_set', { path: activeRelpath, key, value: null }),
    [activeRelpath, write],
  )
  const onAdd = () => {
    const key = newKey.trim()
    if (!key || !activeRelpath) return
    const value = newValue.trim() === '' ? '' : newValue.trim()
    void write('properties_set', { path: activeRelpath, key, value })
    setNewKey('')
    setNewValue('')
  }

  if (!activeRelpath) {
    return <div style={{ padding: 16, fontSize: 12, color: 'var(--text-faint)' }}>No active note.</div>
  }
  if (loading && rows.length === 0 && !meta) {
    return <div style={{ padding: 16, fontSize: 12, color: 'var(--text-faint)' }}>Loading…</div>
  }
  const isMarkdown = activeRelpath.toLowerCase().endsWith('.md')
  const fileType = typeof meta?.file_type === 'string' ? meta.file_type : ''
  const size = typeof meta?.size_bytes === 'number' ? formatBytes(meta.size_bytes) : ''
  const created = typeof meta?.created_at === 'number' ? formatTimestamp(meta.created_at) : ''
  const modified = typeof meta?.modified_at === 'number' ? formatTimestamp(meta.modified_at) : ''

  return (
    <div style={{ padding: 8, fontSize: 12 }}>
      {error && (
        <div style={{ padding: '4px 8px', marginBottom: 6, color: 'var(--text-error)' }}>{error}</div>
      )}
      <table style={{ width: '100%', borderCollapse: 'collapse' }}>
        <tbody>
          <MetaRow label="name" value={basename(activeRelpath)} />
          <MetaRow label="path" value={activeRelpath} />
          <MetaRow label="type" value={fileType} />
          <MetaRow label="size" value={size} />
          <MetaRow label="created" value={created} />
          <MetaRow label="modified" value={modified} />
        </tbody>
      </table>
      {isMarkdown && (
        <>
          <div
            role="note"
            style={{
              margin: '8px 0 4px',
              padding: '4px 8px',
              color: 'var(--text-muted)',
              borderTop: '1px solid var(--background-modifier-border)',
            }}
          >
            Properties — editing rewrites the frontmatter block (YAML comments are not preserved).
          </div>
          <table style={{ width: '100%', borderCollapse: 'collapse' }}>
            <tbody>
              {rows.length === 0 && (
                <tr>
                  <td colSpan={2} style={{ ...cellValue, color: 'var(--text-faint)' }}>
                    No frontmatter properties.
                  </td>
                </tr>
              )}
              {rows.map((row) => (
                <PropertyEditorRow
                  key={row.key}
                  row={row}
                  onValue={onValue}
                  onType={onType}
                  onRemove={onRemove}
                />
              ))}
              <tr>
                <td style={cellLabel}>
                  <input
                    style={inputStyle}
                    aria-label="New property key"
                    placeholder="new key"
                    value={newKey}
                    onChange={(e) => setNewKey(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') onAdd()
                    }}
                  />
                </td>
                <td style={cellValue}>
                  <div style={{ display: 'flex', gap: 4 }}>
                    <input
                      style={inputStyle}
                      aria-label="New property value"
                      placeholder="value"
                      value={newValue}
                      onChange={(e) => setNewValue(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') onAdd()
                      }}
                    />
                    <button type="button" onClick={onAdd} style={{ ...selectStyle, cursor: 'pointer' }}>
                      Add
                    </button>
                  </div>
                </td>
              </tr>
            </tbody>
          </table>
        </>
      )}
    </div>
  )
}

class FilePropertiesPaneView extends ViewBase {
  readonly viewType = VIEW_TYPE
  private root: Root | null = null

  constructor(leaf: Leaf) {
    super(leaf)
  }

  getIcon(): string {
    return 'info'
  }

  async onOpen(containerEl: HTMLElement): Promise<void> {
    this.root = createRoot(containerEl)
    this.root.render(createElement(FilePropertiesView))
  }

  async onClose(): Promise<void> {
    this.root?.unmount()
    this.root = null
  }
}

export const filePropertiesPlugin: Plugin = {
  manifest: {
    id: 'nexus.fileProperties',
    name: 'File Properties',
    version: '0.1.0',
    core: false,
    activationEvents: [`onCommand:${COMMAND_FOCUS}`, `onView:${VIEW_TYPE}`],
    contributes: {
      commands: [{ id: COMMAND_FOCUS, title: 'Focus File Properties', category: 'View' }],
    },
  },

  activate(api: PluginAPI) {
    api.viewRegistry.register(VIEW_TYPE, (leaf) => new FilePropertiesPaneView(leaf))
    api.commands.register(COMMAND_FOCUS, async () => {
      const leaf = await workspace.ensureLeafOfType(VIEW_TYPE, 'right')
      workspace.revealLeaf(leaf)
    })
    // External edits (watcher) to the active note refresh the panel.
    void api.kernel.on(TOPIC_FILE_MODIFIED, (_topic: string, payload: unknown) => {
      const path = payload && typeof payload === 'object' ? (payload as { path?: unknown }).path : undefined
      if (typeof path !== 'string' || path === useEditorStore.getState().activeRelpath) bumpExternalVersion()
    })
  },
}

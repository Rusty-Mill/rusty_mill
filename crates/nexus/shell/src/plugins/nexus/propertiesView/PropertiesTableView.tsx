// RFC 0009 — virtualized read-only table of every note's frontmatter.
// Only the visible rows are mounted (@tanstack/react-virtual, the same
// dependency FilesTree / BasesTable use). Row click opens the note.

import { useEffect, useRef } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { formatCell, OVERSCAN, shouldLoadMore, type PropertyViewRow } from './propertiesViewLogic'
import { usePropertiesViewStore } from './propertiesViewStore'

const ROW_HEIGHT = 28
const TITLE_WIDTH = 220
const COL_WIDTH = 160

const cellStyle: React.CSSProperties = {
  flex: `0 0 ${COL_WIDTH}px`,
  padding: '0 8px',
  lineHeight: `${ROW_HEIGHT}px`,
  whiteSpace: 'nowrap',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  borderRight: '1px solid var(--background-modifier-border)',
}
const titleCellStyle: React.CSSProperties = { ...cellStyle, flex: `0 0 ${TITLE_WIDTH}px` }

interface Props {
  onOpen: (row: PropertyViewRow) => void
}

export function PropertiesTableView({ onOpen }: Props) {
  const columns = usePropertiesViewStore((s) => s.columns)
  const rows = usePropertiesViewStore((s) => s.rows)
  const total = usePropertiesViewStore((s) => s.total)
  const loading = usePropertiesViewStore((s) => s.loading)
  const error = usePropertiesViewStore((s) => s.error)
  const filterKey = usePropertiesViewStore((s) => s.filterKey)
  const filterValue = usePropertiesViewStore((s) => s.filterValue)

  useEffect(() => {
    if (rows.length === 0 && !loading) void usePropertiesViewStore.getState().reload()
    // First-paint load only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const scrollRef = useRef<HTMLDivElement | null>(null)
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: OVERSCAN,
  })
  const items = virtualizer.getVirtualItems()
  const lastVisible = items.length > 0 ? items[items.length - 1].index : 0

  useEffect(() => {
    if (shouldLoadMore(lastVisible, rows.length, total, loading)) {
      void usePropertiesViewStore.getState().loadMore()
    }
  }, [lastVisible, rows.length, total, loading])

  const width = TITLE_WIDTH + columns.length * COL_WIDTH

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', fontSize: 12 }}>
      <div
        style={{
          display: 'flex',
          gap: 8,
          alignItems: 'center',
          padding: '6px 8px',
          borderBottom: '1px solid var(--background-modifier-border)',
          color: 'var(--text-muted)',
        }}
      >
        <select
          aria-label="Filter by key"
          value={filterKey}
          onChange={(e) => void usePropertiesViewStore.getState().setFilter(e.target.value, filterValue)}
        >
          <option value="">All notes</option>
          {columns.map((c) => (
            <option key={c} value={c}>
              has {c}
            </option>
          ))}
        </select>
        <input
          aria-label="Filter value"
          placeholder={filterKey ? `${filterKey} equals…` : 'pick a key first'}
          disabled={!filterKey}
          defaultValue={filterValue}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              void usePropertiesViewStore.getState().setFilter(filterKey, (e.target as HTMLInputElement).value)
            }
          }}
          style={{ font: 'inherit', padding: '2px 4px' }}
        />
        <span>
          {rows.length} / {total} notes · {columns.length} keys
        </span>
        {loading && <span>Loading…</span>}
        {error && <span style={{ color: 'var(--text-error)' }}>{error}</span>}
        <button
          type="button"
          onClick={() => void usePropertiesViewStore.getState().reload()}
          style={{ marginLeft: 'auto', font: 'inherit' }}
        >
          Refresh
        </button>
      </div>

      {columns.length === 0 && rows.length === 0 && !loading ? (
        <div style={{ padding: 16, color: 'var(--text-faint)' }}>
          No frontmatter properties yet. Add a <code>---</code> block to a note to populate the table.
        </div>
      ) : (
        <div ref={scrollRef} style={{ flex: 1, overflow: 'auto' }}>
          <div style={{ minWidth: width }}>
            <div
              style={{
                display: 'flex',
                position: 'sticky',
                top: 0,
                zIndex: 1,
                background: 'var(--background-secondary)',
                borderBottom: '1px solid var(--background-modifier-border)',
                fontWeight: 600,
              }}
            >
              <div style={titleCellStyle}>Title</div>
              {columns.map((c) => (
                <div key={c} style={cellStyle} title={c}>
                  {c}
                </div>
              ))}
            </div>
            <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
              {items.map((vi) => {
                const row = rows[vi.index]
                if (!row) return null
                return (
                  <div
                    key={row.path}
                    role="row"
                    style={{
                      display: 'flex',
                      position: 'absolute',
                      top: 0,
                      left: 0,
                      right: 0,
                      height: ROW_HEIGHT,
                      transform: `translateY(${vi.start}px)`,
                      cursor: 'pointer',
                      borderBottom: '1px solid var(--background-modifier-border)',
                    }}
                    onClick={() => onOpen(row)}
                  >
                    <div style={titleCellStyle} title={row.path}>
                      {row.title}
                    </div>
                    {columns.map((c) => {
                      const text = formatCell(row.properties[c])
                      return (
                        <div key={c} style={cellStyle} title={text}>
                          {text}
                        </div>
                      )
                    })}
                  </div>
                )
              })}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

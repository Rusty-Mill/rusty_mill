// RFC 0009 — pure logic for the Properties table view (ported from
// nexus_forge's properties-view), kept separate from the React/zustand
// files so it's testable without the plugin stack.

/** Rows fetched per `properties_list` call. Bounds DOM/row-cache growth. */
export const PAGE_SIZE = 100
/** Virtualizer overscan; also the load-more threshold from the tail. */
export const OVERSCAN = 8

/** Reply row of `com.nexus.storage::properties_list`. */
export interface PropertyViewRow {
  path: string
  title: string
  properties: Record<string, unknown>
}

/** Reply of `com.nexus.storage::properties_list`. */
export interface PropertiesPage {
  columns: string[]
  rows: PropertyViewRow[]
  total: number
}

/** Coerce an untrusted reply into a page; malformed rows are dropped. */
export function decodePage(raw: unknown): PropertiesPage {
  const empty: PropertiesPage = { columns: [], rows: [], total: 0 }
  if (!raw || typeof raw !== 'object') return empty
  const r = raw as { columns?: unknown; rows?: unknown; total?: unknown }
  const columns = Array.isArray(r.columns) ? r.columns.filter((c): c is string => typeof c === 'string') : []
  const rows: PropertyViewRow[] = []
  if (Array.isArray(r.rows)) {
    for (const row of r.rows) {
      if (!row || typeof row !== 'object') continue
      const { path, title, properties } = row as { path?: unknown; title?: unknown; properties?: unknown }
      if (typeof path !== 'string') continue
      rows.push({
        path,
        title: typeof title === 'string' ? title : path,
        properties:
          properties && typeof properties === 'object' ? (properties as Record<string, unknown>) : {},
      })
    }
  }
  const total = typeof r.total === 'number' && Number.isFinite(r.total) ? r.total : rows.length
  return { columns, rows, total }
}

/** Display form of a cell value. */
export function formatCell(value: unknown): string {
  if (value === null || value === undefined) return ''
  if (Array.isArray(value)) return value.map((v) => String(v)).join(', ')
  if (typeof value === 'object') {
    try {
      return JSON.stringify(value)
    } catch {
      return ''
    }
  }
  return String(value)
}

/**
 * Whether the scroller is close enough to the cached tail that the next
 * page should be requested. `lastVisible` is the last virtual row index.
 */
export function shouldLoadMore(
  lastVisible: number,
  cached: number,
  total: number,
  loading: boolean,
): boolean {
  if (loading || cached === 0 || cached >= total) return false
  return lastVisible >= cached - OVERSCAN
}

/** Filter args for `properties_list`, omitting empty strings. */
export function buildListArgs(
  filterKey: string,
  filterValue: string,
  offset: number,
): { key?: string; value?: string; limit: number; offset: number } {
  const key = filterKey.trim()
  const value = filterValue.trim()
  const args: { key?: string; value?: string; limit: number; offset: number } = {
    limit: PAGE_SIZE,
    offset,
  }
  if (key) {
    args.key = key
    if (value) args.value = value
  }
  return args
}

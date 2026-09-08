// RFC 0009 — pure typed-property helpers for the File Properties panel,
// kept separate from index.tsx so they're testable without React or the
// plugin stack (mirrors noteComposer/noteComposerLogic.ts).

/** Wire names accepted by `com.nexus.storage::properties_{set,set_override}`. */
export const PROPERTY_TYPES = [
  'text',
  'number',
  'date',
  'date_time',
  'boolean',
  'list',
  'link',
  'tags',
] as const

export type PropertyType = (typeof PROPERTY_TYPES)[number]

export function isPropertyType(v: unknown): v is PropertyType {
  return typeof v === 'string' && (PROPERTY_TYPES as readonly string[]).includes(v)
}

export function labelForType(t: PropertyType): string {
  switch (t) {
    case 'text':
      return 'Text'
    case 'number':
      return 'Number'
    case 'date':
      return 'Date'
    case 'date_time':
      return 'Date & time'
    case 'boolean':
      return 'Boolean'
    case 'list':
      return 'List'
    case 'link':
      return 'Link'
    case 'tags':
      return 'Tags'
  }
}

/** Reply row of `properties_get`. */
export interface PropertyRow {
  key: string
  property_type: PropertyType
  value: unknown
}

/** Coerce an untrusted reply row; unknown types fall back to `text`. */
export function decodeRow(raw: unknown): PropertyRow | null {
  if (!raw || typeof raw !== 'object') return null
  const r = raw as { key?: unknown; property_type?: unknown; value?: unknown }
  if (typeof r.key !== 'string') return null
  return {
    key: r.key,
    property_type: isPropertyType(r.property_type) ? r.property_type : 'text',
    value: r.value,
  }
}

/** Render a stored value into the string an `<input>` shows. */
export function valueToInput(type: PropertyType, value: unknown): string {
  if (value === null || value === undefined) return ''
  if (type === 'list' || type === 'tags') {
    return Array.isArray(value) ? value.map((v) => String(v)).join(', ') : String(value)
  }
  if (type === 'date_time' && typeof value === 'string') {
    // `<input type="datetime-local">` wants `YYYY-MM-DDTHH:MM`; tolerate a
    // space separator and drop seconds/zone suffixes.
    return value.replace(' ', 'T').slice(0, 16)
  }
  return typeof value === 'string' ? value : JSON.stringify(value)
}

/**
 * Convert an `<input>` string back to the JSON value sent to
 * `properties_set`. Returns `null` for an empty input so the engine
 * removes the key, except for booleans (handled by the checkbox path)
 * and lists, where an empty input means an empty list.
 */
export function inputToValue(type: PropertyType, raw: string): unknown {
  const trimmed = raw.trim()
  switch (type) {
    case 'number': {
      if (trimmed === '') return null
      const n = Number(trimmed)
      return Number.isFinite(n) ? n : null
    }
    case 'list':
    case 'tags':
      return trimmed
        .split(',')
        .map((s) => s.trim())
        .filter((s) => s.length > 0)
    case 'boolean':
      return trimmed === 'true'
    default:
      return trimmed === '' ? null : trimmed
  }
}

/** Display form of any property value, for read-only cells. */
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

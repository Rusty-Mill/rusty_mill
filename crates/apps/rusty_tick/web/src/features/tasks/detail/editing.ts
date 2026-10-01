/**
 * Text-editing helpers for the description box: slash commands, formatting
 * shortcuts, and converting between checklist items and markdown lines.
 * All pure: they take text and a selection and return new text and a selection.
 */
import type { ChecklistItem } from '@/api/types'

export interface Edit {
  text: string
  /** Selection after the edit. */
  start: number
  end: number
}

// ---- slash commands ------------------------------------------------------

export interface SlashCommand {
  id: string
  label: string
  /** Text inserted in place of the `/query`; `|` marks where the caret goes. */
  insert: string
}

export const SLASH_COMMANDS: SlashCommand[] = [
  { id: 'h1', label: 'Heading 1', insert: '# |' },
  { id: 'h2', label: 'Heading 2', insert: '## |' },
  { id: 'h3', label: 'Heading 3', insert: '### |' },
  { id: 'bullet', label: 'Bulleted list', insert: '- |' },
  { id: 'number', label: 'Numbered list', insert: '1. |' },
  { id: 'todo', label: 'Checkbox', insert: '- [ ] |' },
  { id: 'quote', label: 'Quote', insert: '> |' },
  { id: 'divider', label: 'Divider', insert: '---\n|' },
  { id: 'code', label: 'Code', insert: '`|`' },
]

export interface Slash {
  /** Offset of the `/`. */
  start: number
  query: string
}

/** A `/word` right before the caret, at the start of a line or after a space. */
export function detectSlash(text: string, caret: number): Slash | null {
  const before = text.slice(0, caret)
  const m = /(^|[\s])\/([a-z0-9]*)$/i.exec(before)
  return m ? { start: caret - m[2]!.length - 1, query: m[2]!.toLowerCase() } : null
}

export function matchCommands(query: string): SlashCommand[] {
  if (!query) return SLASH_COMMANDS
  return SLASH_COMMANDS.filter((c) => c.label.toLowerCase().includes(query) || c.id.startsWith(query))
}

/** Replace the `/query` with the command's text and put the caret where it says. */
export function applySlash(text: string, slash: Slash, caret: number, cmd: SlashCommand): Edit {
  const at = cmd.insert.indexOf('|')
  const body = cmd.insert.replace('|', '')
  const next = text.slice(0, slash.start) + body + text.slice(caret)
  const pos = slash.start + (at < 0 ? body.length : at)
  return { text: next, start: pos, end: pos }
}

// ---- formatting ----------------------------------------------------------

export type Format = 'bold' | 'italic' | 'strike' | 'code' | 'quote' | 'bullet' | 'todo' | 'heading'

const WRAP: Partial<Record<Format, string>> = { bold: '**', italic: '*', strike: '~~', code: '`' }
const PREFIX: Partial<Record<Format, string>> = { quote: '> ', bullet: '- ', todo: '- [ ] ', heading: '## ' }

/**
 * Apply `format` to the selection. Inline formats wrap it (or unwrap it if
 * already wrapped); line formats toggle a prefix on every selected line.
 */
export function applyFormat(text: string, start: number, end: number, format: Format): Edit {
  const wrap = WRAP[format]
  if (wrap) {
    const sel = text.slice(start, end)
    const w = wrap.length
    if (text.slice(start - w, start) === wrap && text.slice(end, end + w) === wrap) {
      // Already wrapped around the selection: take the markers off.
      return { text: text.slice(0, start - w) + sel + text.slice(end + w), start: start - w, end: end - w }
    }
    return { text: text.slice(0, start) + wrap + sel + wrap + text.slice(end), start: start + w, end: end + w }
  }
  const prefix = PREFIX[format]!
  const lineStart = text.lastIndexOf('\n', start - 1) + 1
  const lineEndIdx = text.indexOf('\n', end)
  const lineEnd = lineEndIdx < 0 ? text.length : lineEndIdx
  const lines = text.slice(lineStart, lineEnd).split('\n')
  const allHave = lines.every((l) => l.startsWith(prefix))
  const out = lines.map((l) => (allHave ? l.slice(prefix.length) : l.startsWith(prefix) ? l : prefix + l))
  const replaced = out.join('\n')
  return { text: text.slice(0, lineStart) + replaced + text.slice(lineEnd), start: lineStart, end: lineStart + replaced.length }
}

// ---- checklist <-> text ---------------------------------------------------

const MARKER = /^\s*(?:[-*+]\s+(?:\[[ xX]\]\s*)?|\d+[.)]\s+)/

/** Each non-empty line becomes an item; list markers and `[x]` boxes are read and dropped. */
export function notesToItems(notes: string, newId: () => string): ChecklistItem[] {
  return notes
    .split('\n')
    .map((l) => ({ done: /^\s*[-*+]\s+\[[xX]\]/.test(l), title: l.replace(MARKER, '').trim() }))
    .filter((l) => l.title !== '')
    .map((l, i) => ({ id: newId(), title: l.title, done: l.done, sortOrder: i * 1024 }))
}

/** Items as `- [ ] title` lines, in order. */
export function itemsToNotes(items: ChecklistItem[]): string {
  return [...items]
    .sort((a, b) => a.sortOrder - b.sortOrder)
    .map((i) => `- [${i.done ? 'x' : ' '}] ${i.title}`)
    .join('\n')
}

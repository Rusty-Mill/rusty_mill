/**
 * RFC 4180: comma-separated, a field may be quoted, a quote inside a quoted
 * field is doubled, a quoted field may span lines; `\r\n` and `\n` both end
 * a record. Rows come back as arrays of fields; blank lines are skipped.
 */
export function parseCsv(text: string): string[][] {
  const rows: string[][] = []
  let fields: string[] = []
  let field = ''
  let quoted = false
  let pending = false // something on the current row not yet pushed
  for (let i = 0; i < text.length; i++) {
    const c = text[i]!
    if (quoted) {
      if (c === '"' && text[i + 1] === '"') {
        field += '"'
        i++
      } else if (c === '"') quoted = false
      else field += c
      continue
    }
    if (c === '"' && field === '') {
      quoted = true
      pending = true
    } else if (c === ',') {
      fields.push(field)
      field = ''
      pending = true
    } else if (c === '\r' && text[i + 1] === '\n') {
      /* the \n ends the record */
    } else if (c === '\n') {
      if (pending || field !== '') {
        fields.push(field)
        rows.push(fields)
      }
      fields = []
      field = ''
      pending = false
    } else field += c
  }
  if (quoted) throw new Error('unterminated quote')
  if (pending || field !== '') {
    fields.push(field)
    rows.push(fields)
  }
  return rows
}

/** `|`-separated standards; an empty cell is no standards. */
export const parseStandards = (cell: string): string[] =>
  cell
    .split('|')
    .map((s) => s.trim())
    .filter((s) => s !== '')

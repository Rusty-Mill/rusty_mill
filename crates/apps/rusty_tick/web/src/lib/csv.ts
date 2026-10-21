/**
 * Read RFC 4180 CSV into rows of fields. Pure. Quoted fields may hold commas,
 * doubled quotes and line breaks; a leading byte-order mark is dropped; blank
 * lines are skipped. Lenient: a stray quote inside an unquoted field is kept.
 */
export function parseCsv(text: string): string[][] {
  const rows: string[][] = []
  let row: string[] = []
  let field = ''
  let quoted = false
  const endRow = (): void => {
    row.push(field)
    if (row.length > 1 || row[0] !== '') rows.push(row)
    row = []
    field = ''
  }
  const src = text.charCodeAt(0) === 0xfeff ? text.slice(1) : text
  for (let i = 0; i < src.length; i++) {
    const c = src[i]!
    if (quoted) {
      if (c !== '"') field += c
      else if (src[i + 1] === '"') field += src[++i]
      else quoted = false
    } else if (c === '"' && field === '') quoted = true
    else if (c === ',') {
      row.push(field)
      field = ''
    } else if (c === '\n' || c === '\r') {
      if (c === '\r' && src[i + 1] === '\n') i++
      endRow()
    } else field += c
  }
  if (field !== '' || row.length > 0) endRow()
  return rows
}

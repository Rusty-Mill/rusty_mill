/**
 * The editor's content as Markdown, plain text or a standalone HTML file.
 * They read the live DOM (the user may have edited the generated text), so a
 * fixed allow-list decides what survives: anything not on it is reduced to its
 * text, and links keep only safe schemes. That also makes the HTML export safe
 * to open even if odd markup got into the editor.
 */
import { escapeHtml } from './render'

type Format = 'md' | 'txt'

const HEADING = /^H([1-6])$/
const SAFE_HREF = /^(https?:|mailto:)/i

/** A link target worth keeping, or `null`. */
export function safeHref(href: string | null): string | null {
  const h = href?.trim() ?? ''
  return SAFE_HREF.test(h) ? h : null
}

const isChecklist = (el: Element): boolean => el.tagName === 'UL' && el.hasAttribute('data-checklist')

// ---- inline ----------------------------------------------------------

function inline(node: Node, f: Format): string {
  if (node.nodeType === Node.TEXT_NODE) {
    const text = node.textContent ?? ''
    return f === 'md' ? text.replace(/([\\*_`[\]])/g, '\\$1') : text
  }
  if (!(node instanceof Element)) return ''
  const inner = (): string => [...node.childNodes].map((c) => inline(c, f)).join('')
  const tag = node.tagName
  if (tag === 'BR') return f === 'md' ? '  \n' : '\n'
  if (f === 'txt') return inner()
  const text = inner()
  if (text.trim() === '') return text
  switch (tag) {
    case 'B':
    case 'STRONG':
      return `**${text}**`
    case 'I':
    case 'EM':
      return `*${text}*`
    case 'U':
      return `<u>${text}</u>`
    case 'S':
    case 'STRIKE':
    case 'DEL':
      return `~~${text}~~`
    case 'MARK':
      return `==${text}==`
    case 'CODE':
      return `\`${node.textContent ?? ''}\``
    case 'A': {
      const href = safeHref(node.getAttribute('href'))
      return href ? `[${text}](${href})` : text
    }
    default:
      return text
  }
}

// ---- blocks ----------------------------------------------------------

const BLOCKS = new Set(['P', 'DIV', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'UL', 'OL', 'LI', 'BLOCKQUOTE', 'PRE', 'HR'])

/** Blocks of `parent` as text chunks; a chunk is a paragraph, a heading, or a whole list. */
function blocks(parent: Node, f: Format): string[] {
  const out: string[] = []
  let run = '' // consecutive inline nodes make one paragraph
  const flush = (): void => {
    if (run.trim() !== '') out.push(run.trim())
    run = ''
  }
  for (const node of parent.childNodes) {
    if (!(node instanceof Element) || !BLOCKS.has(node.tagName)) {
      run += inline(node, f)
      continue
    }
    flush()
    const tag = node.tagName
    const h = HEADING.exec(tag)
    if (h) {
      const text = inline(node, f).trim()
      if (text) out.push(f === 'md' ? `${'#'.repeat(Number(h[1]))} ${text}` : text)
    } else if (tag === 'HR') {
      out.push(f === 'md' ? '---' : '----------')
    } else if (tag === 'UL' || tag === 'OL') {
      const list = listText(node, f)
      if (list) out.push(list)
    } else if (tag === 'BLOCKQUOTE') {
      const text = blocks(node, f).join('\n\n')
      if (text) out.push(text.split('\n').map((l) => `> ${l}`.trimEnd()).join('\n'))
    } else if (tag === 'PRE') {
      const text = (node.textContent ?? '').replace(/\n+$/, '')
      out.push(f === 'md' ? `\`\`\`\n${text}\n\`\`\`` : text)
    } else if (tag === 'LI') {
      out.push(inline(node, f).trim()) // a stray li outside a list
    } else {
      out.push(...blocks(node, f)) // p, div: their own paragraphs (nested blocks included)
    }
  }
  flush()
  return out.filter((c) => c !== '')
}

function listText(list: Element, f: Format, depth = 0): string {
  const checklist = isChecklist(list)
  const ordered = list.tagName === 'OL'
  const lines: string[] = []
  let n = 0
  for (const li of list.children) {
    if (li.tagName !== 'LI') continue
    n += 1
    const nested = [...li.children].filter((c) => c.tagName === 'UL' || c.tagName === 'OL')
    const own = [...li.childNodes].filter((c) => !nested.includes(c as Element)).map((c) => inline(c, f)).join('').trim()
    const mark = checklist ? (li.getAttribute('data-checked') === 'true' ? '[x] ' : '[ ] ') : ''
    const bullet = ordered ? `${n}. ` : '- '
    lines.push(`${'  '.repeat(depth)}${bullet}${mark}${own}`)
    for (const sub of nested) lines.push(listText(sub, f, depth + 1))
  }
  return lines.join('\n')
}

const render = (root: Element, f: Format): string => blocks(root, f).join('\n\n') + '\n'

export const toMarkdown = (root: Element): string => render(root, 'md')
export const toPlainText = (root: Element): string => render(root, 'txt')

// ---- HTML ------------------------------------------------------------

const KEEP = new Set(['H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'P', 'DIV', 'BR', 'B', 'STRONG', 'I', 'EM', 'U', 'S', 'STRIKE', 'DEL', 'MARK', 'CODE', 'PRE', 'BLOCKQUOTE', 'UL', 'OL', 'LI', 'HR', 'A'])

function safeHtml(node: Node): string {
  if (node.nodeType === Node.TEXT_NODE) return escapeHtml(node.textContent ?? '')
  if (!(node instanceof Element)) return ''
  const tag = node.tagName
  const inner = (): string => [...node.childNodes].map(safeHtml).join('')
  if (!KEEP.has(tag)) return inner() // unknown element (span, font, script…): keep only its text
  const t = tag.toLowerCase()
  if (tag === 'BR' || tag === 'HR') return `<${t}>`
  if (tag === 'A') {
    const href = safeHref(node.getAttribute('href'))
    return href ? `<a href="${escapeHtml(href)}" rel="noopener noreferrer">${inner()}</a>` : inner()
  }
  if (tag === 'UL' && isChecklist(node)) return `<ul class="checklist">${inner()}</ul>`
  if (tag === 'LI' && node.parentElement && isChecklist(node.parentElement)) {
    const done = node.getAttribute('data-checked') === 'true'
    return `<li><input type="checkbox" disabled${done ? ' checked' : ''}> ${inner()}</li>`
  }
  return `<${t}>${inner()}</${t}>`
}

const PAGE_CSS = 'body{font:16px/1.6 system-ui,sans-serif;max-width:720px;margin:2rem auto;padding:0 1rem;color:#191919}ul.checklist{list-style:none;padding-left:0}mark{background:#ffd666}blockquote{margin-left:0;padding-left:1rem;border-left:3px solid #ccc;color:#555}code,pre{background:#f5f5f5;border-radius:4px;padding:0 4px}'

/** A complete, self-contained HTML document of the editor's content. */
export function toHtmlDocument(root: Element, title: string): string {
  const body = [...root.childNodes].map(safeHtml).join('\n')
  return `<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n<title>${escapeHtml(title)}</title>\n<style>${PAGE_CSS}</style>\n</head>\n<body>\n${body}\n</body>\n</html>\n`
}

/** True when the editor holds nothing but whitespace. */
export const isEmptyContent = (root: Element): boolean => (root.textContent ?? '').trim() === '' && root.querySelector('hr') === null

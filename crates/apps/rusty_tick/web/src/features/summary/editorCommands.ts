/**
 * Formatting for the contenteditable. `document.execCommand` is deprecated on
 * paper but it is the only way to get native undo/redo and list handling
 * without a library, and every browser we target still ships it. Where its
 * output is unreliable (highlight, code, quote, checklists) we do the small
 * bit of DOM work ourselves. Each command is a no-op when the selection is
 * outside the editor.
 */
import { escapeHtml } from './render'
import { safeHref } from './exporters'

export const COMMANDS = ['heading', 'bold', 'highlight', 'checklist', 'bullets', 'numbered', 'italic', 'underline', 'strike', 'divider', 'undo', 'redo', 'link', 'code', 'quote'] as const
export type CommandId = (typeof COMMANDS)[number]

const exec = (cmd: string, value?: string): boolean => typeof document.execCommand === 'function' && document.execCommand(cmd, false, value)

/** The nearest ancestor of the selection (inside `root`) matching `selector`. */
function closest(root: HTMLElement, selector: string): HTMLElement | null {
  const node = window.getSelection()?.anchorNode
  if (!node || !root.contains(node)) return null
  const el = node instanceof Element ? node : node.parentElement
  const found = el?.closest<HTMLElement>(selector) ?? null
  return found && root.contains(found) && found !== root ? found : null
}

const selectedText = (root: HTMLElement): string => {
  const sel = window.getSelection()
  return sel && sel.anchorNode && root.contains(sel.anchorNode) ? sel.toString() : ''
}

/** Turn every checklist item into a well-formed one (a state attribute, never missing). */
export function normalizeChecklists(root: HTMLElement): void {
  for (const li of root.querySelectorAll<HTMLElement>('ul[data-checklist] > li')) {
    if (li.getAttribute('data-checked') !== 'true') li.setAttribute('data-checked', 'false')
  }
}

/** Wrap the selection in `<tag>`, or take it out of one it is already in. */
function toggleInline(root: HTMLElement, tag: 'mark' | 'code'): void {
  const inside = closest(root, tag)
  if (inside) {
    const text = inside.textContent ?? ''
    const range = document.createRange()
    range.selectNode(inside)
    const sel = window.getSelection()
    sel?.removeAllRanges()
    sel?.addRange(range)
    if (!exec('insertHTML', escapeHtml(text))) inside.replaceWith(document.createTextNode(text))
    return
  }
  const text = selectedText(root) || (tag === 'code' ? 'code' : '')
  if (text === '') return
  const html = `<${tag}>${escapeHtml(text)}</${tag}>`
  if (!exec('insertHTML', html)) {
    const range = window.getSelection()?.getRangeAt(0)
    if (range) {
      range.deleteContents()
      const holder = document.createElement(tag)
      holder.textContent = text
      range.insertNode(holder)
    }
  }
}

function toggleChecklist(root: HTMLElement): void {
  const list = closest(root, 'ul')
  if (list?.hasAttribute('data-checklist')) {
    exec('insertUnorderedList') // toggles the list off
    return
  }
  if (!list) exec('insertUnorderedList')
  const target = list ?? closest(root, 'ul')
  target?.setAttribute('data-checklist', '')
  normalizeChecklists(root)
}

function toggleBullets(root: HTMLElement): void {
  const list = closest(root, 'ul')
  if (list?.hasAttribute('data-checklist')) list.removeAttribute('data-checklist') // a checklist becomes plain bullets
  else exec('insertUnorderedList')
}

/** Add `https://` (or `mailto:`) when a scheme is missing; `null` for anything that is not a web or mail link. */
export function normalizeUrl(input: string): string | null {
  const text = input.trim()
  if (text === '' || /\s/.test(text)) return null
  if (/^[^\s@:/]+@[^\s@:/]+\.[^\s@:/]+$/.test(text)) return `mailto:${text}`
  const withScheme = /^[a-z][a-z0-9+.-]*:/i.test(text) ? text : `https://${text}`
  return safeHref(withScheme)
}

/** Link the selection (or insert the address itself when nothing is selected). */
function applyLink(root: HTMLElement, url: string): void {
  const href = normalizeUrl(url)
  if (!href) return
  if (selectedText(root) !== '') {
    if (exec('createLink', href)) return
  }
  exec('insertHTML', `<a href="${escapeHtml(href)}">${escapeHtml(url.trim())}</a>`)
}

export function runCommand(root: HTMLElement | null, id: CommandId, arg?: string): void {
  if (!root) return
  root.focus({ preventScroll: true })
  switch (id) {
    case 'bold':
      return void exec('bold')
    case 'italic':
      return void exec('italic')
    case 'underline':
      return void exec('underline')
    case 'strike':
      return void exec('strikeThrough')
    case 'undo':
      return void exec('undo')
    case 'redo':
      return void exec('redo')
    case 'divider':
      return void exec('insertHorizontalRule')
    case 'numbered':
      return void exec('insertOrderedList')
    case 'bullets':
      return toggleBullets(root)
    case 'checklist':
      return toggleChecklist(root)
    case 'highlight':
      return toggleInline(root, 'mark')
    case 'code':
      return toggleInline(root, 'code')
    case 'heading':
      return void exec('formatBlock', closest(root, 'h2') ? 'p' : 'h2')
    case 'quote':
      return void (closest(root, 'blockquote') ? exec('outdent') : exec('formatBlock', 'blockquote'))
    case 'link':
      if (arg) applyLink(root, arg)
  }
}

const queryState = (cmd: string): boolean => {
  try {
    return typeof document.queryCommandState === 'function' && document.queryCommandState(cmd)
  } catch {
    return false // not supported in this browser
  }
}

/** Which toggle buttons should look pressed for the current selection. */
export function activeCommands(root: HTMLElement | null): Set<CommandId> {
  const on = new Set<CommandId>()
  if (!root) return on
  const node = window.getSelection()?.anchorNode
  if (!node || !root.contains(node)) return on
  if (queryState('bold') && !closest(root, 'h1,h2,h3,h4,h5,h6')) on.add('bold') // headings are bold by style, not by choice
  if (queryState('italic')) on.add('italic')
  if (queryState('underline')) on.add('underline')
  if (queryState('strikeThrough')) on.add('strike')
  if (closest(root, 'h1,h2,h3')) on.add('heading')
  if (closest(root, 'mark')) on.add('highlight')
  if (closest(root, 'code')) on.add('code')
  if (closest(root, 'blockquote')) on.add('quote')
  if (closest(root, 'a')) on.add('link')
  if (closest(root, 'ol')) on.add('numbered')
  const ul = closest(root, 'ul')
  if (ul) on.add(ul.hasAttribute('data-checklist') ? 'checklist' : 'bullets')
  return on
}

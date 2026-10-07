import { forwardRef, useCallback, useImperativeHandle, useRef, useState } from 'react'
import { normalizeChecklists } from './editorCommands'
import { isEmptyContent } from './exporters'
import './editor.css'

export interface EditorHandle {
  /** The contenteditable element, for the toolbar and the exporters. */
  root(): HTMLDivElement | null
  /** Replace everything. `undoable` goes through the browser's edit history so Ctrl+Z brings the old text back. */
  setHtml(html: string, undoable?: boolean): void
}

interface Props {
  /** Called after any change the user makes (typing, formatting, ticking a box). */
  onChange?: () => void
}

/**
 * A rich-text area on a bare `contenteditable`. Pasted and dropped content is
 * reduced to plain text, so foreign markup never enters; the only HTML ever
 * assigned to it comes from `setHtml`, which callers build with escaping.
 */
export const RichEditor = forwardRef<EditorHandle, Props>(function RichEditor({ onChange }, ref) {
  const el = useRef<HTMLDivElement>(null)
  const [empty, setEmpty] = useState(true)

  const changed = useCallback((): void => {
    const root = el.current
    if (!root) return
    setEmpty(isEmptyContent(root))
    onChange?.()
  }, [onChange])

  useImperativeHandle(
    ref,
    () => ({
      root: () => el.current,
      setHtml(html, undoable = false) {
        const root = el.current
        if (!root) return
        let done = false
        if (undoable && typeof document.execCommand === 'function') {
          root.focus({ preventScroll: true })
          document.execCommand('selectAll')
          done = document.execCommand('insertHTML', false, html)
        }
        if (!done) root.innerHTML = html
        normalizeChecklists(root)
        changed()
      },
    }),
    [changed],
  )

  const insertText = (text: string): void => {
    if (typeof document.execCommand === 'function' && document.execCommand('insertText', false, text)) return
    const range = window.getSelection()?.getRangeAt(0)
    if (range) {
      range.deleteContents()
      range.insertNode(document.createTextNode(text))
      range.collapse(false)
    }
  }

  return (
    <div className="relative min-h-0 flex-1">
      {empty && (
        <p aria-hidden className="pointer-events-none absolute left-8 top-6 text-base text-grey">
          Pick a template and filters, then press Generate. You can edit the result before saving it.
        </p>
      )}
      <div
        ref={el}
        role="textbox"
        aria-multiline="true"
        aria-label="Summary"
        contentEditable
        suppressContentEditableWarning
        spellCheck
        className="summary-editor h-full overflow-y-auto px-8 py-6 text-base outline-hidden"
        onInput={(e) => {
          const ne = e.nativeEvent as InputEvent
          const root = el.current
          if (root && ne.inputType === 'insertParagraph') {
            // A new item split off a ticked one must start unticked.
            const li = window.getSelection()?.anchorNode
            const item = (li instanceof Element ? li : li?.parentElement)?.closest('ul[data-checklist] > li')
            if (item && (item.textContent ?? '').trim() === '') item.setAttribute('data-checked', 'false')
          }
          if (root) normalizeChecklists(root)
          changed()
        }}
        onPaste={(e) => {
          e.preventDefault()
          insertText(e.clipboardData.getData('text/plain'))
        }}
        onDrop={(e) => {
          e.preventDefault() // dragging markup in is not supported; text can be pasted
          const text = e.dataTransfer.getData('text/plain')
          if (text) insertText(text)
        }}
        onMouseDown={(e) => {
          // The tick box is a pseudo-element on the item's left edge: a press there toggles it.
          const li = e.target instanceof Element ? e.target.closest<HTMLElement>('ul[data-checklist] > li') : null
          if (!li) return
          const box = li.getBoundingClientRect()
          if (e.clientX - box.left > 22 || e.clientY - box.top > 26) return
          e.preventDefault()
          li.setAttribute('data-checked', li.getAttribute('data-checked') === 'true' ? 'false' : 'true')
          changed()
        }}
      />
    </div>
  )
})

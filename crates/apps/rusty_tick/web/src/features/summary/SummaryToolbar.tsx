import { Bold, CheckSquare, Code, Heading2, Highlighter, Italic, Link, List, ListOrdered, Minus, Quote, Redo2, Strikethrough, Underline, Undo2, type LucideIcon } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Popover } from '@/components/Popover'
import { activeCommands, normalizeUrl, runCommand, type CommandId } from './editorCommands'
import type { EditorHandle } from './RichEditor'

interface Btn {
  id: CommandId
  label: string
  icon: LucideIcon
  /** A toggle shows `aria-pressed`; one-shot actions (divider, undo, redo) do not. */
  toggle: boolean
}

// The reference order: H, B, highlight, checkbox, bullets, numbered, I, U, strike, divider, undo/redo, link, code, quote.
const GROUPS: Btn[][] = [
  [
    { id: 'heading', label: 'Heading', icon: Heading2, toggle: true },
    { id: 'bold', label: 'Bold', icon: Bold, toggle: true },
    { id: 'highlight', label: 'Highlight', icon: Highlighter, toggle: true },
  ],
  [
    { id: 'checklist', label: 'Checklist', icon: CheckSquare, toggle: true },
    { id: 'bullets', label: 'Bulleted list', icon: List, toggle: true },
    { id: 'numbered', label: 'Numbered list', icon: ListOrdered, toggle: true },
  ],
  [
    { id: 'italic', label: 'Italic', icon: Italic, toggle: true },
    { id: 'underline', label: 'Underline', icon: Underline, toggle: true },
    { id: 'strike', label: 'Strikethrough', icon: Strikethrough, toggle: true },
    { id: 'divider', label: 'Divider', icon: Minus, toggle: false },
  ],
  [
    { id: 'undo', label: 'Undo', icon: Undo2, toggle: false },
    { id: 'redo', label: 'Redo', icon: Redo2, toggle: false },
  ],
  [
    { id: 'link', label: 'Link', icon: Link, toggle: true },
    { id: 'code', label: 'Code', icon: Code, toggle: true },
    { id: 'quote', label: 'Quote', icon: Quote, toggle: true },
  ],
]

/** The editor's formatting bar. Buttons do not take focus (mousedown is cancelled), so the selection stays put. */
export function SummaryToolbar({ editor }: { editor: React.RefObject<EditorHandle> }) {
  const [active, setActive] = useState<Set<CommandId>>(new Set())
  const [linkOpen, setLinkOpen] = useState(false)
  const [url, setUrl] = useState('')
  const [bad, setBad] = useState(false)
  const linkBtn = useRef<HTMLButtonElement>(null)
  const saved = useRef<Range | null>(null)

  useEffect(() => {
    const update = (): void => setActive(activeCommands(editor.current?.root() ?? null))
    document.addEventListener('selectionchange', update)
    return () => document.removeEventListener('selectionchange', update)
  }, [editor])

  const refresh = (): void => setActive(activeCommands(editor.current?.root() ?? null))

  const openLink = (): void => {
    const root = editor.current?.root()
    const sel = window.getSelection()
    // The address field takes focus and with it the selection, so remember the range now.
    saved.current = root && sel && sel.rangeCount > 0 && root.contains(sel.anchorNode) ? sel.getRangeAt(0).cloneRange() : null
    setUrl('')
    setBad(false)
    setLinkOpen(true)
  }

  const applyLink = (): void => {
    if (!normalizeUrl(url)) return setBad(true)
    const sel = window.getSelection()
    if (saved.current) {
      sel?.removeAllRanges()
      sel?.addRange(saved.current)
    }
    runCommand(editor.current?.root() ?? null, 'link', url)
    setLinkOpen(false)
    refresh()
  }

  return (
    <div role="toolbar" aria-label="Formatting" className="flex flex-wrap items-center gap-x-1 gap-y-0.5 border-b border-line px-4 py-2">
      {GROUPS.map((group, gi) => (
        <div key={gi} className="flex items-center gap-0.5 border-r border-line pr-1 last:border-r-0">
          {group.map((b) => (
            <button
              key={b.id}
              ref={b.id === 'link' ? linkBtn : undefined}
              type="button"
              aria-label={b.label}
              title={b.label}
              aria-pressed={b.toggle ? active.has(b.id) : undefined}
              aria-haspopup={b.id === 'link' ? 'dialog' : undefined}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => {
                if (b.id === 'link') return openLink()
                runCommand(editor.current?.root() ?? null, b.id)
                refresh()
              }}
              className={`flex h-8 w-8 items-center justify-center rounded-row hover:bg-hover ${active.has(b.id) ? 'bg-selected text-primary' : 'text-text'}`}
            >
              <b.icon size={18} strokeWidth={1.5} />
            </button>
          ))}
        </div>
      ))}
      <Popover anchor={linkBtn.current} open={linkOpen} onClose={() => setLinkOpen(false)} placement="bottom-start" role="dialog" ariaLabel="Insert link" className="w-[300px] p-3">
        <form
          onSubmit={(e) => {
            e.preventDefault()
            applyLink()
          }}
          className="flex flex-col gap-2"
        >
          <label className="text-s text-grey" htmlFor="summary-link-url">
            Link address
          </label>
          <input
            id="summary-link-url"
            data-autofocus
            autoFocus
            value={url}
            onChange={(e) => {
              setUrl(e.target.value)
              setBad(false)
            }}
            placeholder="https://example.com"
            aria-invalid={bad}
            className="h-8 rounded-row border border-line bg-surface px-2 text-base outline-hidden focus:border-primary"
          />
          {bad && (
            <p role="alert" className="text-s text-danger">
              Enter a web or email address.
            </p>
          )}
          <div className="flex justify-end">
            <button type="submit" className="h-8 rounded-row bg-primary px-4 text-white">
              Add link
            </button>
          </div>
        </form>
      </Popover>
    </div>
  )
}

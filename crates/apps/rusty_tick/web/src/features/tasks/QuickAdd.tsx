import { Plus } from 'lucide-react'
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import type { List, Task } from '@/api/types'
import { useActions } from '@/app/services'
import { parseQuickAdd, type Token } from '@/lib/nlp/parse'
import { usePrefs } from '../settings/prefs'
import { useUi } from '@/store/ui'
import { buildQuickAdd, defaultListId } from './quickAddParse'
import type { ViewSpec } from './organize'

interface Props {
  view: ViewSpec
  lists: List[]
  inboxId: string
  tasks: Task[]
  now: number
}

/** Tokens the parser recognised, drawn behind the (transparent) input text so they show as highlights. */
function Highlighted({ text, tokens }: { text: string; tokens: Token[] }) {
  const parts: React.ReactNode[] = []
  let at = 0
  tokens.forEach((t, i) => {
    if (t.start > at) parts.push(text.slice(at, t.start))
    parts.push(
      <mark key={i} className="rounded bg-primary/15 text-primary">
        {text.slice(t.start, t.end)}
      </mark>,
    )
    at = t.end
  })
  parts.push(text.slice(at))
  return <>{parts}</>
}

/** The 38px "Add task to …" box. Enter adds; date, priority, `#tag` and `~list` are read from the text. */
export function QuickAdd({ view, lists, inboxId, tasks, now }: Props) {
  const [text, setText] = useState('')
  const [invalid, setInvalid] = useState(false)
  const input = useRef<HTMLInputElement>(null)
  const under = useRef<HTMLDivElement>(null)
  const actions = useActions()
  const defaultReminder = usePrefs((s) => s.prefs.defaultReminder)
  const focusTick = useUi((s) => s.quickAddFocus)

  useEffect(() => {
    if (focusTick > 0) input.current?.focus()
  }, [focusTick])

  const target = lists.find((l) => l.id === defaultListId(view, inboxId))
  const parsed = useMemo(() => parseQuickAdd(text, now), [text, now])

  const submit = (): void => {
    const result = buildQuickAdd(text, {
      now: Date.now(),
      lists,
      inboxId,
      view,
      siblings: (listId) => tasks.filter((t) => t.listId === listId),
      defaultReminder,
    })
    if (!result.ok) {
      setInvalid(true) // nothing left for a title
      return
    }
    setText('')
    setInvalid(false)
    void actions.createTask(result.input).catch((e: unknown) => actions.notify('error', e instanceof Error ? e.message : String(e)))
  }

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>): void => {
    if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
      e.preventDefault()
      submit()
    } else if (e.key === 'Escape') {
      setText('')
      input.current?.blur()
    }
  }

  return (
    <div className={`relative mx-4 mb-2 flex h-[38px] items-center rounded-row bg-black/[.04] focus-within:bg-surface focus-within:ring-1 ${invalid ? 'ring-danger' : 'focus-within:ring-primary/50'}`}>
      <Plus size={18} className="pointer-events-none absolute left-3 text-grey" aria-hidden />
      <div ref={under} aria-hidden className="pointer-events-none absolute inset-y-0 left-10 right-3 flex items-center overflow-hidden whitespace-pre text-base">
        <span>
          <Highlighted text={text} tokens={parsed.tokens} />
        </span>
      </div>
      <input
        ref={input}
        aria-label="Add task"
        value={text}
        onChange={(e) => {
          setText(e.target.value)
          setInvalid(false)
        }}
        onScroll={(e) => {
          if (under.current) under.current.scrollLeft = e.currentTarget.scrollLeft
        }}
        onKeyDown={onKeyDown}
        placeholder={`Add task to "${target?.name ?? 'Inbox'}"`}
        autoComplete="off"
        spellCheck={false}
        // The typed text is drawn by the layer behind (so tokens can be highlighted); the caret stays visible.
        className="h-full w-full bg-transparent pl-10 pr-3 text-base text-transparent caret-[rgb(var(--text))] outline-hidden placeholder:text-grey"
      />
    </div>
  )
}

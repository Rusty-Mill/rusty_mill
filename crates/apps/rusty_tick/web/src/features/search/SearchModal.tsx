import { Search, X } from 'lucide-react'
import { useEffect, useId, useMemo, useRef, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { PATHS, taskPath, viewPath } from '@/app/paths'
import { useData } from '@/app/services'
import { Dialog } from '@/components/Dialog'
import { useUi } from '@/store/ui'
import { highlight, searchLists, searchTasks, type SearchMode } from './search'

function Marked({ text, query }: { text: string; query: string }) {
  return (
    <>
      {highlight(text, query).map((s, i) =>
        s.hit ? (
          <mark key={i} className="rounded-sm bg-mark text-text">
            {s.text}
          </mark>
        ) : (
          s.text
        ),
      )}
    </>
  )
}

/** Ctrl/⌘+K: find a task or a list. Arrow keys move, Enter opens. */
export function SearchModal() {
  const open = useUi((s) => s.searchOpen)
  const setOpen = useUi((s) => s.setSearchOpen)
  return (
    <Dialog open={open} onClose={() => setOpen(false)} width={560} labelledBy="search-title" bare>
      <h2 id="search-title" className="sr-only">
        Search
      </h2>
      {open && <SearchBody onDone={() => setOpen(false)} />}
    </Dialog>
  )
}

function SearchBody({ onDone }: { onDone: () => void }) {
  const navigate = useNavigate()
  const inboxId = useData((s) => s.inboxId)
  const taskMap = useData((s) => s.tasks)
  const listMap = useData((s) => s.lists)
  const [query, setQuery] = useState('')
  const [mode, setMode] = useState<SearchMode>('task')
  const [active, setActive] = useState(0)
  const listId = useId()
  const box = useRef<HTMLUListElement>(null)

  const tasks = useMemo(() => Object.values(taskMap), [taskMap])
  const lists = useMemo(() => Object.values(listMap), [listMap])
  const taskHits = useMemo(() => (mode === 'task' ? searchTasks(query, tasks) : []), [mode, query, tasks])
  const listHits = useMemo(() => (mode === 'list' ? searchLists(query, lists) : []), [mode, query, lists])
  const count = mode === 'task' ? taskHits.length : listHits.length
  const activeIndex = Math.min(active, Math.max(count - 1, 0))

  useEffect(() => setActive(0), [query, mode])
  useEffect(() => {
    box.current?.querySelector('[aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest' })
  }, [activeIndex])

  const openTask = (i: number): void => {
    const hit = taskHits[i]
    if (!hit) return
    const t = hit.task
    onDone()
    if (t.status === 'done') navigate(`${PATHS.completed}/${t.id}`)
    else navigate(taskPath(t.listId === inboxId ? { kind: 'inbox' } : { kind: 'list', id: t.listId }, t.id))
  }
  const openList = (i: number): void => {
    const l = listHits[i]
    if (!l) return
    onDone()
    navigate(viewPath(l.id === inboxId ? { kind: 'inbox' } : { kind: 'list', id: l.id }))
  }
  const choose = (i: number): void => (mode === 'task' ? openTask(i) : openList(i))

  const onKeyDown = (e: React.KeyboardEvent): void => {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      if (count > 0) setActive((a) => (Math.min(a, count - 1) + (e.key === 'ArrowDown' ? 1 : -1) + count) % count)
    } else if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
      e.preventDefault()
      choose(activeIndex)
    }
  }

  return (
    <div className="flex max-h-[70vh] flex-col" onKeyDown={onKeyDown}>
      <div className="flex h-14 shrink-0 items-center gap-3 border-b border-line px-4">
        <Search size={18} className="text-grey" aria-hidden />
        <input
          data-autofocus
          role="combobox"
          aria-expanded="true"
          aria-controls={listId}
          aria-activedescendant={count > 0 ? `${listId}-${activeIndex}` : undefined}
          aria-label="Search"
          autoComplete="off"
          spellCheck={false}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={mode === 'task' ? 'Search tasks' : 'Search lists'}
          className="h-full min-w-0 flex-1 bg-transparent text-[16px] outline-none placeholder:text-grey"
        />
        {query && (
          <button type="button" aria-label="Clear search" onClick={() => setQuery('')} className="flex h-6 w-6 items-center justify-center rounded-full text-grey hover:bg-hover">
            <X size={14} />
          </button>
        )}
        <button type="button" aria-label="Close search" onClick={onDone} className="flex h-7 w-7 items-center justify-center rounded-row text-grey hover:bg-hover">
          <X size={18} />
        </button>
      </div>

      <div role="radiogroup" aria-label="Search in" className="flex shrink-0 gap-2 px-4 py-2">
        {(['task', 'list'] as const).map((m) => (
          <button key={m} type="button" role="radio" aria-checked={mode === m} onClick={() => setMode(m)} className={`h-7 rounded-full px-3 text-base ${mode === m ? 'bg-primary/10 font-semibold text-primary' : 'bg-black/[.05] text-grey hover:text-text'}`}>
            {m === 'task' ? 'Task' : 'List'}
          </button>
        ))}
      </div>

      <ul ref={box} id={listId} role="listbox" aria-label="Results" className="scroll-thin min-h-[120px] flex-1 overflow-y-auto px-2 pb-3">
        {count === 0 && <li role="presentation" className="px-3 py-8 text-center text-grey">{query.trim() ? 'No results' : mode === 'task' ? 'Nothing here yet' : 'No lists'}</li>}
        {mode === 'task' &&
          taskHits.map(({ task, snippet }, i) => (
            <li
              key={task.id}
              id={`${listId}-${i}`}
              role="option"
              aria-selected={i === activeIndex}
              onMouseMove={() => setActive(i)}
              onClick={() => openTask(i)}
              className={`flex cursor-pointer items-center gap-3 rounded-row px-3 py-2 ${i === activeIndex ? 'bg-selected' : ''}`}
            >
              <div className="min-w-0 flex-1">
                <p className={`truncate ${task.status === 'done' ? 'text-grey line-through' : ''}`}>
                  <Marked text={task.title} query={query} />
                </p>
                {snippet && (
                  <p className="truncate text-s text-grey">
                    <Marked text={snippet} query={query} />
                  </p>
                )}
              </div>
              <span className="shrink-0 text-s text-grey">{listMap[task.listId]?.name}</span>
            </li>
          ))}
        {mode === 'list' &&
          listHits.map((l, i) => (
            <li key={l.id} id={`${listId}-${i}`} role="option" aria-selected={i === activeIndex} onMouseMove={() => setActive(i)} onClick={() => openList(i)} className={`flex cursor-pointer items-center gap-3 rounded-row px-3 py-2 ${i === activeIndex ? 'bg-selected' : ''}`}>
              <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: l.color ?? 'rgb(var(--grey))' }} />
              <span className="min-w-0 flex-1 truncate">
                <Marked text={l.name} query={query} />
              </span>
            </li>
          ))}
      </ul>
    </div>
  )
}

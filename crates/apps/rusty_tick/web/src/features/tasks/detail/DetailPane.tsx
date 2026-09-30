import { ArrowLeft, Calendar, Copy, ListChecks, MoreHorizontal, Plus, RotateCcw, Trash2, Type, X, Link2 } from 'lucide-react'
import { useMemo, useRef, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import type { Task } from '@/api/types'
import { useActions, useData } from '@/app/services'
import { DetailArt } from '@/components/Illustrations'
import { Menu } from '@/components/Menu'
import { TaskCheck } from '@/components/TaskCheck'
import { formatDueLong, monthName, formatTime } from '@/lib/date'
import { newId } from '@/lib/id'
import { useNow } from '@/lib/hooks'
import { useDraft } from '@/lib/useDraft'
import { describeRule } from '@/lib/recurrence'
import { usePrefs } from '../../settings/prefs'
import { fieldsOf, type DateFields } from '../dateSelection'
import { useTaskActions } from '../useTaskActions'
import { Checklist } from './Checklist'
import { DatePopover } from './DatePopover'
import { itemsToNotes, notesToItems, type Format } from './editing'
import { NotesEditor, type NotesHandle } from './NotesEditor'
import { PriorityMenu } from './PriorityMenu'
import { TagPicker } from './TagPicker'
import { Popover } from '@/components/Popover'

const shell = 'flex w-[500px] shrink-0 flex-col border-l border-line bg-surface max-[1279px]:w-[400px] max-[999px]:absolute max-[999px]:inset-0 max-[999px]:z-20 max-[999px]:w-full max-[999px]:border-l-0'

/** Where the pane sends you: back to the list it was opened from, or on to another task. */
export interface PanePaths {
  list: string
  task: (id: string) => string
}

export function DetailPane({ paths, taskId }: { paths: PanePaths; taskId: string | null }) {
  const task = useData((s) => (taskId ? s.tasks[taskId] : undefined))
  if (!taskId) {
    return (
      <aside aria-label="Task details" className={`${shell} items-center justify-center max-[999px]:hidden`}>
        <DetailArt />
      </aside>
    )
  }
  if (!task) {
    return (
      <aside aria-label="Task details" className={`${shell} items-center justify-center gap-2 text-grey`}>
        <p>This task no longer exists.</p>
        <Link to={paths.list} className="text-primary underline">
          Back to the list
        </Link>
      </aside>
    )
  }
  // Keyed by id so every field starts from the task being opened.
  return <TaskDetail key={task.id} task={task} paths={paths} />
}

const FORMATS: { kind: Format; label: string }[] = [
  { kind: 'bold', label: 'Bold' },
  { kind: 'italic', label: 'Italic' },
  { kind: 'strike', label: 'Strikethrough' },
  { kind: 'code', label: 'Code' },
  { kind: 'heading', label: 'Heading' },
  { kind: 'quote', label: 'Quote' },
  { kind: 'bullet', label: 'Bulleted list' },
  { kind: 'todo', label: 'Checkbox' },
]

function TaskDetail({ task, paths }: { task: Task; paths: PanePaths }) {
  const navigate = useNavigate()
  const actions = useActions()
  const taskActions = useTaskActions()
  const now = useNow(30_000)
  const hour12 = usePrefs((s) => s.prefs.hour12)
  const lists = useData((s) => s.lists)
  const tagMap = useData((s) => s.tags)
  const tags = useMemo(() => Object.values(tagMap).sort((a, b) => a.sortOrder - b.sortOrder), [tagMap])
  const listArr = useMemo(() => Object.values(lists).sort((a, b) => a.sortOrder - b.sortOrder), [lists])
  const trashed = task.deletedMs !== null
  const fail = (e: unknown): void => actions.notify('error', e instanceof Error ? e.message : String(e))
  const update = (patch: Parameters<typeof actions.updateTask>[1]): void => void actions.updateTask(task.id, patch).catch(fail)

  const [dateOpen, setDateOpen] = useState(false)
  const [tagsOpen, setTagsOpen] = useState(false)
  const [fmtOpen, setFmtOpen] = useState(false)
  const [moreOpen, setMoreOpen] = useState(false)
  const [listOpen, setListOpen] = useState(false)
  const dateBtn = useRef<HTMLButtonElement>(null)
  const tagBtn = useRef<HTMLButtonElement>(null)
  const fmtBtn = useRef<HTMLButtonElement>(null)
  const moreBtn = useRef<HTMLButtonElement>(null)
  const listBtn = useRef<HTMLButtonElement>(null)
  const notes = useRef<NotesHandle>(null)

  const titleDraft = useDraft(task.id, task.title, (v) => {
    const t = v.trim()
    if (t && t !== task.title) update({ title: t })
  }, 600)

  const due = formatDueLong(task, now, hour12)
  const overdue = task.dueMs !== null && !task.isAllDay ? task.dueMs < now : task.dueMs !== null && task.dueMs < now - 86_400_000 + 1
  const list = lists[task.listId]
  const checklistMode = task.kind === 'checklist'
  const showChecklist = checklistMode || task.items.length > 0

  const applyDates = (f: DateFields): void => update({ dueMs: f.dueMs, startMs: f.startMs, isAllDay: f.isAllDay, reminders: f.reminders, repeatFlag: f.repeatFlag })

  const toggleMode = (): void => {
    if (trashed) return
    if (checklistMode) update({ kind: 'text', notes: [task.notes, itemsToNotes(task.items)].filter(Boolean).join('\n'), items: [] })
    else update({ kind: 'checklist', items: [...task.items, ...notesToItems(task.notes, () => newId())], notes: '' })
  }

  const dateRange = (): string | null => {
    if (task.startMs === null) return due
    const s = new Date(task.startMs)
    const start = `${monthName(s.getMonth())} ${s.getDate()}${task.isAllDay ? '' : ` ${formatTime(task.startMs, hour12)}`}`
    return `${start} – ${due ?? ''}`
  }

  return (
    <aside aria-label="Task details" className={shell}>
      {trashed && (
        <div role="status" className="flex items-center gap-2 bg-black/[.04] px-4 py-2 text-base">
          <Trash2 size={16} className="text-grey" />
          <span className="flex-1">This task is in the Trash.</span>
          <button type="button" onClick={() => taskActions.restore(task.id)} className="flex items-center gap-1 rounded-row px-2 py-1 text-primary hover:bg-hover">
            <RotateCcw size={14} /> Restore
          </button>
        </div>
      )}

      <div className="flex h-14 shrink-0 items-center gap-2 px-4">
        <button type="button" aria-label="Back to the list" onClick={() => navigate(paths.list)} className="hidden h-8 w-8 items-center justify-center rounded-row text-grey hover:bg-hover max-[999px]:flex">
          <ArrowLeft size={18} />
        </button>
        <TaskCheck checked={task.status === 'done'} priority={task.priority} label={`Complete: ${task.title}`} onChange={() => taskActions.toggle(task.id)} size={18} />
        <button
          ref={dateBtn}
          type="button"
          disabled={trashed}
          aria-haspopup="dialog"
          aria-expanded={dateOpen}
          onClick={() => setDateOpen((o) => !o)}
          className={`flex h-8 min-w-0 items-center gap-1.5 rounded-row px-2 hover:bg-hover disabled:opacity-60 ${due ? (overdue ? 'text-danger' : 'text-primary') : 'text-grey'}`}
        >
          <Calendar size={16} className="shrink-0" />
          <span className="truncate">{due ? dateRange() : 'Due Date'}</span>
        </button>
        {task.repeatFlag && <span className="truncate text-s text-grey" title="Repeats">{describeRule(task.repeatFlag)}</span>}
        <span className="flex-1" />
        <PriorityMenu value={task.priority} onChange={(p) => taskActions.setPriority(task.id, p)} disabled={trashed} />
      </div>
      <DatePopover anchor={dateBtn.current} open={dateOpen} onClose={() => setDateOpen(false)} fields={fieldsOf(task)} now={now} onApply={applyDates} />

      <div className="scroll-thin flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-4 pb-4">
        <textarea
          aria-label="Title"
          rows={1}
          value={titleDraft.value}
          disabled={trashed}
          maxLength={500}
          onChange={(e) => titleDraft.set(e.target.value.replace(/\n/g, ' '))}
          onBlur={() => {
            titleDraft.flush()
            if (titleDraft.value.trim() === '') titleDraft.set(task.title) // an empty title is not allowed: put it back
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
              e.preventDefault()
              titleDraft.flush()
              notes.current?.focus()
            }
          }}
          ref={(el) => {
            if (el) {
              el.style.height = 'auto'
              el.style.height = `${el.scrollHeight}px`
            }
          }}
          className={`w-full resize-none bg-transparent text-h1 font-semibold outline-none ${task.status === 'done' ? 'text-grey line-through' : ''}`}
        />

        <div className="relative flex-1">
          <button
            type="button"
            aria-label={checklistMode ? 'Switch to text' : 'Switch to checklist'}
            aria-pressed={checklistMode}
            title={checklistMode ? 'Switch to text' : 'Switch to checklist'}
            disabled={trashed}
            onClick={toggleMode}
            className="absolute -top-1 right-0 z-10 flex h-7 w-7 items-center justify-center rounded-row text-grey hover:bg-hover disabled:opacity-40"
          >
            {checklistMode ? <Type size={16} /> : <ListChecks size={16} />}
          </button>
          {checklistMode ? null : <NotesEditor ref={notes} taskId={task.id} notes={task.notes} disabled={trashed} onSave={(v) => v !== task.notes && update({ notes: v })} />}
          {showChecklist && (
            <div className={checklistMode ? '' : 'mt-3 border-t border-line pt-3'}>
              <Checklist taskId={task.id} items={task.items} disabled={trashed} onChange={(items) => update({ items })} />
            </div>
          )}
        </div>

        <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="Tags">
          {task.tags.map((name) => {
            const t = tagMap[name]
            return (
              <span key={name} className="flex h-6 items-center gap-1 rounded-full pl-2 pr-1 text-s" style={{ color: t?.color ?? 'rgb(var(--grey))', backgroundColor: `${t?.color ?? '#a8a8a8'}1f` }}>
                {t?.label ?? name}
                {!trashed && (
                  <button type="button" aria-label={`Remove tag ${t?.label ?? name}`} onClick={() => update({ tags: task.tags.filter((n) => n !== name) })} className="flex h-4 w-4 items-center justify-center rounded-full hover:bg-black/10">
                    <X size={11} />
                  </button>
                )}
              </span>
            )
          })}
          {!trashed && (
            <button ref={tagBtn} type="button" aria-label="Add tag" aria-haspopup="dialog" onClick={() => setTagsOpen((o) => !o)} className="flex h-6 w-6 items-center justify-center rounded-full text-grey hover:bg-hover">
              <Plus size={14} />
            </button>
          )}
        </div>
        <TagPicker
          anchor={tagBtn.current}
          open={tagsOpen}
          onClose={() => setTagsOpen(false)}
          tags={tags}
          selected={task.tags}
          onToggle={(name) => update({ tags: task.tags.includes(name) ? task.tags.filter((n) => n !== name) : [...task.tags, name] })}
          onCreate={(label) => update({ tags: [...task.tags, label] })}
        />
      </div>

      <footer className="flex h-12 shrink-0 items-center gap-1 border-t border-line px-3">
        <button ref={listBtn} type="button" disabled={trashed} aria-haspopup="menu" aria-expanded={listOpen} onClick={() => setListOpen((o) => !o)} className="flex h-8 min-w-0 max-w-[200px] items-center gap-2 rounded-row px-2 hover:bg-hover disabled:opacity-60">
          <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: list?.color ?? 'rgb(var(--grey))' }} />
          <span className="truncate">{list?.name ?? 'No list'}</span>
        </button>
        <Menu
          anchor={listBtn.current}
          open={listOpen}
          onClose={() => setListOpen(false)}
          label="Move to list"
          placement="top-start"
          items={listArr.filter((l) => !l.archived || l.id === task.listId).map((l) => ({ id: l.id, label: l.name, checked: l.id === task.listId, onSelect: () => taskActions.moveTo(task.id, l.id) }))}
        />
        <span className="flex-1" />
        <button ref={fmtBtn} type="button" aria-label="Formatting" aria-haspopup="dialog" disabled={trashed || checklistMode} onClick={() => setFmtOpen((o) => !o)} className="flex h-8 w-8 items-center justify-center rounded-row font-semibold text-grey hover:bg-hover disabled:opacity-40">
          A
        </button>
        <Popover anchor={fmtBtn.current} open={fmtOpen} onClose={() => setFmtOpen(false)} placement="top-end" ariaLabel="Formatting" role="dialog" className="p-1.5" restoreFocus={false}>
          <div className="grid grid-cols-4 gap-1">
            {FORMATS.map((f) => (
              <button key={f.kind} type="button" title={f.label} aria-label={f.label} onMouseDown={(e) => e.preventDefault()} onClick={() => { notes.current?.format(f.kind); setFmtOpen(false) }} className="flex h-8 w-14 items-center justify-center rounded-row text-s hover:bg-hover">
                {f.label.split(' ')[0]}
              </button>
            ))}
          </div>
        </Popover>
        <button ref={moreBtn} type="button" aria-label="More" aria-haspopup="menu" aria-expanded={moreOpen} onClick={() => setMoreOpen((o) => !o)} className="flex h-8 w-8 items-center justify-center rounded-row text-grey hover:bg-hover">
          <MoreHorizontal size={18} />
        </button>
        <Menu
          anchor={moreBtn.current}
          open={moreOpen}
          onClose={() => setMoreOpen(false)}
          label="Task options"
          placement="top-end"
          items={[
            { id: 'dup', label: 'Duplicate', icon: <Copy size={16} />, disabled: trashed, onSelect: () => void taskActions.duplicate(task).then((t) => t && navigate(paths.task(t.id))) },
            { id: 'link', label: 'Copy link', icon: <Link2 size={16} />, onSelect: () => void navigator.clipboard?.writeText(`${location.origin}${location.pathname}#${paths.task(task.id)}`).then(() => actions.notify('info', 'Link copied')) },
            'separator',
            trashed
              ? { id: 'purge', label: 'Delete forever', icon: <Trash2 size={16} />, danger: true, onSelect: () => { taskActions.purge(task.id); navigate(paths.list) } }
              : { id: 'delete', label: 'Move to Trash', icon: <Trash2 size={16} />, danger: true, onSelect: () => { taskActions.remove(task.id); navigate(paths.list) } },
            'separator',
            { id: 'created', label: `Created ${new Date(task.createdMs).toLocaleDateString()}`, disabled: true },
          ]}
        />
      </footer>
    </aside>
  )
}

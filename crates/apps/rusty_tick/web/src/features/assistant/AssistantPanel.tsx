/**
 * The assistant: a chat panel over `rusty_tick`'s `POST /api/agent`, the
 * first consumer of `@rusty-mill/agui-react`. The agent is deterministic and
 * store-free; this panel gives it its tools (`useAction`, see `tools.ts`),
 * run here against the same store the rest of the UI uses.
 */
import { Sparkles, X } from 'lucide-react'
import { useEffect, useMemo, useState, type FormEvent } from 'react'
import { AgentProvider, useAction, useAgent } from '@rusty-mill/agui-react'
import { contentText, type Message } from '@rusty-mill/agui-core'
import { getToken } from '@/app/env'
import { useActions, useData, useServices } from '@/app/services'
import { useView } from '@/app/useView'
import { assigneeMap } from '@/features/assignees/logic'
import { useAssignees } from '@/features/assignees/store'
import { useFilters } from '@/features/filters/store'
import { useHabits } from '@/features/habits/store'
import { useUi } from '@/store/ui'
import { habitTools } from './habitTools'
import { taskTools } from './tools'

export const AGENT_URL = '/api/agent'

/** Adds the bearer token at request time, so a token entered later is used. */
const authedFetch: typeof fetch = (url, init) => {
  const headers = new Headers(init?.headers)
  const token = getToken()
  if (token) headers.set('Authorization', `Bearer ${token}`)
  return fetch(url, { ...init, headers })
}

/** Mounted once in the shell; shows nothing until opened from the rail. */
export function AssistantPanel({ fetchImpl = authedFetch }: { fetchImpl?: typeof fetch }) {
  const open = useUi((s) => s.assistantOpen)
  const endpoint = useMemo(() => ({ url: AGENT_URL, fetch: fetchImpl }), [fetchImpl])
  if (!open) return null
  return (
    <AgentProvider endpoint={endpoint}>
      <Panel />
    </AgentProvider>
  )
}

function Panel() {
  const close = useUi((s) => s.closeAssistant)
  const { mode } = useServices()
  const { messages, running, error, send } = useAgent()
  const [draft, setDraft] = useState('')
  useTaskTools()
  useHabitTools()

  const submit = (e: FormEvent): void => {
    e.preventDefault()
    const text = draft.trim()
    if (!text || running) return
    setDraft('')
    void send(text)
  }

  return (
    <aside aria-label="Assistant" className="pop-shadow fixed bottom-4 right-4 z-[60] flex h-[420px] w-[340px] flex-col rounded-menu bg-surface text-base">
      <header className="flex items-center gap-2 border-b border-line px-3 py-2">
        <Sparkles size={16} className="text-primary" />
        <span className="font-semibold">Assistant</span>
        <button type="button" aria-label="Close assistant" onClick={close} className="ml-auto rounded p-0.5 hover:bg-hover">
          <X size={16} />
        </button>
      </header>
      <ol aria-label="Conversation" className="flex flex-1 flex-col gap-2 overflow-y-auto px-3 py-2">
        {messages.length === 0 && <li className="text-s text-grey">{mode === 'demo' ? 'The assistant needs the server; sample data has none.' : 'Try “add buy milk”, “what’s due today?”, “find milk” or “complete buy milk”'}</li>}
        {messages.map((m) => (
          <Bubble key={m.id} message={m} />
        ))}
        {running && <li className="text-s text-grey">…</li>}
        {error && (
          <li role="alert" className="text-s text-danger">
            {error}
          </li>
        )}
      </ol>
      <form onSubmit={submit} className="border-t border-line p-2">
        <input
          aria-label="Message the assistant"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder="Ask the assistant"
          disabled={running}
          className="h-9 w-full rounded-row border border-line bg-surface px-3 outline-hidden focus:border-primary"
        />
      </form>
    </aside>
  )
}

function Bubble({ message }: { message: Message }) {
  if (message.role === 'tool') return null
  if (message.role !== 'user' && message.role !== 'assistant') return null
  const text = contentText(message.content)
  if (!text) return null
  const mine = message.role === 'user'
  return (
    <li className={`max-w-[85%] whitespace-pre-wrap rounded-menu px-3 py-1.5 ${mine ? 'self-end bg-primary text-white' : 'self-start bg-hover'}`}>{text}</li>
  )
}

/** The agent's tools (`tools.ts`), bound to the store and the list that is open. */
function useTaskTools(): void {
  const { spec } = useView()
  const tasks = useData((s) => s.tasks)
  const lists = useData((s) => s.lists)
  const tags = useData((s) => s.tags)
  const inboxId = useData((s) => s.inboxId)
  const filters = useFilters((s) => s.items)
  const assigneeItems = useAssignees((s) => s.items)
  const actions = useActions()
  const [add, search, due, byPriority, complete, reopen, update, remove, listLists] = taskTools({
    entities: { tasks: Object.values(tasks), lists: Object.values(lists), tags: Object.values(tags), inboxId, filters, assignees: assigneeMap(assigneeItems) },
    listId: spec?.kind === 'list' ? spec.id : inboxId,
    now: Date.now(),
    createTask: actions.createTask,
    toggleDone: actions.toggleDone,
    reopen: (id) => actions.updateTask(id, { status: 'open' }),
    trash: actions.trashTask,
    update: actions.updateTask,
    move: actions.moveTask,
  })
  useAction(add!)
  useAction(search!)
  useAction(due!)
  useAction(byPriority!)
  useAction(complete!)
  useAction(reopen!)
  useAction(update!)
  useAction(remove!)
  useAction(listLists!)
}

/** The habit tools (`habitTools.ts`), bound to the habits store, loaded as the habits page loads it. */
function useHabitTools(): void {
  const { api } = useServices()
  const { notify } = useActions()
  const habits = useHabits((s) => s.habits)
  const checkins = useHabits((s) => s.checkins)
  useEffect(() => {
    void useHabits.getState().load(api, notify)
  }, [api, notify])
  const [list, checkIn, add] = habitTools({ habits, checkins, now: Date.now(), add: useHabits.getState().add, setCount: useHabits.getState().setCount })
  useAction(list!)
  useAction(checkIn!)
  useAction(add!)
}

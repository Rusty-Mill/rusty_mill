/**
 * The assistant's tools, as the browser runs them: named and shaped as the
 * MCP server's (`src/mcp.rs`), but given a title where MCP takes an id, since
 * a person names a task by what it says. Each answers with plain JSON the
 * agent turns into a reply; a task named ambiguously is not guessed at.
 */
import type { ActionDefinition } from '@rusty-mill/agui-core'
import type { NewTask, Task } from '@/api/types'
import { searchTasks } from '@/features/search/search'
import { tasksForView, type Entities } from '@/features/tasks/organize'
import { startOfDay } from '@/lib/date'

/** Most tasks one answer lists. */
export const MAX_SHOWN = 20

type Answer = Record<string, string | string[]>

export interface ToolContext {
  /** Everything the store holds, trash included, for the views the tools read. */
  entities: Entities
  /** Where `add_task` adds: the list the person has open, else the Inbox. */
  listId: string
  now: number
  createTask(input: Omit<NewTask, 'id'>): Promise<Task>
  toggleDone(id: string): Promise<void>
  reopen(id: string): Promise<void>
  trash(id: string): Promise<void>
}

const norm = (text: string): string => text.trim().toLowerCase()

/** One line per task: its title and, if it has one, its due date. */
export const line = (t: Task): string => (t.dueMs === null ? t.title : `${t.title} (due ${new Date(t.dueMs).toLocaleDateString()})`)

export type Pick = { task: Task } | { matches: string[] } | { error: string }

/**
 * The one task among `tasks` that `query` names: its exact title, else the
 * only title containing it. A blank query names none (every title contains it).
 */
export function pickTask(tasks: readonly Task[], query: string, what: string): Pick {
  const q = norm(query)
  if (q === '') return { error: 'Say which task.' }
  const exact = tasks.filter((t) => norm(t.title) === q)
  const found = exact.length > 0 ? exact : tasks.filter((t) => norm(t.title).includes(q))
  if (found.length === 1) return { task: found[0]! }
  if (found.length === 0) return { error: `No ${what} matches “${query.trim()}”.` }
  return { matches: found.slice(0, 5).map((t) => t.title) }
}

/** Open tasks due before today. */
export const overdue = (tasks: readonly Task[], now: number): Task[] =>
  tasks.filter((t) => t.deletedMs === null && t.status === 'open' && t.dueMs !== null && t.dueMs < startOfDay(now))

const titled = { type: 'object', properties: { query: { type: 'string', description: 'The task’s title, or enough of it' } }, required: ['query'] }

/** Run `act` on the one task `query` names among `tasks`, answering with its title. */
async function onOne(tasks: readonly Task[], query: string, what: string, act: (t: Task) => Promise<void>): Promise<Answer> {
  const pick = pickTask(tasks, query, what)
  if (!('task' in pick)) return pick
  await act(pick.task)
  return { title: pick.task.title }
}

/** The tools, bound to `c`; register each with `useAction`. */
export function taskTools(c: ToolContext): ActionDefinition<{ [key: string]: string }, Answer>[] {
  const live = c.entities.tasks.filter((t) => t.deletedMs === null)
  const lines = (tasks: Task[]): Answer => ({ tasks: tasks.slice(0, MAX_SHOWN).map(line) })
  return [
    {
      name: 'add_task',
      description: 'Add a task with this title to the list the person has open.',
      parameters: { type: 'object', properties: { title: { type: 'string' } }, required: ['title'] },
      handler: async ({ title }) => {
        const task = await c.createTask({ listId: c.listId, title: title ?? '' })
        return { title: task.title, id: task.id }
      },
    },
    {
      name: 'search_tasks',
      description: 'Tasks whose title or notes match the query.',
      parameters: { type: 'object', properties: { query: { type: 'string' } }, required: ['query'] },
      handler: ({ query }) => lines(searchTasks(query ?? '', c.entities.tasks, MAX_SHOWN).map((h) => h.task)),
    },
    {
      name: 'due_tasks',
      description: 'Open tasks due today (overdue included), in the next 7 days, or overdue.',
      parameters: { type: 'object', properties: { view: { type: 'string', enum: ['today', 'next7', 'overdue'] } }, required: ['view'] },
      handler: ({ view }) => {
        if (view === 'overdue') return lines(overdue(live, c.now))
        return lines(tasksForView({ kind: view === 'next7' ? 'week' : 'today' }, c.entities, c.now))
      },
    },
    {
      name: 'complete_task',
      description: 'Complete the one open task this names.',
      parameters: titled,
      handler: ({ query }) => onOne(live.filter((t) => t.status === 'open'), query ?? '', 'open task', (t) => c.toggleDone(t.id)),
    },
    {
      name: 'reopen_task',
      description: 'Open again the one done task this names.',
      parameters: titled,
      handler: ({ query }) => onOne(live.filter((t) => t.status !== 'open'), query ?? '', 'done task', (t) => c.reopen(t.id)),
    },
    {
      name: 'delete_task',
      description: 'Move the one task this names to the Trash.',
      parameters: titled,
      handler: ({ query }) => onOne(live, query ?? '', 'task', (t) => c.trash(t.id)),
    },
    {
      name: 'list_lists',
      description: 'The person’s lists, by name.',
      parameters: { type: 'object' },
      handler: () => ({ lists: c.entities.lists.filter((l) => !l.archived).map((l) => l.name) }),
    },
  ]
}

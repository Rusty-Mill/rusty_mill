/**
 * Hash routes, matching the ones the reference app uses:
 *
 *   #q/all|today|week/tasks[/<taskId>]   smart lists
 *   #p/inbox|<listId>/tasks[/<taskId>]   Inbox and lists
 *   #t/<tag>/tasks[/<taskId>]            a tag
 *   #f/<filterId>/tasks[/<taskId>]       a saved filter
 *   #q/all/completed  #q/all/trash  #q/all/summary  #q/all/habit
 *   #c/all/calendar/<m|w|d|3|t|a> (month, week, day, 3-day, ten-day, agenda)            calendar
 *   #focus                               Pomodoro
 *   #countdown                           Countdown
 */
import type { ViewSpec } from '@/features/tasks/organize'

export const HOME = '/q/all/tasks'

const SMART: Record<string, ViewSpec> = { all: { kind: 'all' }, today: { kind: 'today' }, week: { kind: 'week' } }

/** The view a route's params name, or `null` if they name nothing valid. */
export function parseView(params: { smart?: string; listId?: string; tag?: string; filterId?: string }, inboxId: string): ViewSpec | null {
  if (params.smart !== undefined) return SMART[params.smart] ?? null
  if (params.listId !== undefined) return params.listId === 'inbox' || params.listId === inboxId ? { kind: 'inbox' } : { kind: 'list', id: params.listId }
  if (params.tag !== undefined) return { kind: 'tag', name: params.tag }
  if (params.filterId !== undefined) return { kind: 'filter', id: params.filterId }
  return null
}

/** The URL of a view (without a selected task). */
export function viewPath(spec: ViewSpec): string {
  switch (spec.kind) {
    case 'all':
      return '/q/all/tasks'
    case 'today':
      return '/q/today/tasks'
    case 'week':
      return '/q/week/tasks'
    case 'inbox':
      return '/p/inbox/tasks'
    case 'list':
      return `/p/${spec.id}/tasks`
    case 'tag':
      return `/t/${encodeURIComponent(spec.name)}/tasks`
    case 'filter':
      return `/f/${spec.id}/tasks`
  }
}

/** The URL of a view with `taskId` open in the detail pane. */
export const taskPath = (spec: ViewSpec, taskId: string): string => `${viewPath(spec)}/${taskId}`

export const CALENDAR_MODES = ['m', 'w', 'd', '3', 't', 'a'] as const
export type CalendarMode = (typeof CALENDAR_MODES)[number]
export const calendarPath = (mode: CalendarMode = 'm'): string => `/c/all/calendar/${mode}`

export const PATHS = {
  completed: '/q/all/completed',
  trash: '/q/all/trash',
  summary: '/q/all/summary',
  habit: '/q/all/habit',
  countdown: '/countdown',
  focus: '/focus',
} as const

/** Settings is a modal over whatever route is open: `?modalType=settings&tabs=account`. */
export const SETTINGS_TABS = ['account', 'premium', 'features', 'smart-list', 'notifications', 'date-time', 'appearance', 'ai', 'more', 'integrations', 'collaborate', 'shortcuts', 'about'] as const
export type SettingsTab = (typeof SETTINGS_TABS)[number]

export function settingsHref(tab: SettingsTab = 'account'): string {
  return `?modalType=settings&tabs=${tab}`
}

/** Which rail icon a path belongs to. */
export type RailSection = 'tasks' | 'calendar' | 'focus' | 'habit' | 'countdown'
export function railSection(path: string): RailSection {
  if (path.startsWith('/c/')) return 'calendar'
  if (path.startsWith('/focus')) return 'focus'
  if (path.startsWith(PATHS.habit)) return 'habit'
  if (path.startsWith(PATHS.countdown)) return 'countdown'
  return 'tasks'
}

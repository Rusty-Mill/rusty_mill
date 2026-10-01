import type { Task } from '@/api/types'
import type { WeekStart } from '@/lib/date'
import type { Reschedule } from './layout'

/** A time (or, for `allDay`, a day) the add popover creates a task at. */
export interface AddTarget {
  ms: number
  allDay: boolean
}

/** What every view needs from the page, so the views stay free of store and routing concerns. */
export interface CalendarCtx {
  now: number
  hour12: boolean
  weekStart: WeekStart
  colorOf(task: Task): string
  openTask(task: Task, anchor: HTMLElement): void
  openAdd(target: AddTarget, anchor: HTMLElement): void
  openDay(day: number, anchor: HTMLElement): void
  move(task: Task, to: Reschedule): void
  /** The slot the add popover is open on, so the grid can mark it. */
  addSlot: AddTarget | null
}

import type { Task } from '@/api/types'

/**
 * What is being dragged. `dataTransfer` cannot be read during `dragover`, and
 * the drop needs more than the id (where the bar was grabbed), so the views
 * share this instead.
 */
export interface DragInfo {
  task: Task
  /** The day under the pointer when the drag began. */
  grabDay: number
  /** The first day of the dragged event (its bar may be clipped by the week row). */
  startDay: number
  /** Minutes from the top of the dragged block to the pointer (0 for a bar). */
  grabMinutes: number
}

export const drag: { current: DragInfo | null } = { current: null }

/** Firefox will not start a drag without data on the transfer. */
export function beginDrag(e: React.DragEvent, info: DragInfo): void {
  drag.current = info
  e.dataTransfer.effectAllowed = 'move'
  e.dataTransfer.setData('text/plain', info.task.id)
}

export const endDrag = (): void => {
  drag.current = null
}

/** A stand-in anchor for a popover that hangs off a rectangle, such as a time slot, not an element. */
export function rectAnchor(rect: DOMRect): HTMLElement {
  const el = document.createElement('div')
  el.getBoundingClientRect = () => rect
  return el
}

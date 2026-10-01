import { ChevronDown, ChevronLeft, ChevronRight, Plus } from 'lucide-react'
import { useState } from 'react'
import type { CalendarMode } from '@/app/paths'
import { Menu } from '@/components/Menu'

export const MODE_LABEL: Record<CalendarMode, string> = { m: 'Month', w: 'Week', d: 'Day', a: 'Agenda' }
const NAV = { m: 'month', w: 'week', d: 'day', a: 'page' } as const

interface Props {
  title: string
  mode: CalendarMode
  showDone: boolean
  onMode: (m: CalendarMode) => void
  onShowDone: (v: boolean) => void
  onPrev: () => void
  onNext: () => void
  onToday: () => void
  onAdd: (anchor: HTMLElement) => void
}

const btn = 'flex h-8 items-center justify-center rounded-row text-base outline-none hover:bg-hover focus-visible:ring-2 focus-visible:ring-primary'

export function Toolbar({ title, mode, showDone, onMode, onShowDone, onPrev, onNext, onToday, onAdd }: Props) {
  const [viewAnchor, setViewAnchor] = useState<HTMLElement | null>(null)
  const [open, setOpen] = useState(false)
  return (
    <header className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-line px-4 py-2.5">
      <h1 className="mr-1 min-w-0 text-title font-semibold" aria-live="polite">
        {title}
      </h1>
      <div className="ml-auto flex items-center gap-2">
        <button type="button" aria-label="Add task" onClick={(e) => onAdd(e.currentTarget)} className={`${btn} w-8 text-primary`}>
          <Plus size={20} strokeWidth={1.5} />
        </button>
        <button
          ref={setViewAnchor}
          type="button"
          aria-haspopup="menu"
          aria-expanded={open}
          aria-label={`View: ${MODE_LABEL[mode]}`}
          onClick={() => setOpen(true)}
          className={`${btn} gap-1 border border-line pl-3 pr-2`}
        >
          {MODE_LABEL[mode]}
          <ChevronDown size={16} strokeWidth={1.5} className="text-grey" />
        </button>
        <div className="flex items-center overflow-hidden rounded-row border border-line">
          <button type="button" aria-label={`Previous ${NAV[mode]}`} onClick={onPrev} className={`${btn} w-8 rounded-none text-grey`}>
            <ChevronLeft size={20} strokeWidth={1.5} />
          </button>
          <button type="button" onClick={onToday} className={`${btn} rounded-none border-x border-line px-3`}>
            Today
          </button>
          <button type="button" aria-label={`Next ${NAV[mode]}`} onClick={onNext} className={`${btn} w-8 rounded-none text-grey`}>
            <ChevronRight size={20} strokeWidth={1.5} />
          </button>
        </div>
      </div>
      <Menu
        anchor={viewAnchor}
        open={open}
        onClose={() => setOpen(false)}
        placement="bottom-end"
        label="Calendar view"
        items={[
          ...(Object.keys(MODE_LABEL) as CalendarMode[]).map((m) => ({ id: m, label: MODE_LABEL[m], checked: m === mode, onSelect: () => onMode(m) })),
          'separator',
          { id: 'done', label: 'Show completed', checked: showDone, onSelect: () => onShowDone(!showDone) },
        ]}
      />
    </header>
  )
}

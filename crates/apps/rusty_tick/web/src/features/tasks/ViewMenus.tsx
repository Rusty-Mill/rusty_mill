import { ArrowUpDown, Columns3, GanttChart, List as ListIcon, Printer, SlidersHorizontal, Eye, EyeOff, MoreHorizontal } from 'lucide-react'
import { useRef, useState } from 'react'
import type { ViewMode } from '@/api/types'
import { Menu, type MenuEntry } from '@/components/Menu'
import type { GroupBy, Order, SortBy, ViewOptions } from './organize'

const GROUP_LABEL: Record<GroupBy, string> = { date: 'Date', list: 'List', priority: 'Priority', tag: 'Tag', none: 'None' }
const SORT_LABEL: Record<SortBy, string> = { manual: 'Custom', date: 'Date', title: 'Title', priority: 'Priority', created: 'Created' }
const ORDER_LABEL: Record<Order, string> = { asc: 'Ascending', desc: 'Descending' }

interface Common {
  options: ViewOptions
  onChange: (patch: Partial<ViewOptions>) => void
}

/** The sort icon's popover: three rows (Group by, Sort by, Order), each opening a submenu of choices. */
export function SortMenu({ options, onChange }: Common) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLButtonElement>(null)
  const pick = <K extends keyof ViewOptions>(key: K, labels: Record<string, string>): MenuEntry[] =>
    Object.entries(labels).map(([value, label]) => ({ id: value, label, checked: options[key] === value, onSelect: () => onChange({ [key]: value } as Partial<ViewOptions>) }))
  const items: MenuEntry[] = [
    { id: 'group', label: 'Group by', hint: GROUP_LABEL[options.groupBy], submenu: pick('groupBy', GROUP_LABEL) },
    { id: 'sort', label: 'Sort by', hint: SORT_LABEL[options.sortBy], submenu: pick('sortBy', SORT_LABEL) },
    { id: 'order', label: 'Order', hint: ORDER_LABEL[options.order], submenu: pick('order', ORDER_LABEL) },
  ]
  return (
    <>
      <button ref={ref} type="button" aria-label="Sort" aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((o) => !o)} className="flex h-8 w-8 items-center justify-center rounded-row text-grey hover:bg-hover">
        <ArrowUpDown size={18} />
      </button>
      <Menu anchor={ref.current} open={open} onClose={() => setOpen(false)} items={items} label="Sort" placement="bottom-end" className="min-w-[220px]" />
    </>
  )
}

const MODES: { mode: ViewMode; label: string; icon: React.ReactNode }[] = [
  { mode: 'list', label: 'List', icon: <ListIcon size={18} /> },
  { mode: 'kanban', label: 'Kanban', icon: <Columns3 size={18} /> },
  { mode: 'timeline', label: 'Timeline', icon: <GanttChart size={18} /> },
]

/** The "..." popover: view switcher, Hide/Show Completed, Show Details, View Options, Print. */
export function MoreMenu({ options, onChange, viewMode, onViewMode, onViewOptions }: Common & { viewMode: ViewMode; onViewMode: (m: ViewMode) => void; onViewOptions: () => void }) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLButtonElement>(null)
  const items: MenuEntry[] = [
    {
      id: 'views',
      custom: (
        <div role="radiogroup" aria-label="View" className="flex gap-1 px-3 pb-1.5 pt-1">
          {MODES.map((m) => (
            <button
              key={m.mode}
              type="button"
              role="radio"
              aria-checked={viewMode === m.mode}
              aria-label={m.label}
              title={m.label}
              onClick={() => {
                onViewMode(m.mode)
                setOpen(false)
              }}
              className={`flex h-9 flex-1 items-center justify-center rounded-row ${viewMode === m.mode ? 'bg-primary/10 text-primary' : 'text-grey hover:bg-hover'}`}
            >
              {m.icon}
            </button>
          ))}
        </div>
      ),
    },
    'separator',
    { id: 'completed', label: options.showCompleted ? 'Hide Completed' : 'Show Completed', icon: options.showCompleted ? <EyeOff size={16} /> : <Eye size={16} />, onSelect: () => onChange({ showCompleted: !options.showCompleted }) },
    { id: 'details', label: 'Show Details', checked: options.showDetails, onSelect: () => onChange({ showDetails: !options.showDetails }) },
    { id: 'options', label: 'View Options', icon: <SlidersHorizontal size={16} />, onSelect: onViewOptions },
    'separator',
    { id: 'print', label: 'Print', icon: <Printer size={16} />, onSelect: () => window.print() },
  ]
  return (
    <>
      <button ref={ref} type="button" aria-label="More" aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((o) => !o)} className="flex h-8 w-8 items-center justify-center rounded-row text-grey hover:bg-hover">
        <MoreHorizontal size={18} />
      </button>
      <Menu anchor={ref.current} open={open} onClose={() => setOpen(false)} items={items} label="More" placement="bottom-end" className="min-w-[220px]" />
    </>
  )
}

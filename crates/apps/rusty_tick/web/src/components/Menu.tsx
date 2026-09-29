import { ChevronRight, Check } from 'lucide-react'
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Popover, type Placement } from './Popover'

export type MenuEntry =
  | 'separator'
  /** Arbitrary content in the menu (a segmented control, say). Not reachable with the arrow keys. */
  | { id: string; custom: ReactNode }
  | {
      id: string
      label: string
      icon?: ReactNode
      /** Right-aligned secondary text (a shortcut, or the current value of a submenu). */
      hint?: string
      /** Makes the item a checkbox (`true`/`false`) that shows a tick when on. */
      checked?: boolean
      danger?: boolean
      disabled?: boolean
      onSelect?: () => void
      submenu?: MenuEntry[]
    }

interface MenuProps {
  anchor: HTMLElement | null
  open: boolean
  onClose: () => void
  items: MenuEntry[]
  placement?: Placement
  label?: string
  className?: string
  /** A submenu: the left arrow closes it. */
  nested?: boolean
}

type Item = Exclude<MenuEntry, 'separator' | { custom: ReactNode }>
const isItem = (e: MenuEntry): e is Item => e !== 'separator' && !('custom' in e)

/** A keyboard-navigable menu: arrows, Home/End, Enter/Space, → opens a submenu, ← or Esc closes it. */
export function Menu({ anchor, open, onClose, items, placement = 'bottom-start', label, className = '', nested = false }: MenuProps) {
  const listRef = useRef<HTMLDivElement>(null)
  const [active, setActive] = useState(0)
  const [sub, setSub] = useState<{ id: string; anchor: HTMLElement } | null>(null)
  const enabled = items.map((e, i) => (isItem(e) && !e.disabled ? i : -1)).filter((i) => i >= 0)

  useEffect(() => {
    if (!open) return
    setActive(enabled[0] ?? 0)
    setSub(null)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  useEffect(() => {
    // While a submenu is open it owns the focus; when it closes, focus comes back here.
    if (open && !sub) listRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`)?.focus()
  }, [open, active, sub])

  const move = (delta: number): void => {
    const at = enabled.indexOf(active)
    setActive(enabled[(at + delta + enabled.length) % enabled.length] ?? active)
  }

  const choose = (item: Item, el: HTMLElement): void => {
    if (item.disabled) return
    if (item.submenu) return setSub({ id: item.id, anchor: el })
    item.onSelect?.()
    onClose()
  }

  const onKeyDown = (e: React.KeyboardEvent): void => {
    const item = items[active]
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault()
        return move(1)
      case 'ArrowUp':
        e.preventDefault()
        return move(-1)
      case 'Home':
        e.preventDefault()
        return setActive(enabled[0] ?? 0)
      case 'End':
        e.preventDefault()
        return setActive(enabled[enabled.length - 1] ?? 0)
      case 'ArrowRight':
        if (item && isItem(item) && item.submenu) {
          e.preventDefault()
          setSub({ id: item.id, anchor: e.target as HTMLElement })
        }
        return
      case 'ArrowLeft':
        if (nested) {
          e.preventDefault()
          onClose()
        }
        return
      case 'Tab':
        return onClose()
    }
  }

  const subItem = sub ? items.find((e): e is Item => isItem(e) && e.id === sub.id) : undefined

  return (
    <>
      <Popover anchor={anchor} open={open} onClose={onClose} placement={placement} role="menu" ariaLabel={label} className={`min-w-[180px] py-1.5 ${className}`}>
        <div ref={listRef} onKeyDown={onKeyDown}>
          {items.map((entry, i) =>
            entry === 'separator' ? (
              <div key={`sep-${i}`} role="separator" className="my-1 h-px bg-line" />
            ) : 'custom' in entry ? (
              <div key={entry.id}>{entry.custom}</div>
            ) : (
              <button
                key={entry.id}
                type="button"
                role={entry.checked === undefined ? 'menuitem' : 'menuitemcheckbox'}
                aria-checked={entry.checked}
                aria-haspopup={entry.submenu ? 'menu' : undefined}
                aria-expanded={entry.submenu ? sub?.id === entry.id : undefined}
                aria-disabled={entry.disabled || undefined}
                data-index={i}
                tabIndex={i === active ? 0 : -1}
                onMouseEnter={() => setActive(i)}
                onClick={(e) => choose(entry, e.currentTarget)}
                className={`flex w-full items-center gap-2.5 px-3 py-1.5 text-left text-base outline-none hover:bg-hover focus-visible:bg-hover ${entry.danger ? 'text-danger' : ''} ${entry.disabled ? 'cursor-default opacity-40' : ''}`}
              >
                {entry.icon && <span className="flex h-5 w-5 items-center justify-center text-grey">{entry.icon}</span>}
                <span className="flex-1 truncate">{entry.label}</span>
                {entry.hint && <span className="text-s text-grey">{entry.hint}</span>}
                {entry.checked && <Check size={16} className="text-primary" aria-hidden />}
                {entry.submenu && <ChevronRight size={16} className="text-grey" aria-hidden />}
              </button>
            ),
          )}
        </div>
        {/* Rendered inside the parent so presses in it count as inside the parent (React events cross portals). */}
        {sub && subItem?.submenu && (
          <Menu nested anchor={sub.anchor} open onClose={() => setSub(null)} items={wrapClose(subItem.submenu, onClose)} placement="right-start" label={subItem.label} />
        )}
      </Popover>
    </>
  )
}

/** Picking something in a submenu closes the whole menu, not just the submenu. */
function wrapClose(entries: MenuEntry[], closeAll: () => void): MenuEntry[] {
  return entries.map((e) => (e === 'separator' || 'custom' in e ? e : { ...e, onSelect: e.onSelect ? () => { e.onSelect?.(); closeAll() } : undefined }))
}

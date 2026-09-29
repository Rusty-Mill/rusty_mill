import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'

export type Placement = 'bottom-start' | 'bottom-end' | 'top-start' | 'top-end' | 'right-start' | 'left-start'

interface PopoverProps {
  /** The element the popover hangs off; clicks on it do not count as "outside". */
  anchor: HTMLElement | null
  open: boolean
  onClose: () => void
  placement?: Placement
  /** Gap between anchor and popover, px. */
  offset?: number
  className?: string
  role?: string
  ariaLabel?: string
  /** Give focus back to whatever had it before opening. Default true. */
  restoreFocus?: boolean
  children: ReactNode
}

const MARGIN = 8

/** Open popovers, innermost last: only the top one answers Escape. */
const openStack: symbol[] = []

/** Where a popover of `size` goes for `placement`, kept on screen and flipped if it would not fit. */
export function computePosition(
  anchor: DOMRect,
  size: { width: number; height: number },
  placement: Placement,
  offset: number,
  viewport: { width: number; height: number },
): { left: number; top: number } {
  let [side, align] = placement.split('-') as ['bottom' | 'top' | 'right' | 'left', 'start' | 'end']
  const room = {
    bottom: viewport.height - anchor.bottom - offset - MARGIN,
    top: anchor.top - offset - MARGIN,
    right: viewport.width - anchor.right - offset - MARGIN,
    left: anchor.left - offset - MARGIN,
  }
  // Flip when there is not room on the preferred side but there is on the opposite one.
  if (side === 'bottom' && room.bottom < size.height && room.top > room.bottom) side = 'top'
  else if (side === 'top' && room.top < size.height && room.bottom > room.top) side = 'bottom'
  else if (side === 'right' && room.right < size.width && room.left > room.right) side = 'left'
  else if (side === 'left' && room.left < size.width && room.right > room.left) side = 'right'

  let left: number
  let top: number
  if (side === 'bottom' || side === 'top') {
    left = align === 'start' ? anchor.left : anchor.right - size.width
    top = side === 'bottom' ? anchor.bottom + offset : anchor.top - offset - size.height
  } else {
    top = align === 'start' ? anchor.top : anchor.bottom - size.height
    left = side === 'right' ? anchor.right + offset : anchor.left - offset - size.width
  }
  return {
    left: Math.max(MARGIN, Math.min(left, viewport.width - size.width - MARGIN)),
    top: Math.max(MARGIN, Math.min(top, viewport.height - size.height - MARGIN)),
  }
}

/**
 * A floating panel anchored to an element. Closes on Escape and on a press
 * outside it, including outside any popover React-nested inside it (a submenu
 * belongs to its parent even though it renders in a separate portal).
 */
export function Popover({ anchor, open, onClose, placement = 'bottom-start', offset = 6, className = '', role, ariaLabel, restoreFocus = true, children }: PopoverProps) {
  const ref = useRef<HTMLDivElement>(null)
  const inside = useRef(false)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)
  const onCloseRef = useRef(onClose)
  onCloseRef.current = onClose

  useLayoutEffect(() => {
    if (!open || !anchor || !ref.current) return
    const place = (): void => {
      const el = ref.current
      if (!el) return
      setPos(computePosition(anchor.getBoundingClientRect(), { width: el.offsetWidth, height: el.offsetHeight }, placement, offset, { width: window.innerWidth, height: window.innerHeight }))
    }
    place()
    window.addEventListener('resize', place)
    window.addEventListener('scroll', place, true)
    return () => {
      window.removeEventListener('resize', place)
      window.removeEventListener('scroll', place, true)
    }
  }, [open, anchor, placement, offset, children])

  useEffect(() => {
    if (!open) return
    const me = Symbol('popover')
    openStack.push(me)
    const returnTo = restoreFocus ? (document.activeElement as HTMLElement | null) : null
    const onPointerDown = (e: PointerEvent): void => {
      const wasInside = inside.current
      inside.current = false
      if (wasInside || (anchor && anchor.contains(e.target as Node))) return
      onCloseRef.current()
    }
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape' && openStack[openStack.length - 1] === me) {
        e.stopPropagation()
        onCloseRef.current()
      }
    }
    document.addEventListener('pointerdown', onPointerDown)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('pointerdown', onPointerDown)
      document.removeEventListener('keydown', onKey)
      openStack.splice(openStack.indexOf(me), 1)
      if (returnTo && document.contains(returnTo)) returnTo.focus()
    }
  }, [open, anchor, restoreFocus])

  if (!open) return null
  return createPortal(
    <div
      ref={ref}
      role={role}
      aria-label={ariaLabel}
      data-popover=""
      onPointerDownCapture={() => {
        inside.current = true // React events bubble through portals, so nested popovers land here too
      }}
      // opacity, not visibility: a `visibility:hidden` element cannot take focus, and menus focus themselves as they open
      style={{ position: 'fixed', left: pos?.left ?? 0, top: pos?.top ?? 0, opacity: pos ? 1 : 0, pointerEvents: pos ? 'auto' : 'none', zIndex: 60 }}
      className={`pop-shadow rounded-menu border border-line bg-surface text-text ${className}`}
    >
      {children}
    </div>,
    document.body,
  )
}

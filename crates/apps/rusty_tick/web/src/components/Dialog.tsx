import { X } from 'lucide-react'
import { useEffect, useId, useRef, type ReactNode } from 'react'
import { createPortal } from 'react-dom'

const FOCUSABLE = 'a[href],button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex="-1"])'

interface DialogProps {
  open: boolean
  onClose: () => void
  /** Visible heading; also names the dialog for assistive tech. */
  title?: string
  /** Pass instead of `title` when the heading is rendered by the children. */
  labelledBy?: string
  width?: number | string
  height?: number | string
  className?: string
  /** Hide the built-in close button (the content provides its own). */
  bare?: boolean
  children: ReactNode
}

/** A modal: traps Tab, closes on Escape or a press on the backdrop, and returns focus when it closes. */
export function Dialog({ open, onClose, title, labelledBy, width = 480, height, className = '', bare = false, children }: DialogProps) {
  const ref = useRef<HTMLDivElement>(null)
  const titleId = useId()
  const onCloseRef = useRef(onClose)
  onCloseRef.current = onClose

  useEffect(() => {
    if (!open) return
    const returnTo = document.activeElement as HTMLElement | null
    const el = ref.current
    const first = el?.querySelector<HTMLElement>('[data-autofocus]') ?? el?.querySelector<HTMLElement>(FOCUSABLE) ?? el
    first?.focus()
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') {
        e.stopPropagation()
        return onCloseRef.current()
      }
      if (e.key !== 'Tab' || !el) return
      const nodes = [...el.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((n) => n.offsetParent !== null || n === document.activeElement)
      if (nodes.length === 0) return e.preventDefault()
      const firstNode = nodes[0]!
      const lastNode = nodes[nodes.length - 1]!
      if (e.shiftKey && document.activeElement === firstNode) {
        e.preventDefault()
        lastNode.focus()
      } else if (!e.shiftKey && document.activeElement === lastNode) {
        e.preventDefault()
        firstNode.focus()
      }
    }
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('keydown', onKey)
      if (returnTo && document.contains(returnTo)) returnTo.focus()
    }
  }, [open])

  if (!open) return null
  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelledBy ?? (title ? titleId : undefined)}
        tabIndex={-1}
        style={{ width, height, maxWidth: 'calc(100vw - 32px)', maxHeight: 'calc(100vh - 32px)' }}
        className={`relative flex flex-col overflow-hidden rounded-dialog bg-surface text-text pop-shadow outline-hidden ${className}`}
      >
        {title && (
          <h2 id={titleId} className="px-6 pb-2 pt-5 text-title font-semibold">
            {title}
          </h2>
        )}
        {!bare && (
          <button type="button" aria-label="Close" onClick={onClose} className="absolute right-3 top-3 rounded-row p-1.5 text-grey hover:bg-hover">
            <X size={18} />
          </button>
        )}
        {children}
      </div>
    </div>,
    document.body,
  )
}

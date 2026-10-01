import { useCallback, useEffect, useRef, useState } from 'react'

export interface Draft<T> {
  value: T
  /** Edit the draft; it is saved `delay` ms after the last edit. */
  set: (next: T) => void
  /** Save now if there is an unsaved edit (call on blur). */
  flush: () => void
  /** Whether there is an edit not yet handed to `save`. */
  dirty: boolean
}

/**
 * Local editing state over a server value, with debounced autosave.
 *
 * - A change to `key` (a different record) resets the draft, saving the old one first.
 * - A change to `serverValue` is adopted only while there is nothing unsaved, so
 *   an edit in progress is never overwritten by a refresh.
 * - Leaving (unmount) saves what is pending, so navigating away mid-typing loses nothing.
 */
export function useDraft<T>(key: string, serverValue: T, save: (value: T) => void, delay = 500, equals: (a: T, b: T) => boolean = Object.is): Draft<T> {
  const [value, setValue] = useState(serverValue)
  const [dirty, setDirty] = useState(false)
  const latest = useRef({ value, dirty, save, key })
  latest.current = { value, dirty, save, key }
  // The `save` in force when the edit was made. Switching records swaps `save` for the new
  // record's before the pending edit is flushed; using that one would write the old record's
  // text into the new record.
  const pendingSave = useRef(save)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const lastKey = useRef(key)

  const flush = useCallback(() => {
    clearTimeout(timer.current)
    const cur = latest.current
    if (!cur.dirty) return
    latest.current = { ...cur, dirty: false }
    setDirty(false)
    pendingSave.current(cur.value)
  }, [])

  const set = useCallback(
    (next: T) => {
      pendingSave.current = latest.current.save
      latest.current = { ...latest.current, value: next, dirty: true }
      setValue(next)
      setDirty(true)
      clearTimeout(timer.current)
      timer.current = setTimeout(flush, delay)
    },
    [delay, flush],
  )

  // Another record: save the one we are leaving (with its own save function), then start over.
  useEffect(() => {
    if (lastKey.current === key) return
    lastKey.current = key
    setValue(serverValue)
    setDirty(false)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key])

  // The server moved on: take it, unless the user has typed something we have not saved.
  useEffect(() => {
    if (!latest.current.dirty && !equals(latest.current.value, serverValue)) setValue(serverValue)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serverValue])

  useEffect(() => flush, [flush, key])

  return { value, set, flush, dirty }
}

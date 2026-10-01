import { useCallback, useEffect, useRef, useState } from 'react'
import type { ApiClient } from '@/api/client'
import { newId } from '@/lib/id'
import { defaultOptions, sanitizeOptions, type SummaryOptions } from './options'

const SAVE_DELAY = 600

/**
 * The last-used template, filters and display options, kept as one
 * `summary_template` document: loaded when the page opens, saved shortly after
 * each change (and once more on leaving, so a quick change is not lost).
 */
export function useSummaryOptions(api: ApiClient, now: () => number = Date.now): { options: SummaryOptions; loaded: boolean; update: (patch: Partial<SummaryOptions>) => void } {
  const [options, setOptions] = useState<SummaryOptions>(() => defaultOptions(now()))
  const [loaded, setLoaded] = useState(false)
  const docId = useRef<string | null>(null)
  const latest = useRef(options)
  const touched = useRef(false)
  const dirty = useRef(false)
  const ready = useRef(false) // saving waits for the load, or a new doc would be made next to the existing one
  const timer = useRef<ReturnType<typeof setTimeout>>()

  const save = useCallback((): void => {
    clearTimeout(timer.current)
    if (!dirty.current || !ready.current) return
    dirty.current = false
    docId.current ??= newId()
    void api.putDoc('summary_template', docId.current, { v: 1, name: 'last-used', options: latest.current }).catch(() => undefined) // a lost preference is not worth an error
  }, [api])

  useEffect(() => {
    let cancelled = false
    api
      .listDocs('summary_template')
      .then((docs) => {
        if (cancelled) return
        const doc = docs[0]
        if (doc) {
          docId.current = doc.id
          const body = doc.body as { options?: unknown } | null
          if (!touched.current) {
            latest.current = sanitizeOptions(body?.options, now())
            setOptions(latest.current)
          }
        }
      })
      .catch(() => undefined) // offline: start from defaults
      .finally(() => {
        if (cancelled) return
        ready.current = true
        setLoaded(true)
        save() // a change made while loading
      })
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api])

  useEffect(() => save, [save]) // flush on unmount

  const update = useCallback(
    (patch: Partial<SummaryOptions>): void => {
      touched.current = true
      latest.current = { ...latest.current, ...patch }
      setOptions(latest.current)
      dirty.current = true
      clearTimeout(timer.current)
      timer.current = setTimeout(save, SAVE_DELAY)
    },
    [save],
  )

  return { options, loaded, update }
}

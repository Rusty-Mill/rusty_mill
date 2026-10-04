import { ChevronDown, ChevronRight, RotateCcw } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { BaselineResponse, Card, DiffField } from '@/api/types'
import { useActions, useServices } from '@/app/services'
import { describe } from '@/store/data'
import { Confirm } from '@/components/Confirm'

const FIELD_LABEL: Record<DiffField, string> = {
  name: 'Name',
  suit: 'Suit',
  conception: 'Conception',
  planning: 'Planning',
  execution: 'Execution',
  minimum_standard_of_care: 'Minimum standard of care',
}

/** For a deck card: a disclosure with the field-by-field diff against the original, and the reset. */
export function BaselineBlock({ card }: { card: Card }) {
  const { api } = useServices()
  const { reset, notify } = useActions()
  const [open, setOpen] = useState(false)
  const [data, setData] = useState<BaselineResponse | null>(null)
  const [confirm, setConfirm] = useState(false)

  // Re-fetch whenever the card changes while open (a save or reset changes the diff).
  useEffect(() => {
    if (!open) return
    let live = true
    setData(null)
    api
      .baseline(card.id)
      .then((r) => live && setData(r))
      .catch((e: unknown) => live && notify('error', describe(e)))
    return () => {
      live = false
    }
  }, [open, card, api, notify])

  const doReset = async (): Promise<void> => {
    setConfirm(false)
    await reset(card.id).catch(() => undefined)
  }

  return (
    <section aria-labelledby="baseline-h" className="rounded-card border border-line">
      <h3 id="baseline-h" className="m-0">
        <button type="button" aria-expanded={open} onClick={() => setOpen((o) => !o)} className="flex w-full items-center gap-1.5 px-3 py-2 text-left text-base font-semibold hover:bg-hover">
          {open ? <ChevronDown size={16} className="text-grey" /> : <ChevronRight size={16} className="text-grey" />}
          Changes from the original
        </button>
      </h3>
      {open && (
        <div className="flex flex-col gap-3 border-t border-line px-3 py-3">
          {!data && <p className="text-s text-grey">Loading…</p>}
          {data && data.diff.length === 0 && <p className="text-s text-grey">This card matches the original deck card.</p>}
          {data && data.diff.length > 0 && (
            <table className="w-full border-collapse text-s" aria-label="Differences from the original">
              <thead>
                <tr className="text-left text-grey">
                  <th className="pb-1 pr-2 font-medium">Field</th>
                  <th className="pb-1 pr-2 font-medium">Now</th>
                  <th className="pb-1 font-medium">Original</th>
                </tr>
              </thead>
              <tbody>
                {data.diff.map((d) => (
                  <tr key={d.field} className="border-t border-line align-top">
                    <td className="py-1.5 pr-2 font-medium">{FIELD_LABEL[d.field]}</td>
                    <td className="whitespace-pre-wrap py-1.5 pr-2">{d.field === 'minimum_standard_of_care' ? d.card.split('|').join('\n') : d.card}</td>
                    <td className="whitespace-pre-wrap py-1.5 text-grey">{d.field === 'minimum_standard_of_care' ? d.baseline.split('|').join('\n') : d.baseline}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {card.state === 'edited' && (
            <div>
              <button type="button" onClick={() => setConfirm(true)} className="btn">
                <RotateCcw size={14} /> Reset to original
              </button>
            </div>
          )}
        </div>
      )}
      <Confirm
        open={confirm}
        title="Reset to the original card?"
        message="Name, suit, conception, planning, execution and the minimum standard of care go back to the deck's text. Owner, notes and any split children are kept."
        confirmLabel="Reset"
        danger
        onConfirm={() => void doReset()}
        onCancel={() => setConfirm(false)}
      />
    </section>
  )
}

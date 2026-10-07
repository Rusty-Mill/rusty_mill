import { useEffect, useState } from 'react'
import { AMBIENT, isAmbient } from './ambient'
import type { FocusSettings } from './logic'

const FIELDS: { key: Exclude<keyof FocusSettings, 'ambient'>; label: string; unit: string }[] = [
  { key: 'focusMin', label: 'Focus', unit: 'min' },
  { key: 'shortMin', label: 'Short break', unit: 'min' },
  { key: 'longMin', label: 'Long break', unit: 'min' },
  { key: 'longEvery', label: 'Long break after', unit: 'pomos' },
]

interface Props {
  settings: FocusSettings
  onChange: (patch: Partial<FocusSettings>) => void
}

export function SettingsRow({ settings, onChange }: Props) {
  return (
    <div className="flex flex-wrap justify-center gap-x-5 gap-y-2" role="group" aria-label="Timer settings">
      {FIELDS.map((f) => (
        <NumberField key={f.key} label={f.label} unit={f.unit} value={settings[f.key]} onCommit={(v) => onChange({ [f.key]: v })} />
      ))}
      <label className="flex items-center gap-1.5 text-s text-grey">
        Sound
        <select
          value={settings.ambient}
          aria-label="Ambient sound"
          onChange={(e) => isAmbient(e.target.value) && onChange({ ambient: e.target.value })}
          className="h-7 rounded-row border border-line bg-surface px-2 text-base text-text outline-hidden focus:border-primary"
        >
          {AMBIENT.map((a) => (
            <option key={a.id} value={a.id}>
              {a.label}
            </option>
          ))}
        </select>
      </label>
    </div>
  )
}

/** Edits a draft and commits on blur or Enter, so typing "45" does not pass through a rejected "4". */
function NumberField({ label, unit, value, onCommit }: { label: string; unit: string; value: number; onCommit: (v: number) => void }) {
  const [draft, setDraft] = useState(String(value))
  useEffect(() => setDraft(String(value)), [value])
  const commit = (): void => {
    const n = Number(draft)
    if (draft.trim() !== '' && Number.isFinite(n)) onCommit(n)
    else setDraft(String(value))
  }
  return (
    <label className="flex items-center gap-1.5 text-s text-grey">
      {label}
      <input
        type="number"
        inputMode="numeric"
        min={1}
        value={draft}
        aria-label={`${label} (${unit})`}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === 'Enter' && commit()}
        className="h-7 w-14 rounded-row border border-line bg-surface px-2 text-base text-text outline-hidden focus:border-primary"
      />
    </label>
  )
}

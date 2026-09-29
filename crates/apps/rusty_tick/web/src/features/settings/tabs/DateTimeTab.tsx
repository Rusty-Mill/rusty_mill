import { REMINDERS_TIMED } from '@/features/tasks/dateSelection'
import { usePrefs } from '../prefs'
import { PaneTitle, Row, Segmented, selectClass } from './controls'

export function DateTimeTab() {
  const prefs = usePrefs((s) => s.prefs)
  const update = usePrefs((s) => s.update)
  return (
    <>
      <PaneTitle>Date &amp; Time</PaneTitle>
      <Row label="Week starts on" hint="Calendars, the Next 7 Days list and summaries start their week here.">
        <Segmented
          name="week-start"
          label="Week starts on"
          value={prefs.weekStart}
          onChange={(weekStart) => update({ weekStart })}
          options={[
            { value: 0, label: 'Sunday' },
            { value: 1, label: 'Monday' },
            { value: 6, label: 'Saturday' },
          ]}
        />
      </Row>
      <Row label="Time format">
        <Segmented
          name="time-format"
          label="Time format"
          value={prefs.hour12 ? '12' : '24'}
          onChange={(v) => update({ hour12: v === '12' })}
          options={[
            { value: '12', label: '12-hour' },
            { value: '24', label: '24-hour' },
          ]}
        />
      </Row>
      <Row label="Default reminder" hint="Given to a task when you set a time on it." htmlFor="default-reminder">
        <select id="default-reminder" value={prefs.defaultReminder} onChange={(e) => update({ defaultReminder: e.target.value })} className={selectClass}>
          {REMINDERS_TIMED.map((r) => (
            <option key={r.value} value={r.value}>
              {r.label}
            </option>
          ))}
        </select>
      </Row>
    </>
  )
}

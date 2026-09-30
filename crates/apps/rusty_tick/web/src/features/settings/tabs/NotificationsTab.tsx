import { useState } from 'react'
import { usePrefs } from '../prefs'
import { PaneTitle, Row, Switch } from './controls'

export function NotificationsTab() {
  const supported = typeof Notification !== 'undefined'
  const on = usePrefs((s) => s.prefs.notifications)
  const update = usePrefs((s) => s.update)
  const [denied, setDenied] = useState(supported && Notification.permission === 'denied')

  const toggle = async (next: boolean): Promise<void> => {
    if (next && Notification.permission === 'default') await Notification.requestPermission()
    const blocked = next && Notification.permission !== 'granted'
    setDenied(blocked)
    update({ notifications: next && !blocked })
  }

  if (!supported) return <><PaneTitle>Notifications</PaneTitle><p className="text-base text-grey">This browser does not support notifications.</p></>
  return (
    <>
      <PaneTitle>Notifications</PaneTitle>
      <Row label="Task reminders" hint="Notify when a task's reminder comes due. Works while Tick Local is open in a tab." htmlFor="notify">
        <Switch id="notify" checked={on} onChange={(v) => void toggle(v)} label="Task reminders" />
      </Row>
      {denied && <p role="status" className="mt-3 text-s text-grey">Notifications are blocked for this site. Allow them in the browser's site settings, then turn this on.</p>}
    </>
  )
}

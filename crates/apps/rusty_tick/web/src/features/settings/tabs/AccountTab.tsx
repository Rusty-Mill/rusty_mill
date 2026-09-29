import { useState } from 'react'
import { getToken } from '@/app/env'
import { useServices } from '@/app/services'
import { Confirm } from '@/components/Confirm'
import { resetDemoData, signOut } from '../session'
import { APP_NAME, APP_VERSION } from './AboutTab'
import { PaneTitle, Row } from './controls'

/** `••••••••abcd`: enough to recognise a token, not enough to use one. */
export function maskToken(token: string | null): string {
  if (!token) return 'No token'
  return `${'•'.repeat(8)}${token.length > 12 ? token.slice(-4) : ''}`
}

const button = 'h-8 rounded-row border border-line px-4 hover:bg-hover'

export function AccountTab() {
  const { mode } = useServices()
  const [confirming, setConfirming] = useState(false)
  return (
    <>
      <PaneTitle>Account</PaneTitle>
      {mode === 'server' ? (
        <>
          <Row label="Connected to" hint="The Tick Local server that stores your data.">
            <span className="text-base">{location.host}</span>
          </Row>
          <Row label="Access token" hint="Kept in this browser; it is never shown in full.">
            <code className="text-base">{maskToken(getToken())}</code>
          </Row>
          <Row label="Sign out" hint="Forgets the token on this device. Your data stays on the server.">
            <button type="button" onClick={signOut} className={button}>
              Sign out
            </button>
          </Row>
        </>
      ) : (
        <>
          <p className="border-b border-line pb-4 text-base text-grey">You are using the demo. Everything you create is stored in this browser only, and is lost if you clear the site's data.</p>
          <Row label="Reset sample data" hint="Deletes your demo tasks and lists and starts again from the samples.">
            <button type="button" onClick={() => setConfirming(true)} className={`${button} text-danger`}>
              Reset sample data
            </button>
          </Row>
          <Confirm
            open={confirming}
            danger
            title="Reset sample data?"
            message="This deletes every task and list in the demo and restores the sample data. It cannot be undone."
            confirmLabel="Reset"
            onCancel={() => setConfirming(false)}
            onConfirm={resetDemoData}
          />
        </>
      )}
      <Row label="Application">
        <span className="text-base text-grey">
          {APP_NAME} {APP_VERSION}
        </span>
      </Row>
    </>
  )
}

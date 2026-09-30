import { useState } from 'react'
import { getToken, identity } from '@/app/env'
import { useActions, useServices } from '@/app/services'
import { downloadFile } from '@/features/summary/download'
import { dayKey } from '@/lib/date'
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

const card = 'flex flex-col rounded-[10px] bg-side px-4 py-1 [&>*+*]:border-t [&>*+*]:border-line'
const line = 'flex min-h-10 items-center justify-between gap-3 py-2 text-base'
const soon = 'text-primary/50'

export function AccountTab() {
  const { mode, api } = useServices()
  const { notify } = useActions()
  const who = mode === 'server' ? identity(getToken()) || 'Tick Local' : 'Demo'
  const backup = (): void => {
    api
      .snapshot()
      .then((snap) => downloadFile(`tick-backup-${dayKey(Date.now())}.json`, 'application/json', JSON.stringify(snap, null, 2)))
      .catch(() => notify('error', 'Could not create a backup'))
  }
  const [confirming, setConfirming] = useState(false)
  return (
    <>
      <PaneTitle>Account</PaneTitle>
      <div className="mb-4 flex flex-col items-center gap-1">
        <span aria-hidden className="flex h-16 w-16 items-center justify-center rounded-full bg-selected text-title font-semibold text-grey">
          {who.charAt(0).toUpperCase()}
        </span>
        <p className="mt-1 font-semibold">{who}</p>
        <p className="text-s text-grey">{mode === 'server' ? location.host : 'Stored in this browser'}</p>
      </div>
      <div className={`${card} mb-3`}>
        <div className={line}>
          <span>Email</span>
          <span className="text-grey">Not used</span>
        </div>
        <div className={line}>
          <span>Password</span>
          <span className={soon} title="Tick Local signs in with a token">Change Password</span>
        </div>
        <div className={line}>
          <span>2-Step Verification</span>
          <span className={soon} title="Not available">Setting</span>
        </div>
      </div>
      <div className={`${card} mb-3`}>
        <div className={line}>
          <span>Login Devices</span>
          <span className={soon} title="Not available">Manage</span>
        </div>
        <div className={line}>
          <span>API Keys</span>
          <span className={soon} title="Tokens are managed with the rusty_tick command line">Manage</span>
        </div>
        <div className={line}>
          <span>Backup &amp; Restore</span>
          <span className="flex gap-4">
            <button type="button" onClick={backup} className="text-primary hover:underline">
              Generate Backup
            </button>
            <span className={soon} title="Not available">Import Backups</span>
          </span>
        </div>
        <div className={line}>
          <span>Manage Account</span>
          <span className="text-danger/50" title="Not available">Delete Account</span>
        </div>
      </div>
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

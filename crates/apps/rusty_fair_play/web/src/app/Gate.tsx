import { useState, type FormEvent } from 'react'
import { getToken, setToken } from './env'

const Card = ({ children }: { children: React.ReactNode }) => (
  <div className="flex h-full items-center justify-center bg-side p-4">
    <div className="pop-shadow flex w-[420px] max-w-full flex-col gap-4 rounded-dialog bg-surface p-8">
      <div className="flex items-center gap-3">
        <span aria-hidden className="flex h-9 w-9 items-center justify-center rounded-row bg-suit-home text-base font-bold text-white">
          FP
        </span>
        <h1 className="text-h1 font-semibold">Fair Play</h1>
      </div>
      {children}
    </div>
  </div>
)

/** Shown when the server answers 401: ask for the API token. It is kept for this tab only unless "remember" is ticked. */
export function TokenPrompt({ error, onSubmit }: { error?: string | null; onSubmit: () => void }) {
  const [token, setTokenText] = useState('')
  const [remember, setRemember] = useState(false)
  const submit = (e: FormEvent): void => {
    e.preventDefault()
    if (!token.trim()) return
    setToken(token.trim(), remember)
    onSubmit()
  }
  return (
    <Card>
      <p className="text-grey">This server asks for a token.</p>
      <form onSubmit={submit} className="flex flex-col gap-3">
        <label className="flex flex-col gap-1.5">
          <span className="text-s text-grey">API token</span>
          <input
            data-autofocus
            autoFocus
            type="password"
            autoComplete="off"
            value={token}
            onChange={(e) => setTokenText(e.target.value)}
            placeholder={getToken() ? 'The saved token was not accepted' : 'The token the server was started with'}
            className="h-10 rounded-row border border-line bg-surface px-3 outline-none focus:border-primary"
          />
        </label>
        {error && (
          <p role="alert" className="text-s text-danger">
            {error}
          </p>
        )}
        <label className="flex items-center gap-2 text-s text-grey">
          <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
          Remember on this device
        </label>
        <button type="submit" disabled={!token.trim()} className="h-10 rounded-row bg-primary text-white disabled:opacity-40">
          Connect
        </button>
      </form>
    </Card>
  )
}

export function Splash({ children }: { children: React.ReactNode }) {
  return <div className="flex h-full items-center justify-center text-grey">{children}</div>
}

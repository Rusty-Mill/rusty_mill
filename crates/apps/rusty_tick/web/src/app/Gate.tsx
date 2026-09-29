import { useState, type FormEvent } from 'react'
import { NoTasksArt } from '@/components/Illustrations'
import { clearToken, getToken, setMode, setToken, type Mode } from './env'

const Card = ({ children }: { children: React.ReactNode }) => (
  <div className="flex h-full items-center justify-center bg-side p-4">
    <div className="pop-shadow flex w-[420px] max-w-full flex-col gap-4 rounded-dialog bg-surface p-8">
      <div className="flex items-center gap-3">
        <NoTasksArt />
        <h1 className="text-h1 font-semibold">Tick Local</h1>
      </div>
      {children}
    </div>
  </div>
)

/** First run: connect to a `rusty_tick` server, or try the app on sample data in this browser. */
export function Welcome({ onChoose }: { onChoose: (mode: Mode) => void }) {
  return (
    <Card>
      <p className="text-grey">A private task manager. Your tasks live on your own server.</p>
      <button type="button" data-autofocus onClick={() => { setMode('server'); onChoose('server') }} className="h-10 rounded-row bg-primary text-white">
        Connect to my server
      </button>
      <button type="button" onClick={() => { setMode('demo'); onChoose('demo') }} className="h-10 rounded-row border border-line hover:bg-hover">
        Try it with sample data
      </button>
      <p className="text-s text-grey">Sample data stays in this browser and is not sent anywhere.</p>
    </Card>
  )
}

/** Ask for the API token. It is kept for this tab only unless "remember" is ticked. */
export function TokenPrompt({ error, onSubmit, onDemo }: { error?: string | null; onSubmit: () => void; onDemo: () => void }) {
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
      <form onSubmit={submit} className="flex flex-col gap-3">
        <label className="flex flex-col gap-1.5">
          <span className="text-s text-grey">API token</span>
          <input
            data-autofocus
            type="password"
            autoComplete="off"
            value={token}
            onChange={(e) => setTokenText(e.target.value)}
            placeholder={getToken() ? 'The saved token was not accepted' : 'The token the server was started with'}
            className="h-10 rounded-row border border-line bg-surface px-3 outline-none focus:border-primary"
          />
        </label>
        {error && <p role="alert" className="text-s text-danger">{error}</p>}
        <label className="flex items-center gap-2 text-s text-grey">
          <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
          Remember on this device
        </label>
        <button type="submit" disabled={!token.trim()} className="h-10 rounded-row bg-primary text-white disabled:opacity-40">
          Connect
        </button>
      </form>
      <button type="button" onClick={() => { clearToken(); setMode('demo'); onDemo() }} className="text-s text-grey underline">
        Use sample data instead
      </button>
    </Card>
  )
}

export function Splash({ children }: { children: React.ReactNode }) {
  return <div className="flex h-full items-center justify-center text-grey">{children}</div>
}

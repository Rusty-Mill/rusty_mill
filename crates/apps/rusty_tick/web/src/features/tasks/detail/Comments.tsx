import { useEffect, useState, type FormEvent } from 'react'
import { Trash2 } from 'lucide-react'
import { useActions, useServices } from '@/app/services'
import { newId } from '@/lib/id'

interface CommentBody {
  v: 1
  taskId: string
  text: string
  createdMs: number
}
interface Comment extends CommentBody {
  id: string
}

const asBody = (b: unknown): CommentBody | null => {
  const o = b as Partial<CommentBody> | null
  return o && typeof o.taskId === 'string' && typeof o.text === 'string' && typeof o.createdMs === 'number' ? { v: 1, taskId: o.taskId, text: o.text, createdMs: o.createdMs } : null
}

/** A task's comments, oldest first. Each is a `comment` document naming its task. */
export function Comments({ taskId, disabled }: { taskId: string; disabled: boolean }) {
  const { api } = useServices()
  const { notify } = useActions()
  const [items, setItems] = useState<Comment[]>([])
  const [text, setText] = useState('')

  useEffect(() => {
    let live = true
    api
      .listDocs('comment')
      .then((docs) => {
        if (!live) return
        const mine = docs.flatMap((d) => {
          const b = asBody(d.body)
          return b && b.taskId === taskId ? [{ id: d.id, ...b }] : []
        })
        setItems(mine.sort((a, b) => a.createdMs - b.createdMs))
      })
      .catch(() => notify('error', 'Could not load comments'))
    return () => {
      live = false
    }
  }, [api, notify, taskId])

  const add = (e: FormEvent): void => {
    e.preventDefault()
    const body: CommentBody = { v: 1, taskId, text: text.trim(), createdMs: Date.now() }
    if (!body.text) return
    const id = newId()
    setItems((cur) => [...cur, { id, ...body }])
    setText('')
    api.putDoc('comment', id, body).catch(() => {
      setItems((cur) => cur.filter((c) => c.id !== id))
      notify('error', 'Could not save the comment')
    })
  }

  const remove = (c: Comment): void => {
    setItems((cur) => cur.filter((x) => x.id !== c.id))
    api.deleteDoc('comment', c.id).catch(() => {
      setItems((cur) => [...cur, c].sort((a, b) => a.createdMs - b.createdMs))
      notify('error', 'Could not delete the comment')
    })
  }

  return (
    <section aria-label="Comments" className="flex flex-col gap-2 border-t border-line pt-3">
      <ul className="flex flex-col gap-2">
        {items.map((c) => (
          <li key={c.id} className="group flex items-start gap-2 rounded-row bg-side px-3 py-2">
            <div className="min-w-0 flex-1">
              <p className="whitespace-pre-wrap break-words">{c.text}</p>
              <time className="text-s text-grey" dateTime={new Date(c.createdMs).toISOString()}>
                {new Date(c.createdMs).toLocaleString('en-US', { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}
              </time>
            </div>
            {!disabled && (
              <button type="button" aria-label="Delete comment" onClick={() => remove(c)} className="hidden h-6 w-6 items-center justify-center rounded text-grey hover:bg-hover group-focus-within:flex group-hover:flex">
                <Trash2 size={14} />
              </button>
            )}
          </li>
        ))}
      </ul>
      {!disabled && (
        <form onSubmit={add} className="flex gap-2">
          <input aria-label="Add a comment" value={text} maxLength={2000} onChange={(e) => setText(e.target.value)} placeholder="Add a comment" className="h-8 min-w-0 flex-1 rounded-row border border-line bg-surface px-3 outline-none focus:border-primary" />
          <button type="submit" disabled={!text.trim()} className="h-8 rounded-row bg-primary px-4 text-white disabled:opacity-40">
            Send
          </button>
        </form>
      )}
    </section>
  )
}

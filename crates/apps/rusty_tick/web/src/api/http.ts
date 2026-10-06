/** The real backend: `rusty_tick`'s JSON API, same-origin, bearer-token auth. */
import type { ApiClient } from './client'
import { ApiError, ConflictError, InvalidError, NetworkError, NotFoundError, StaleError, UnauthorizedError } from './errors'
import type { Doc, DocKind, List, ListPatch, NewList, NewTask, Snapshot, Tag, TagPatch, Task, TaskPatch } from './types'

export interface HttpOptions {
  /** Prefix for every request; empty means same-origin. */
  baseUrl?: string
  getToken: () => string | null
  fetchImpl?: typeof fetch
  timeoutMs?: number
}

interface ErrorBody {
  error?: { code?: string; message?: string }
  current?: unknown
}

export class HttpAdapter implements ApiClient {
  private readonly base: string
  private readonly fetchImpl: typeof fetch
  private readonly timeoutMs: number

  constructor(private readonly options: HttpOptions) {
    this.base = `${options.baseUrl ?? ''}/api/v1`
    this.fetchImpl = options.fetchImpl ?? ((...args) => fetch(...args))
    this.timeoutMs = options.timeoutMs ?? 15_000
  }

  private async request<T>(method: string, path: string, body?: unknown, etag?: string): Promise<T> {
    const headers: Record<string, string> = { Accept: 'application/json' }
    const token = this.options.getToken()
    if (token) headers.Authorization = `Bearer ${token}`
    if (body !== undefined) headers['Content-Type'] = 'application/json'
    if (etag !== undefined) headers['If-Match'] = `"${etag}"`

    const controller = new AbortController()
    const timer = setTimeout(() => controller.abort(), this.timeoutMs)
    let response: Response
    try {
      response = await this.fetchImpl(this.base + path, {
        method,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: controller.signal,
        cache: 'no-store',
      })
    } catch {
      throw new NetworkError() // refused, offline, DNS, or timed out
    } finally {
      clearTimeout(timer)
    }
    if (response.status === 204) return undefined as T
    const text = await response.text()
    const data = text ? (JSON.parse(text) as unknown) : undefined
    if (!response.ok) throw toError(response.status, data as ErrorBody | undefined)
    return data as T
  }

  snapshot(): Promise<Snapshot> {
    return this.request('GET', '/snapshot')
  }

  createList(input: NewList): Promise<List> {
    return this.request('POST', '/lists', input)
  }
  updateList(id: string, patch: ListPatch, etag?: string): Promise<List> {
    return this.request('PATCH', `/lists/${id}`, patch, etag)
  }
  deleteList(id: string): Promise<void> {
    return this.request('DELETE', `/lists/${id}`)
  }

  createTask(input: NewTask): Promise<Task> {
    return this.request('POST', '/tasks', input)
  }
  updateTask(id: string, patch: TaskPatch, etag?: string): Promise<Task> {
    return this.request('PATCH', `/tasks/${id}`, patch, etag)
  }
  trashTask(id: string, etag?: string): Promise<void> {
    return this.request('DELETE', `/tasks/${id}`, undefined, etag)
  }
  restoreTask(id: string): Promise<Task> {
    return this.request('POST', `/tasks/${id}/restore`)
  }
  purgeTask(id: string): Promise<void> {
    return this.request('DELETE', `/tasks/${id}?permanent=true`)
  }
  async emptyTrash(): Promise<number> {
    const r = await this.request<{ purged: number }>('DELETE', '/trash')
    return r.purged
  }

  createTag(label: string, color: string | null = null): Promise<Tag> {
    return this.request('POST', '/tags', { label, color })
  }
  updateTag(name: string, patch: TagPatch, etag?: string): Promise<Tag> {
    return this.request('PATCH', `/tags/${encodeURIComponent(name)}`, patch, etag)
  }
  renameTag(name: string, label: string): Promise<Tag> {
    return this.request('POST', `/tags/${encodeURIComponent(name)}/rename`, { label })
  }
  deleteTag(name: string): Promise<void> {
    return this.request('DELETE', `/tags/${encodeURIComponent(name)}`)
  }

  async listDocs<T = unknown>(kind: DocKind): Promise<Doc<T>[]> {
    const r = await this.request<{ docs: Doc<T>[] }>('GET', `/docs/${kind}`)
    return r.docs
  }
  putDoc<T = unknown>(kind: DocKind, id: string, body: T): Promise<Doc<T>> {
    return this.request('PUT', `/docs/${kind}/${id}`, body)
  }
  deleteDoc(kind: DocKind, id: string): Promise<void> {
    return this.request('DELETE', `/docs/${kind}/${id}`)
  }

  async fetchIcs(url: string): Promise<string> {
    return (await this.request<{ text: string }>('POST', '/fetch-ics', { url })).text
  }
}

function toError(status: number, body: ErrorBody | undefined): Error {
  const message = body?.error?.message ?? `request failed (${status})`
  switch (status) {
    case 401:
      return new UnauthorizedError(message)
    case 404:
      return new NotFoundError(message)
    case 409:
      return new ConflictError(message)
    case 412:
      return new StaleError(body?.current)
    case 400:
    case 422:
      return new InvalidError(status, message)
    default:
      return new ApiError(status, body?.error?.code ?? 'internal', message)
  }
}

/** The real backend: `rusty_fair_play`'s JSON API, same-origin, optional bearer token. */
import type { ApiClient } from './client'
import { ApiError, ConflictError, InvalidError, NetworkError, NotFoundError, StaleError, UnauthorizedError } from './errors'
import type { BaselineResponse, Card, CardPatch, NewCard, Person, SeedResult, Snapshot, SplitInput, SplitResult, UnsplitResult } from './types'

export interface HttpOptions {
  /** Prefix for every request; empty means same-origin. */
  baseUrl?: string
  getToken: () => string | null
  fetchImpl?: typeof fetch
  timeoutMs?: number
}

interface ErrorBody {
  error?: { code?: string; message?: string }
  /** On a 412: the card as stored now. */
  current?: Card
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
    if (etag !== undefined) headers['If-Match'] = etag === '*' ? '*' : `"${etag}"`

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
  seed(): Promise<SeedResult> {
    return this.request('POST', '/seed')
  }

  createPerson(name: string): Promise<Person> {
    return this.request('POST', '/people', { name })
  }
  renamePerson(id: string, name: string): Promise<Person> {
    return this.request('PATCH', `/people/${id}`, { name })
  }
  deletePerson(id: string): Promise<void> {
    return this.request('DELETE', `/people/${id}`)
  }

  getCard(id: string): Promise<Card> {
    return this.request('GET', `/cards/${id}`)
  }
  updateCard(id: string, patch: CardPatch, etag?: string): Promise<Card> {
    return this.request('PATCH', `/cards/${id}`, patch, etag)
  }
  createCard(input: NewCard): Promise<Card> {
    return this.request('POST', '/cards', input)
  }
  split(id: string, input: SplitInput, etag?: string): Promise<SplitResult> {
    return this.request('POST', `/cards/${id}/split`, input, etag)
  }
  reset(id: string, etag?: string): Promise<Card> {
    return this.request('POST', `/cards/${id}/reset`, undefined, etag)
  }
  baseline(id: string): Promise<BaselineResponse> {
    return this.request('GET', `/cards/${id}/baseline`)
  }
  setPosition(id: string, position: number, etag?: string): Promise<void> {
    return this.request('PUT', `/cards/${id}/position`, { position }, etag)
  }
  deleteCard(id: string, etag?: string): Promise<void> {
    return this.request('DELETE', `/cards/${id}`, undefined, etag)
  }
  unsplitCard(id: string, treeEtag?: string): Promise<UnsplitResult> {
    return this.request('POST', `/cards/${id}/unsplit`, undefined, treeEtag)
  }
  async reorderChildren(parentId: string, ids: string[], treeEtag?: string): Promise<Card[]> {
    const { cards } = await this.request<{ cards: Card[] }>('PUT', `/cards/${parentId}/children/order`, { ids }, treeEtag)
    return cards
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
      if (body?.current) return new StaleError(body.current, message)
      return new ApiError(412, 'precondition_failed', message)
    case 400:
    case 422:
      return new InvalidError(status, message)
    default:
      return new ApiError(status, body?.error?.code ?? 'internal', message)
  }
}

import { afterEach, describe, expect, it } from 'vitest'
import { startBackend, TOKEN, type Backend } from '@/test/backend'
import { runApiContract } from './contract'
import { UnauthorizedError, NetworkError } from './errors'
import { HttpAdapter } from './http'

const running: Backend[] = []
afterEach(async () => {
  await Promise.all(running.splice(0).map((b) => b.stop()))
})

const adapter = async (token: string | null = TOKEN) => {
  const backend = await startBackend()
  running.push(backend)
  return new HttpAdapter({ baseUrl: backend.url, getToken: () => token })
}

// The same behaviour the in-browser MemoryAdapter is held to, against the real server.
runApiContract('HttpAdapter + rusty_tick', () => adapter())

describe('HttpAdapter transport', () => {
  it('maps a missing or wrong token to UnauthorizedError', async () => {
    await expect((await adapter(null)).snapshot()).rejects.toBeInstanceOf(UnauthorizedError)
    await expect((await adapter('wrong-token-0123456789')).snapshot()).rejects.toBeInstanceOf(UnauthorizedError)
  })

  it('maps an unreachable server to NetworkError', async () => {
    const api = new HttpAdapter({ baseUrl: 'http://127.0.0.1:1', getToken: () => TOKEN, timeoutMs: 2000 })
    await expect(api.snapshot()).rejects.toBeInstanceOf(NetworkError)
  })

  it('round-trips awkward tag names through the URL', async () => {
    const api = await adapter()
    const list = await api.createList({ name: 'L' })
    await api.createTask({ listId: list.id, title: 't', tags: ['c++', 'a/b', 'café', 'x y'] })
    for (const name of ['c++', 'a/b', 'café', 'x y']) {
      const tag = await api.updateTag(name, { color: '#123456' })
      expect(tag.name).toBe(name)
    }
  })
})

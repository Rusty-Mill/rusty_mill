import { afterEach, describe, expect, it } from 'vitest'
import { startBackend, TOKEN, type Backend } from '@/test/backend'
import { runApiContract } from './contract'
import { NetworkError, UnauthorizedError } from './errors'
import { HttpAdapter } from './http'

const running: Backend[] = []
afterEach(async () => {
  await Promise.all(running.splice(0).map((b) => b.stop()))
})

const adapter = async (options: { token?: string | null; serverToken?: string | null } = {}) => {
  const backend = await startBackend({ token: options.serverToken })
  running.push(backend)
  return new HttpAdapter({ baseUrl: backend.url, getToken: () => options.token ?? null })
}

// The same behaviour the in-browser MemoryAdapter is held to, against the real server (no token).
runApiContract('HttpAdapter + rusty_fair_play', () => adapter())

describe('HttpAdapter transport', () => {
  it('serves the full deck on first start', async () => {
    const api = await adapter()
    expect((await api.snapshot()).cards).toHaveLength(100)
  })

  it('maps a missing or wrong token to UnauthorizedError when the server has one', async () => {
    await expect((await adapter({ serverToken: TOKEN })).snapshot()).rejects.toBeInstanceOf(UnauthorizedError)
    await expect((await adapter({ serverToken: TOKEN, token: 'wrong-token-0123456789' })).snapshot()).rejects.toBeInstanceOf(UnauthorizedError)
    expect((await (await adapter({ serverToken: TOKEN, token: TOKEN })).snapshot()).cards).toHaveLength(100)
  })

  it('maps an unreachable server to NetworkError', async () => {
    const api = new HttpAdapter({ baseUrl: 'http://127.0.0.1:1', getToken: () => null, timeoutMs: 2000 })
    await expect(api.snapshot()).rejects.toBeInstanceOf(NetworkError)
  })
})

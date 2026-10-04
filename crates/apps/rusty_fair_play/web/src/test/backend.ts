/**
 * Start the real `rusty_fair_play` binary for integration tests: a fresh data
 * directory and a free loopback port per instance, torn down afterwards.
 * Build it first: `cargo build -p rusty_fair_play` (or set RUSTY_FAIR_PLAY_BIN).
 */
import { spawn, type ChildProcess } from 'node:child_process'
import { existsSync, mkdtempSync, rmSync } from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export const TOKEN = 'integration-test-token-0123'

export interface Backend {
  url: string
  stop(): Promise<void>
}

// src/test -> web -> rusty_fair_play -> crates -> rusty_multimodal_db -> apps -> crates -> repo root
const defaultBin = resolve(fileURLToPath(new URL('.', import.meta.url)), '../../../../../../target/debug/rusty_fair_play')

function freePort(): Promise<number> {
  return new Promise((ok, err) => {
    const server = createServer()
    server.once('error', err)
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address() as { port: number }
      server.close(() => ok(port))
    })
  })
}

async function waitForHealth(url: string, child: ChildProcess): Promise<void> {
  for (let i = 0; i < 200; i++) {
    if (child.exitCode !== null) throw new Error(`rusty_fair_play exited early (${child.exitCode})`)
    try {
      if ((await fetch(`${url}/health`)).ok) return
    } catch {
      /* not listening yet */
    }
    await new Promise((r) => setTimeout(r, 50))
  }
  throw new Error('rusty_fair_play did not become healthy')
}

export interface BackendOptions {
  webDir?: string
  /** Require this bearer token; null/undefined = open (loopback only). */
  token?: string | null
  noSeed?: boolean
}

export async function startBackend(options: BackendOptions = {}): Promise<Backend> {
  const bin = process.env.RUSTY_FAIR_PLAY_BIN ?? defaultBin
  if (!existsSync(bin)) throw new Error(`${bin} not found; run \`cargo build -p rusty_fair_play\``)
  const dir = mkdtempSync(join(tmpdir(), 'fair-play-it-'))
  const port = await freePort()
  const args = ['--data-dir', dir, '--addr', `127.0.0.1:${port}`]
  if (options.webDir) args.push('--web-dir', options.webDir)
  if (options.noSeed) args.push('--no-seed')
  const env = { ...process.env }
  delete env.RUSTY_FAIR_PLAY_TOKEN
  if (options.token) env.RUSTY_FAIR_PLAY_TOKEN = options.token
  const child = spawn(bin, args, { env, stdio: 'ignore' })
  const url = `http://127.0.0.1:${port}`
  try {
    await waitForHealth(url, child)
  } catch (e) {
    child.kill()
    rmSync(dir, { recursive: true, force: true })
    throw e
  }
  return {
    url,
    async stop() {
      child.kill()
      await new Promise((r) => child.once('exit', r))
      rmSync(dir, { recursive: true, force: true })
    },
  }
}

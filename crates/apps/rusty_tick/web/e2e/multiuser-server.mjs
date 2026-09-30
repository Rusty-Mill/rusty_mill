// Starts rusty_tick in multi-user mode (ADR-0002) with one user, alice, for the e2e run.
// Her token is written to .e2e-users/alice.token (gitignored) for the tests to read.
import { spawn, spawnSync } from 'node:child_process'
import { mkdirSync, rmSync, writeFileSync } from 'node:fs'

const [bin, port] = process.argv.slice(2)
const dir = '.e2e-users'
rmSync(dir, { recursive: true, force: true })
mkdirSync(dir, { recursive: true })

const added = spawnSync(bin, ['user', 'add', 'alice', '--data-dir', dir], { encoding: 'utf8' })
const token = /alice\.[A-Za-z0-9_-]+/.exec(added.stdout)?.[0]
if (!token) throw new Error('rusty_tick user add printed no token')
writeFileSync(`${dir}/alice.token`, token, { mode: 0o600 })

const server = spawn(bin, ['--data-dir', dir, '--addr', `127.0.0.1:${port}`, '--web-dir', 'dist'], { stdio: 'inherit', env: { ...process.env, RUSTY_TICK_TOKEN: undefined } })
for (const sig of ['SIGTERM', 'SIGINT']) process.on(sig, () => server.kill(sig))
server.on('exit', (code) => process.exit(code ?? 0))

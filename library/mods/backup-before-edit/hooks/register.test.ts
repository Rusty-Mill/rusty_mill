import { expect, test, mock } from 'claude-code/testing'

const ok = { result: {}, isReadOnly: false } as never
const HOME = '/home/u'

// This runner is POSIX, so the engine treats `C:\\work\\a.ts` as relative and prefixes the cwd.
// Strip that prefix so Windows-style paths reach the fake fs as written.
const norm = (p: string) => p.replace(/^.*?(?=[A-Za-z]:\\)/, '')

/** An in-memory file system standing for the engine's, beneath the plugin. */
function memfs(on: any, files: Record<string, string>): Record<string, string> {
  on('fs.read', (_$: unknown, e: { path: string }) => {
    const path = norm(e.path)
    return path in files ? { value: files[path] } : { deny: `ENOENT ${path}` }
  })
  on('fs.write', (_$: unknown, e: { path: string; text: string }) => {
    files[norm(e.path)] = e.text
    return { value: undefined }
  })
  return files
}

const backupsOf = (files: Record<string, string>) =>
  Object.entries(files).filter(([p]) => p.includes('/.claude/backups/'))

async function edit($: any, tool: 'Edit' | 'Write', file_path: string) {
  await $.tool.call({ tool, file_path, old_string: 'a', new_string: 'b', content: 'b' } as never)
}

for (const tool of ['Edit', 'Write'] as const) {
  test(`${tool}: backs up under the home dir, then lets the call through`, async ($, on) => {
    mock.env(on, { HOME })
    const files = memfs(on, { '/work/src/a.ts': 'before' })
    let reached = false
    on('tool.call', { tool }, () => { reached = true; return ok })
    await edit($, tool, '/work/src/a.ts')
    const [backup] = backupsOf(files)
    expect(backup[0].startsWith(`${HOME}/.claude/backups/`)).toBe(true)
    expect(backup[0].endsWith('__work__src__a.ts')).toBe(true)
    expect(backup[1]).toBe('before')
    expect(reached).toBe(true)
  })
}

test('a file outside the project is backed up under home, never beside it', async ($, on) => {
  mock.env(on, { HOME })
  const files = memfs(on, { '/etc/app.conf': 'x' })
  on('tool.call', { tool: 'Edit' }, () => ok)
  await edit($, 'Edit', '/etc/app.conf')
  expect(backupsOf(files).every(([p]) => p.startsWith(`${HOME}/`))).toBe(true)
  expect(backupsOf(files).length).toBe(1)
})

const WIN = 'C:\\Users\\u'
const SECRETS = [
  '/work/.env', '/work/.env.local', '/work/keys/server.pem', '/home/u/.ssh/id_ed25519', '/work/credentials.json',
  // Windows spellings of the same files, and an upper-case name.
  'C:\\work\\.env', 'C:\\work\\.env.local', 'C:\\work\\keys\\server.pem', `${WIN}\\.ssh\\id_ed25519`, 'C:\\work\\credentials.json', 'C:\\work\\.ENV',
]
for (const secret of SECRETS) {
  test(`never copies a secret-looking file: ${secret}`, async ($, on) => {
    mock.env(on, { HOME })
    const files = memfs(on, { [secret]: 'TOKEN=1' })
    let reached = false
    on('tool.call', { tool: 'Edit' }, () => { reached = true; return ok })
    await edit($, 'Edit', secret)
    expect(backupsOf(files)).toEqual([])
    expect(reached).toBe(true)
  })
}

test('no home directory: writes nothing, call still goes through', async ($, on) => {
  mock.env(on, {})
  const files = memfs(on, { '/work/a.ts': 'x' })
  let reached = false
  on('tool.call', { tool: 'Edit' }, () => { reached = true; return ok })
  await edit($, 'Edit', '/work/a.ts')
  expect(Object.keys(files)).toEqual(['/work/a.ts'])
  expect(reached).toBe(true)
})

test('a missing file writes no backup and the call still goes through', async ($, on) => {
  mock.env(on, { HOME })
  const files = memfs(on, {})
  let reached = false
  on('tool.call', { tool: 'Write' }, () => { reached = true; return ok })
  await edit($, 'Write', '/work/new.txt')
  expect(backupsOf(files)).toEqual([])
  expect(reached).toBe(true)
})

test('Windows path: backed up under a Windows home, with a flat name and no doubled separator', async ($, on) => {
  mock.env(on, { USERPROFILE: `${WIN}\\` })
  const src = 'C:\\work\\src\\a.ts'
  const files = memfs(on, { [src]: 'before' })
  on('tool.call', { tool: 'Edit' }, () => ok)
  await edit($, 'Edit', src)
  const [backup] = backupsOf(files)
  expect(backup[0].startsWith(`${WIN}/.claude/backups/`)).toBe(true)
  expect(backup[0].endsWith('__C__work__src__a.ts')).toBe(true)
  expect(backup[0].slice(WIN.length + 1)).not.toContain('\\')
  expect(backup[1]).toBe('before')
})

test('names that only look like secrets are still backed up (".environment", "monkey.txt")', async ($, on) => {
  mock.env(on, { HOME })
  const files = memfs(on, { '/work/.environment': 'a', 'C:\\work\\monkey.txt': 'b' })
  on('tool.call', { tool: 'Edit' }, () => ok)
  await edit($, 'Edit', '/work/.environment')
  await edit($, 'Edit', 'C:\\work\\monkey.txt')
  expect(backupsOf(files).length).toBe(2)
})

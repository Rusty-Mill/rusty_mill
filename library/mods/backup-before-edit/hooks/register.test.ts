import { expect, test } from 'claude-code/testing'

const ok = { result: {}, isReadOnly: false } as never
const FILE = '/work/src/a.ts' // absolute: the engine resolves relative paths against the cwd

/** An in-memory file system standing for the engine's, beneath the plugin. */
function memfs(on: any, files: Record<string, string>): Record<string, string> {
  on('fs.read', (_$: unknown, e: { path: string }) => {
    return e.path in files ? { value: files[e.path] } : { deny: `ENOENT ${e.path}` }
  })
  on('fs.write', (_$: unknown, e: { path: string; text: string }) => {
    files[e.path] = e.text
    return { value: undefined }
  })
  return files
}

const backupsOf = (files: Record<string, string>) =>
  Object.entries(files).filter(([p]) => p.includes('/.claude/backups/'))

for (const tool of ['Edit', 'Write'] as const) {
  test(`${tool}: backs up the old content, then lets the call through`, async ($, on) => {
    const files = memfs(on, { [FILE]: 'before' })
    let reached = false
    on('tool.call', { tool }, () => { reached = true; return ok })
    await $.tool.call({ tool, file_path: FILE, old_string: 'before', new_string: 'x', content: 'x' } as never)
    const [backup] = backupsOf(files)
    expect(backup[0].endsWith('__work__src__a.ts')).toBe(true)
    expect(backup[1]).toBe('before')
    expect(reached).toBe(true)
  })
}

test('a missing file writes no backup and the call still goes through', async ($, on) => {
  const files = memfs(on, {})
  let reached = false
  on('tool.call', { tool: 'Write' }, () => { reached = true; return ok })
  await $.tool.call({ tool: 'Write', file_path: FILE, content: 'x' } as never)
  expect(backupsOf(files)).toEqual([])
  expect(reached).toBe(true)
})

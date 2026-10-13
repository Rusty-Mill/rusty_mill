import type { Register } from 'claude-code'

const DIR = '.claude/backups'

export const register: Register = on => {
  for (const tool of ['Edit', 'Write'] as const) {
    on('tool.call', { tool }, async ($, e, next) => {
      const stamp = new Date().toISOString().replace(/[:.]/g, '-')
      const name = e.file_path.replace(/^\/+/, '').replace(/\//g, '__')
      try {
        await $.fs.write(`${DIR}/${stamp}__${name}`, await $.fs.read(e.file_path))
      } catch {
        // New file or unreadable: nothing to back up.
      }
      return next(e)
    }).catch((_$, e, next) => next(e)) // fail open: a backup bug must not block edits
  }
}

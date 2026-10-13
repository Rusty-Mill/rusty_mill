import type { Register } from 'claude-code'

// Backups live in the user's home, never in the checkout, so `git add .` can't pick them up.
const SUBDIR = '.claude/backups'
// Files that usually hold secrets are never copied: a backup would just duplicate the secret.
const SECRET = /(^|\/)(\.env(\..*)?|\.netrc|credentials(\.json)?|id_(rsa|ed25519|ecdsa)|[^/]*\.(pem|key|p12|pfx))$/i

export const register: Register = on => {
  for (const tool of ['Edit', 'Write'] as const) {
    on('tool.call', { tool }, async ($, e, next) => {
      const home = (await $.env.get('HOME')) ?? (await $.env.get('USERPROFILE'))
      if (home && !SECRET.test(e.file_path)) {
        const stamp = new Date().toISOString().replace(/[:.]/g, '-')
        const name = e.file_path.replace(/^[/\\]+/, '').replace(/[/\\:]+/g, '__')
        try {
          await $.fs.write(`${home}/${SUBDIR}/${stamp}__${name}`, await $.fs.read(e.file_path))
        } catch {
          // New file or unreadable: nothing to back up.
        }
      }
      return next(e)
    }).catch((_$, e, next) => next(e)) // fail open: a backup bug must not block edits
  }
}

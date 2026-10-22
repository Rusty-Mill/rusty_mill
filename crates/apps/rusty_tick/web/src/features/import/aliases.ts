/**
 * Where a merged-away task now lives. Merging a duplicate into the task to
 * keep records the duplicate's id as an alias of the kept one (an
 * `import_alias` doc: its id is the duplicate's, its body names the kept
 * task). An import derives a task's id from its source, so without this a
 * later import would recreate the duplicate once it was purged from the Trash;
 * `applyImport` writes to the kept task instead.
 */
import type { ApiClient } from '@/api/client'

const KIND = 'import_alias'

interface AliasBody {
  to: string
}

const asAlias = (body: unknown): AliasBody | null => {
  const to = typeof body === 'object' && body !== null ? (body as { to?: unknown }).to : undefined
  return typeof to === 'string' ? { to } : null
}

/** Merged-away task id -> the id of the task kept in its place. */
export async function readAliases(api: ApiClient): Promise<Map<string, string>> {
  const docs = await api.listDocs(KIND)
  return new Map(
    docs.flatMap((d) => {
      const alias = asAlias(d.body)
      return alias ? [[d.id, alias.to] as const] : []
    }),
  )
}

/**
 * Record that `duplicateId` is now `keepId`: aliases that named the duplicate
 * follow it, and one that named the kept task away is dropped (it is live
 * again), so aliases never chain or loop. The caller trashes the duplicate.
 */
export async function recordMerge(api: ApiClient, duplicateId: string, keepId: string): Promise<void> {
  const aliases = await readAliases(api)
  for (const [from, to] of aliases) if (to === duplicateId) await api.putDoc<AliasBody>(KIND, from, { to: keepId })
  if (aliases.has(keepId)) await api.deleteDoc(KIND, keepId)
  await api.putDoc<AliasBody>(KIND, duplicateId, { to: keepId })
}

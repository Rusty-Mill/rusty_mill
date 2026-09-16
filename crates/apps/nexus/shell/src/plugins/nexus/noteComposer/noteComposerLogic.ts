// RFC 0009 — pure logic for the note-composer plugin, kept separate from
// index.ts so it's testable without the plugin stack (mirrors
// uniqueNote/uniqueNoteSettings.ts, dailyNotes/dailyNoteFormat.ts).

/** Persisted by Settings → Core plugins → Note composer (SettingsStubPages.tsx). */
export const CONFIG_KEY_TEXT_AFTER_EXTRACTION = 'nexus.settings.noteComposer.textAfterExtraction'
export const CONFIG_KEY_CONFIRM_MERGE = 'nexus.settings.noteComposer.confirmMerge'
/** Shared with the editor's rename flow (C2 #355). */
export const CONFIG_KEY_LINKS_AUTO_UPDATE = 'nexus.settings.links.autoUpdate'
/** Shared with the editor's delete flow (C3 #356). */
export const CONFIG_KEY_DELETED_FILES_DESTINATION = 'nexus.settings.files.deletedFilesDestination'

export type TextAfterExtraction = 'link' | 'embed' | 'nothing'

/** Wire value accepted by `com.nexus.storage::note_merge.destination`. */
export type MergeDestination = 'forge' | 'system' | 'permanent'

export interface NoteComposerSettings {
  textAfterExtraction: TextAfterExtraction
  confirmMerge: boolean
  updateLinks: boolean
  destination: MergeDestination
}

type GetValue = <T>(key: string, defaultValue: T) => T

export function readNoteComposerSettings(getValue: GetValue): NoteComposerSettings {
  const rawMode = getValue<string>(CONFIG_KEY_TEXT_AFTER_EXTRACTION, 'link')
  const rawDest = getValue<string>(CONFIG_KEY_DELETED_FILES_DESTINATION, 'system')
  return {
    textAfterExtraction: isTextAfterExtraction(rawMode) ? rawMode : 'link',
    confirmMerge: getValue<boolean>(CONFIG_KEY_CONFIRM_MERGE, true),
    updateLinks: getValue<boolean>(CONFIG_KEY_LINKS_AUTO_UPDATE, true),
    // The files setting stores 'system' | 'forge' | 'permanent' (C3 #356);
    // anything unexpected falls back to the recoverable forge trash.
    destination: isMergeDestination(rawDest) ? rawDest : 'forge',
  }
}

function isTextAfterExtraction(v: string): v is TextAfterExtraction {
  return v === 'link' || v === 'embed' || v === 'nothing'
}

function isMergeDestination(v: string): v is MergeDestination {
  return v === 'forge' || v === 'system' || v === 'permanent'
}

/** Basename of a forge-relative path. Forward-slash only. */
export function basename(relpath: string): string {
  const i = relpath.lastIndexOf('/')
  return i === -1 ? relpath : relpath.slice(i + 1)
}

/** `notes/My Idea.md` → `My Idea`. The stem is what `[[…]]` resolves on. */
export function stemOf(relpath: string): string {
  const base = basename(relpath)
  return base.toLowerCase().endsWith('.md') ? base.slice(0, -3) : base
}

/**
 * Text that replaces the extracted selection in the source note, per the
 * `textAfterExtraction` setting. Wikilinks use the stem so they resolve
 * through the index's stem tier regardless of folder.
 */
export function replacementForExtraction(mode: TextAfterExtraction, newNotePath: string): string {
  const stem = stemOf(newNotePath)
  switch (mode) {
    case 'link':
      return `[[${stem}]]`
    case 'embed':
      return `![[${stem}]]`
    case 'nothing':
      return ''
  }
}

/** Candidate merge targets: every other markdown note, sorted by path. */
export function mergeTargetCandidates(paths: readonly string[], source: string): string[] {
  return paths
    .filter((p) => p !== source && p.toLowerCase().endsWith('.md'))
    .sort((a, b) => a.localeCompare(b))
}

// RFC 0009 — pure settings/argument shaping for the unique-note plugin,
// kept separate from index.ts so it's testable without the plugin stack
// (mirrors dailyNotes/dailyNoteFormat.ts).

export const CONFIG_KEY_ID_FORMAT = 'nexus.settings.uniqueNote.idFormat'
export const CONFIG_KEY_SEPARATOR = 'nexus.settings.uniqueNote.separator'
export const CONFIG_KEY_FILE_LOCATION = 'nexus.settings.uniqueNote.fileLocation'

/** chrono `strftime` template the storage engine defaults to. */
export const DEFAULT_ID_FORMAT = '%Y%m%d%H%M%S'
export const DEFAULT_SEPARATOR = ' '

export interface UniqueNoteSettings {
  idFormat: string
  separator: string
  /** Forge-relative folder; empty string means the forge root. */
  fileLocation: string
}

/** Args shape for `com.nexus.storage::note_create_unique`. */
export interface NoteCreateUniqueArgs {
  title: string
  id_format?: string
  separator?: string
  folder?: string
}

type GetValue = <T>(key: string, defaultValue: T) => T

/**
 * Read the three unique-note settings, normalising blanks back to the
 * engine defaults. `separator` is deliberately not trimmed: a single
 * space is the default and a legitimate choice.
 */
export function readUniqueNoteSettings(getValue: GetValue): UniqueNoteSettings {
  const idFormat = getValue<string>(CONFIG_KEY_ID_FORMAT, DEFAULT_ID_FORMAT).trim()
  const separator = getValue<string>(CONFIG_KEY_SEPARATOR, DEFAULT_SEPARATOR)
  const fileLocation = getValue<string>(CONFIG_KEY_FILE_LOCATION, '').trim()
  return {
    idFormat: idFormat || DEFAULT_ID_FORMAT,
    separator: separator === '' ? DEFAULT_SEPARATOR : separator,
    fileLocation: fileLocation.replace(/^\/+|\/+$/g, ''),
  }
}

/**
 * Build the IPC args. Values equal to the engine defaults are omitted so
 * the wire payload stays minimal and the Rust side owns the defaults.
 */
export function buildCreateArgs(title: string, settings: UniqueNoteSettings): NoteCreateUniqueArgs {
  const args: NoteCreateUniqueArgs = { title: title.trim() }
  if (settings.idFormat !== DEFAULT_ID_FORMAT) args.id_format = settings.idFormat
  if (settings.separator !== DEFAULT_SEPARATOR) args.separator = settings.separator
  if (settings.fileLocation) args.folder = settings.fileLocation
  return args
}

/** Basename of a forge-relative path. Forward-slash only. */
export function basename(relpath: string): string {
  const i = relpath.lastIndexOf('/')
  return i === -1 ? relpath : relpath.slice(i + 1)
}

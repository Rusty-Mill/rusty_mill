/**
 * UUIDv7: 48-bit millisecond timestamp, then random bits. Time-ordered like
 * the ids the backend mints, so a client-chosen id sorts with server ones.
 */
let lastMs = 0
let seq = 0

export function newId(now: number = Date.now()): string {
  // Within one millisecond, count up so ids from one client stay ordered.
  if (now <= lastMs) {
    seq += 1
    now = lastMs
  } else {
    lastMs = now
    seq = 0
  }
  const bytes = new Uint8Array(16)
  crypto.getRandomValues(bytes)
  const ts = BigInt(now)
  for (let i = 0; i < 6; i++) bytes[i] = Number((ts >> BigInt(8 * (5 - i))) & 0xffn)
  bytes[6] = 0x70 | ((seq >> 8) & 0x0f) // version 7, counter high bits
  bytes[7] = seq & 0xff
  bytes[8] = (bytes[8]! & 0x3f) | 0x80 // RFC 4122 variant
  const hex = [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('')
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

export function isUuid(text: string): boolean {
  return UUID.test(text)
}

/** cyrb128: a small, well-mixed 128-bit string hash (four 32-bit words). Not cryptographic; it only needs to be stable. */
function hash128(text: string): [number, number, number, number] {
  let h1 = 1779033703, h2 = 3144134277, h3 = 1013904242, h4 = 2773480762
  for (let i = 0; i < text.length; i++) {
    const k = text.charCodeAt(i)
    h1 = h2 ^ Math.imul(h1 ^ k, 597399067)
    h2 = h3 ^ Math.imul(h2 ^ k, 2869860233)
    h3 = h4 ^ Math.imul(h3 ^ k, 951274213)
    h4 = h1 ^ Math.imul(h4 ^ k, 2716044179)
  }
  h1 = Math.imul(h3 ^ (h1 >>> 18), 597399067)
  h2 = Math.imul(h4 ^ (h2 >>> 22), 2869860233)
  h3 = Math.imul(h1 ^ (h3 >>> 17), 951274213)
  h4 = Math.imul(h2 ^ (h4 >>> 19), 2716044179)
  return [(h1 ^ h2 ^ h3 ^ h4) >>> 0, (h2 ^ h1) >>> 0, (h3 ^ h1) >>> 0, (h4 ^ h1) >>> 0]
}

/**
 * A UUID derived from `text`: the same text gives the same id on every device,
 * and different texts give different ones. UUID-shaped (version nibble 5, RFC
 * 4122 variant) because the server parses ids as UUIDs. Use it for a document
 * that is "the one of its kind for X", with the kind in the text, since the
 * server keeps one document per id whatever its kind.
 */
export function derivedId(text: string): string {
  const words = hash128(text)
  const hex = words.map((w) => w.toString(16).padStart(8, '0')).join('')
  const variant = ((parseInt(hex[16]!, 16) & 0x3) | 0x8).toString(16)
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-5${hex.slice(13, 16)}-${variant}${hex.slice(17, 20)}-${hex.slice(20, 32)}`
}

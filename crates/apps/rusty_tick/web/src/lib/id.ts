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

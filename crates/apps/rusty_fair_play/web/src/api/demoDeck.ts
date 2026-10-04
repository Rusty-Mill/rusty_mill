/**
 * The full deck for the in-browser demo and the unit tests: the same CSV the
 * binary ships, read at build time (Vite `?raw`), so the demo cannot drift.
 */
import deckCsv from '../../../../../libs/storage/rusty_fair_play_domain/data/fair-play-cards.csv?raw'
import { parseCsv, parseStandards } from '@/lib/csv'
import { SUITS, type Baseline, type Suit } from './types'

/** The baseline id the server mints for deck card `number`. */
export const baselineId = (number: number): string => `00000000-0000-4000-8000-${String(number).padStart(12, '0')}`

/** `number,name,suit,conception,planning,execution,minimum_standard_of_care,notes` → baselines, in deck order. */
export function parseDeck(csv: string): Baseline[] {
  const [header, ...rows] = parseCsv(csv)
  const col = (name: string): number => {
    const at = header?.indexOf(name) ?? -1
    if (at < 0) throw new Error(`deck CSV lacks a ${name} column`)
    return at
  }
  const [number, name, suit, conception, planning, execution, standards] = ['number', 'name', 'suit', 'conception', 'planning', 'execution', 'minimum_standard_of_care'].map(col)
  return rows.map((r) => {
    const n = Number(r[number!])
    const s = r[suit!] ?? ''
    if (!Number.isInteger(n) || !(SUITS as readonly string[]).includes(s)) throw new Error(`bad deck row ${r.join(',')}`)
    return { id: baselineId(n), number: n, name: r[name!] ?? '', suit: s as Suit, conception: r[conception!] ?? '', planning: r[planning!] ?? '', execution: r[execution!] ?? '', minimumStandardOfCare: parseStandards(r[standards!] ?? '') }
  })
}

export const DEMO_DECK: Baseline[] = parseDeck(deckCsv)

export const PATHS = {
  deck: '/deck',
  card: (id: string) => `/deck/${id}`,
  players: '/players',
  balance: '/balance',
} as const

export type Section = 'deck' | 'players' | 'balance'

export const sectionOf = (pathname: string): Section => (pathname.startsWith('/players') ? 'players' : pathname.startsWith('/balance') ? 'balance' : 'deck')

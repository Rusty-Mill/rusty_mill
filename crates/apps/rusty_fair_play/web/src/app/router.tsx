import { createHashRouter, Navigate, type RouteObject } from 'react-router-dom'
import { BalancePage } from '@/features/balance/BalancePage'
import { DeckPage } from '@/features/deck/DeckPage'
import { PlayersPage } from '@/features/players/PlayersPage'
import { PATHS } from './paths'
import { Shell } from './Shell'

/** Hash routes: `#/deck`, `#/deck/:cardId`, `#/players`, `#/balance`. Anything else goes to the deck. */
export const routes: RouteObject[] = [
  {
    element: <Shell />,
    children: [
      { path: 'deck/:cardId?', element: <DeckPage /> },
      { path: 'players', element: <PlayersPage /> },
      { path: 'balance', element: <BalancePage /> },
      { path: '*', element: <Navigate to={PATHS.deck} replace /> },
    ],
  },
]

export const createRouter = () => createHashRouter(routes)

// Dates are local-time logic; pin a DST-observing zone so tests are the same everywhere.
process.env.TZ = 'America/Chicago'

import '@testing-library/jest-dom/vitest'
import { afterEach } from 'vitest'
import { cleanup } from '@testing-library/react'

afterEach(() => {
  cleanup()
  localStorage.clear()
  sessionStorage.clear()
})

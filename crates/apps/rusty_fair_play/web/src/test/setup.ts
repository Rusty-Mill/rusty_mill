import '@testing-library/jest-dom/vitest'
import { afterEach } from 'vitest'
import { cleanup } from '@testing-library/react'
import { useUi } from '@/store/ui'

// useUi is a module singleton; start every test from the state it was born with.
const pristineUi = useUi.getState()

afterEach(() => {
  cleanup()
  localStorage.clear()
  sessionStorage.clear()
  useUi.setState(pristineUi, true)
})

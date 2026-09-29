import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitest/config'

// Runs the ApiClient contract against the real `rusty_tick` binary
// (built with `cargo build -p rusty_tick`); see src/test/backend.ts.
export default defineConfig({
  resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
  test: {
    environment: 'node',
    globals: true,
    include: ['src/**/*.integration.test.ts'],
    testTimeout: 30_000,
    hookTimeout: 60_000,
    pool: 'forks',
  },
})

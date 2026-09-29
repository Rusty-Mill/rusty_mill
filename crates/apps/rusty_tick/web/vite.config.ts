import react from '@vitejs/plugin-react'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitest/config'

// The dev server proxies the API to a locally running `rusty_tick`, so the UI
// and the API share an origin exactly as they do when the binary serves `dist/`.
const backend = process.env.TICK_BACKEND ?? 'http://127.0.0.1:8787'

export default defineConfig({
  plugins: [react()],
  resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
  server: { proxy: { '/api': backend, '/health': backend } },
  build: { sourcemap: true },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
    exclude: ['**/node_modules/**', 'src/**/*.integration.test.ts'], // these need the real binary: `npm run test:integration`
    css: false,
  },
})

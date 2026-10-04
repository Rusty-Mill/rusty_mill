import react from '@vitejs/plugin-react'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitest/config'

// The dev server proxies the API to a locally running `rusty_fair_play`, so the
// UI and the API share an origin exactly as they do when the binary serves `dist/`.
const backend = process.env.FAIR_PLAY_BACKEND ?? 'http://127.0.0.1:8790'

export default defineConfig({
  plugins: [react()],
  resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
  // The demo deck is the domain crate's CSV, imported `?raw` from outside web/; the dev server must be allowed to serve it.
  server: { proxy: { '/api': backend, '/health': backend }, fs: { allow: ['.', '../../../libs/storage/rusty_fair_play_domain/data'] } },
  build: { sourcemap: true },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
    exclude: ['**/node_modules/**', 'e2e/**', 'src/**/*.integration.test.ts'], // these need the real binary: `npm run test:integration`
    css: false,
  },
})

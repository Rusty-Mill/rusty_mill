import { defineConfig } from '@playwright/test'
import { existsSync } from 'node:fs'

// The sandbox ships a Chromium; elsewhere Playwright's own download is used.
const preinstalled = '/opt/pw-browsers/chromium-1194/chrome-linux/chrome'
const executablePath = process.env.CHROMIUM_PATH ?? (existsSync(preinstalled) ? preinstalled : undefined)

const PORT = 4599
export const TOKEN = 'e2e-token-0123456789abcdef'
const bin = process.env.RUSTY_TICK_BIN ?? '../../../../target/debug/rusty_tick'

export default defineConfig({
  testDir: 'e2e',
  fullyParallel: false,
  workers: 1,
  reporter: 'list',
  use: { baseURL: `http://127.0.0.1:${PORT}`, launchOptions: { executablePath } },
  // The real thing: the rusty_tick binary serving the built UI from a fresh data directory.
  webServer: {
    command: `rm -rf .e2e-data && ${bin} --data-dir .e2e-data --addr 127.0.0.1:${PORT} --web-dir dist`,
    url: `http://127.0.0.1:${PORT}/health`,
    env: { RUSTY_TICK_TOKEN: TOKEN },
    reuseExistingServer: false,
    timeout: 30_000,
  },
})

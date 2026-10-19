// End-to-end tests (`npm run test:e2e`): a real cfd-server (release build, CPU or GPU, an empty
// data directory) and the Vite dev server, on ports of their own, driven by Chromium.
import { defineConfig, devices } from '@playwright/test'

const API_PORT = 7879
const WEB_PORT = 5174

export default defineConfig({
  testDir: 'e2e',
  testMatch: '**/*.e2e.ts',
  timeout: 120_000,
  fullyParallel: false,
  workers: 1,
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    baseURL: `http://localhost:${WEB_PORT}`,
    viewport: { width: 1440, height: 900 },
    trace: 'retain-on-failure',
    // Software WebGL (SwiftShader) everywhere: CI runners have no GPU, and headless Chromium no
    // longer falls back to it by itself; locally it keeps runs comparable with CI.
    launchOptions: { args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader'] },
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 } },
    },
  ],
  webServer: [
    {
      // Built by CI before the tests (or by the first local run: it takes a few minutes).
      // A fresh data directory each run (no earlier jobs or scenes), then the server.
      command: `node -e "require('fs').rmSync('target/e2e-data', { recursive: true, force: true })" && cargo run --release -p cfd-server -- --listen 127.0.0.1:${API_PORT} --data target/e2e-data`,
      url: `http://127.0.0.1:${API_PORT}/api/capabilities`,
      cwd: '../..',
      timeout: 600_000,
      reuseExistingServer: false,
      stdout: 'ignore',
    },
    {
      command: `npm run dev -- --port ${WEB_PORT} --strictPort`,
      url: `http://localhost:${WEB_PORT}`,
      env: { CFD_API: `http://127.0.0.1:${API_PORT}` },
      reuseExistingServer: false,
    },
  ],
})

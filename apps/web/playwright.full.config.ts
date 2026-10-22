// The @full end-to-end tests (`npm run test:e2e:full`): the Phase 5 acceptance run, which needs a
// GPU server and takes a few minutes, so CI does not run it. Same servers and browser as
// playwright.config.ts; only the tests selected and the time allowed differ.
import { defineConfig } from '@playwright/test'
import base from './playwright.config.ts'

export default defineConfig({
  ...base,
  grep: /@full/,
  grepInvert: undefined,
  timeout: 600_000,
  expect: { timeout: 30_000 },
})

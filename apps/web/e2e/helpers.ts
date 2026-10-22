// Helpers shared by the end-to-end tests: pixels of the viewer's canvas (through the
// development-only `window.__viewer` hook) and files saved through the browser's download.
import { readFileSync } from 'node:fs'
import type { Page } from '@playwright/test'

export type Pixel = [number, number, number, number]

/** The canvas pixel at the centre of a cell. */
export function pixel(page: Page, x: number, y: number): Promise<Pixel> {
  return page.evaluate(
    ([x, y]) =>
      (
        globalThis as unknown as { __viewer: { pixelAtCell(x: number, y: number): Pixel } }
      ).__viewer.pixelAtCell(x, y),
    [x, y],
  )
}

/** The editor's solid colour, as the viewer draws it. */
export const SOLID: Pixel = [71, 85, 105, 255]

/** Clicks a button that saves a file and returns the file's name and bytes. */
export async function saved(page: Page, button: string) {
  const [download] = await Promise.all([
    page.waitForEvent('download'),
    page.getByTestId(button).click(),
  ])
  return { name: download.suggestedFilename(), bytes: readFileSync((await download.path())!) }
}

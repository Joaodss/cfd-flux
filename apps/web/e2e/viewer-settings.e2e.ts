// The viewer's settings, the full-resolution "sharpen" and the reference-scale override, on a
// short cylinder run against a real cfd-server: a coarser viewer format (256 cells on a side:
// 221 × 42 blocks of the 442 × 84 grid) still shows the solid, a paused frame zoomed in is shown
// at full resolution, and a session's velocity scale changes C_D,max by 1/U².
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { expect, test } from '@playwright/test'
import { pixel, SOLID } from './helpers.ts'

const scene = JSON.parse(
  readFileSync(
    fileURLToPath(new URL('../../../scenes/cylinder-re100.json', import.meta.url)),
    'utf8',
  ),
)

/** The parts of the development-only viewer hook the test uses. */
interface ViewerHook {
  frame: { header: { width: number; height: number } } | null
}

const CYLINDER: [number, number] = [41, 41] // centre of the cylinder's disk
const WAKE: [number, number] = [70, 45] // just behind it

test('viewer format, full-resolution sharpen and reference-scale override', async ({
  page,
  request,
}) => {
  const r = await request.post('/api/simulations', {
    data: { scene, options: { endTime: 0.5, outputInterval: 0.02 } },
  })
  expect(r.status()).toBe(201)
  const { id } = (await r.json()) as { id: string }

  await page.goto(`/#/jobs/${id}`)
  await expect(page.getByTestId('job-status')).toHaveText('completed', { timeout: 60_000 })
  const size = page.getByTestId('viewer-size')
  // The default format (1024) holds every cell of the cylinder: nothing to sharpen.
  await expect(size).toHaveText('442 × 84')
  const streamed = () =>
    page.evaluate(
      () => (globalThis as unknown as { __viewer: ViewerHook }).__viewer.frame?.header.width,
    )

  // #17: 256 cells on a side — the stream reconnects and the frame is loaded again in blocks of
  // 2 × 2 cells.
  await page.getByTestId('viewer-settings').click()
  await page.getByTestId('viewer-max-size').selectOption('256')
  await expect(page.getByTestId('viewer-format')).toHaveText(/^221 × 42 blocks of 2 × 2 cells/)
  await page.keyboard.press('Escape')
  await expect.poll(streamed).toBe(221)
  // Zoomed out until a block is smaller than a pixel, the viewer format is what is shown.
  const canvas = page.getByTestId('viewer-canvas')
  const box = (await canvas.boundingBox())!
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  for (let i = 0; i < 6; i++) await page.mouse.wheel(0, 100)
  await expect(size).toHaveText('221 × 42')
  expect(await pixel(page, ...CYLINDER)).toEqual(SOLID)

  // #18: back to the fit (≈ 2 px per cell, 4 px per block) while paused: the frame is fetched
  // at full resolution and drawn instead.
  await page.getByRole('button', { name: 'Fit' }).click()
  await expect(size).toHaveText('442 × 84 (full resolution)')
  expect(await pixel(page, ...CYLINDER)).toEqual(SOLID)
  expect(await pixel(page, ...WAKE)).not.toEqual(SOLID)
  expect(await streamed()).toBe(221) // the overlays and the cache keep the viewer format
  // Playing goes back to the cached viewer format; pausing sharpens again.
  await page.getByLabel('Playback speed').selectOption('1')
  await page.getByTestId('timeline-play').click()
  await expect(size).toHaveText('221 × 42')
  await page.getByTestId('timeline-pause').click()
  await expect(size).toHaveText('442 × 84 (full resolution)')

  // The setting is remembered by the browser.
  await page.reload()
  await expect(page.getByTestId('viewer-frame')).toHaveText(/frame \d+/)
  await expect.poll(streamed).toBe(221)

  // #19: twice the reference velocity for this session: C_D,max / 4.
  await page.getByTestId('job-tab-stats').click()
  const cdMax = page.getByTestId('stat-cdMax')
  await expect(cdMax).toHaveText(/^C_D,max\d/)
  const value = async () => Number((await cdMax.locator('td').nth(1).textContent())!)
  const before = await value()
  const u = page.getByTestId('stats-scale-velocity')
  await expect(u).toHaveValue('1')
  await u.fill('2')
  await u.press('Enter')
  await expect(page.getByTestId('stats-scales-overridden')).toBeVisible()
  // The tab shows four significant digits: compare the ratio.
  await expect.poll(async () => (await value()) / before).toBeCloseTo(0.25, 3)
  await page.getByTestId('stats-scales-reset').click()
  await expect.poll(value).toBe(before)
})

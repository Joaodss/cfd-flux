// Phase 5 acceptance (docs/sessions/phase5-design-proposal.md §14), the MVP criterion end to end
// on the GPU: from the editor, open "Cylinder Re=100" and run it; the vortex street forms live in
// the job view; after the run, scrub back to the start and play it again; the statistics over the
// default window (the second half, 10–20 s) read St within 5 % of 0.300 and C_D,max within 5 % of
// 3.23 (C_L,max is for information); a PNG, the force CSV and a video of the street are exported.
//
// Tagged @full: the server needs a GPU (a 20 s run of the full example, 1,000 frames, ≈ 430 MB of
// results) and the run takes minutes, so CI leaves it out. Run it with `npm run test:e2e:full`.
import { readFileSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'
import { pixel, saved, SOLID } from './helpers.ts'

const ST_REFERENCE = 0.3
const CD_MAX_REFERENCE = 3.23
const TOLERANCE = 0.05 // the validation suite's tolerance at D = 20 cells

const CYLINDER: [number, number] = [41, 41] // centre of the cylinder's disk
const WIDTH = 442
const HEIGHT = 84

/** One look at the job view: the job status and the timeline's text at that moment. */
interface Observation {
  status: string
  position: string
}

/** Records the job view's status and timeline position at every change of the page (from the page). */
async function observeJobView(page: Page) {
  await page.addInitScript(() => {
    const seen: Observation[] = []
    ;(globalThis as unknown as { __seen: Observation[] }).__seen = seen
    const text = (id: string) =>
      document.querySelector(`[data-testid="${id}"]`)?.textContent?.trim() ?? ''
    new MutationObserver(() => {
      const status = text('job-status')
      const position = text('timeline-position')
      const last = seen[seen.length - 1]
      if (status && (!last || last.status !== status || last.position !== position)) {
        seen.push({ status, position })
      }
    }).observe(document, { subtree: true, childList: true, characterData: true })
  })
}

const seen = (page: Page) =>
  page.evaluate(() => (globalThis as unknown as { __seen: Observation[] }).__seen)

/** The sign of the vorticity (coolwarm: red positive, blue negative) at the centre of a cell. */
async function sign(page: Page, x: number, y: number): Promise<number> {
  const [r, , b] = await pixel(page, x, y)
  return Math.sign(r - b)
}

/** Sign changes of the vorticity along a row of cells, ignoring the near-zero colours. */
async function signChanges(page: Page, y: number, from: number, to: number) {
  let changes = 0
  let previous = 0
  for (let x = from; x < to; x += 3) {
    const s = await sign(page, x, y)
    if (s !== 0 && previous !== 0 && s !== previous) changes++
    if (s !== 0) previous = s
  }
  return changes
}

/** A statistic's value from the Statistics tab, and its check mark. */
async function statistic(page: Page, key: string) {
  const row = page.getByTestId(`stat-${key}`)
  const value = Number((await row.locator('td').nth(1).textContent())!.trim())
  const check = (await page.getByTestId(`stat-${key}-check`).textContent())!.trim()
  return { value, check }
}

test(
  'Cylinder Re=100 on the GPU: live street, replay, St and C_D,max, PNG, CSV and video',
  { tag: '@full' },
  async ({ page, request }) => {
    // The server must have a GPU: the quick variants and the CPU would not reach the 20 s.
    const capabilities = (await (await request.get('/api/capabilities')).json()) as {
      auto: string | null
    }
    expect(capabilities.auto, 'this test needs a GPU server').toMatch(/^(cuda|wgpu)$/)

    await observeJobView(page)
    page.on('dialog', (d) => d.accept()) // "run with these warnings?", if there are any
    const started = Date.now()

    // From the editor: the gallery, the full example (no quick variant on a GPU server), Run.
    await page.goto('/')
    await page.getByTestId('open-gallery').click()
    await page.getByTestId('example-cylinder-re100').click()
    await expect(page.getByTestId('scene-name')).toHaveText('Cylinder Re=100')
    await expect(page.getByTestId('issues')).toHaveText(/Ready/, { timeout: 15_000 })
    await page.getByTestId('tab-run').click()
    await expect(page.getByTestId('run-end-time')).toHaveValue('20')
    await page.getByTestId('run').click()
    await expect(page).toHaveURL(/#\/jobs\/[\w-]+$/)

    // Live: frames reach the viewer while the job runs, and the timeline follows the newest.
    await expect(page.getByTestId('viewer-frame')).toHaveText(/frame \d+/, { timeout: 60_000 })
    await page.getByTestId('viewer-quantity').selectOption('vorticity')
    await expect(page.getByTestId('job-status')).toHaveText('completed', { timeout: 300_000 })
    const position = page.getByTestId('timeline-position')
    await expect(position).toHaveText(/^(\d+) \/ \1 /, { timeout: 60_000 }) // at the last frame
    const last = Number((await position.textContent())!.split(' ')[0])
    expect(last).toBe(1000) // 20 s at a frame every 0.02 s, and frame 0
    const engine = (await page
      .getByTestId('job-view')
      .getByText(/^(cuda|wgpu) · /)
      .first()
      .textContent())!
    const looks = await seen(page)
    const live = looks.filter((o) => o.status !== 'completed' && /^\d+ \/ \d+ /.test(o.position))
    const liveFrames = new Set(live.map((o) => Number(o.position.split(' ')[0])))
    test.info().annotations.push({
      type: 'live',
      description: `${liveFrames.size} distinct playhead positions seen before the job completed`,
    })
    expect(liveFrames.size, 'the playhead moves while the run is going on').toBeGreaterThan(2)

    // The street at the end of the run: the cylinder is solid, and the vorticity along the
    // wake's centre line (the street alternates in sign) changes sign many times.
    expect(await pixel(page, ...CYLINDER)).toEqual(SOLID)
    const changesEnd = await signChanges(page, 42, 70, WIDTH - 10)
    console.log(
      `Live: ${liveFrames.size} playhead positions before completion; ${changesEnd} sign changes in the wake at the end`,
    )
    expect(changesEnd, 'sign changes of the vorticity in the wake').toBeGreaterThanOrEqual(8)

    // Back to the start: the fluid is at rest there (no street), and play it again from there.
    await page.getByTestId('timeline-slider').fill('0')
    await expect(position).toHaveText(/^0 \/ 1000 /)
    await expect(page.getByTestId('viewer-frame')).toHaveText(/frame 0 /)
    expect(await signChanges(page, 42, 70, WIDTH - 10)).toBeLessThan(2)
    await page.getByLabel('Playback speed').selectOption('60')
    await page.getByTestId('timeline-play').click()
    // Part-way through the street has formed behind the cylinder, then it plays to the end.
    await expect
      .poll(async () => Number((await position.textContent())!.split(' ')[0]), { timeout: 60_000 })
      .toBeGreaterThan(500)
    expect(await signChanges(page, 42, 70, WIDTH - 10)).toBeGreaterThanOrEqual(4)
    await expect(position).toHaveText(/^1000 \/ 1000 /, { timeout: 60_000 })
    await expect(page.getByTestId('timeline-play')).toBeVisible()

    // The statistics over the default window (second half: 10–20 s).
    await page.getByTestId('job-tab-stats').click()
    await expect(page.getByTestId('stats-window')).toHaveText('Window 10–20 s (second half)')
    await expect(page.getByTestId('stats-benchmark')).toHaveValue('schaferTurek2D2')
    const st = await statistic(page, 'strouhal')
    const cdMax = await statistic(page, 'cdMax')
    const clMax = await statistic(page, 'clMax')
    const stError = st.value / ST_REFERENCE - 1
    const cdError = cdMax.value / CD_MAX_REFERENCE - 1
    expect(Math.abs(stError), `St ${st.value}`).toBeLessThan(TOLERANCE)
    expect(Math.abs(cdError), `C_D,max ${cdMax.value}`).toBeLessThan(TOLERANCE)
    expect(st.check).toMatch(/✓$/)
    expect(cdMax.check).toMatch(/✓$/)
    await expect(page.getByTestId('stats-tolerance')).toContainText('within 5 %')
    const measured = `St ${st.value} (${(100 * stError).toFixed(1)} %), C_D,max ${cdMax.value} (${(100 * cdError).toFixed(1)} %), C_L,max ${clMax.value} (information), ${engine}`
    test.info().annotations.push({ type: 'acceptance', description: measured })
    console.log(`Acceptance: ${measured}`)

    // Exports. A PNG of the whole domain at 2 px per cell, with its footer.
    await page.getByTestId('job-tab-export').click()
    await page.getByTestId('export-size').selectOption('2')
    const png = await saved(page, 'export-png')
    expect(png.name).toMatch(/-vorticity-frame1000\.png$/)
    expect([...png.bytes.subarray(0, 8)]).toEqual([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])
    expect([png.bytes.readUInt32BE(16), png.bytes.readUInt32BE(20)]).toEqual([
      WIDTH * 2,
      HEIGHT * 2 + 48,
    ])
    expect([...png.bytes.subarray(-8)]).toEqual([0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82]) // IEND
    expect(png.bytes.length, 'a PNG with a street in it, not a flat image').toBeGreaterThan(20_000)

    // The force CSV: the coefficients of the cylinder, 10–20 s agreeing with the statistics, and
    // the run's own forces.csv.
    const coefficients = await saved(page, 'export-cd')
    const rows = coefficients.bytes.toString().trim().split('\n')
    expect(rows[0]).toBe('step,time_s,cd,cl')
    expect(rows.length).toBeGreaterThan(1000)
    const table = rows.slice(1).map((r) => r.split(',').map(Number))
    expect(table[table.length - 1][1]).toBeCloseTo(20, 1)
    const csvCdMax = Math.max(...table.filter((r) => r[1] >= 10).map((r) => r[2]))
    expect(csvCdMax / cdMax.value - 1).toBeCloseTo(0, 2)
    const [forces] = await Promise.all([
      page.waitForEvent('download'),
      page.getByRole('link', { name: 'forces.csv' }).click(),
    ])
    const forcesCsv = readFileSync((await forces.path())!).toString()
    expect(forcesCsv.split('\n')[0]).toMatch(/^step,time_s,/)
    expect(forcesCsv.trim().split('\n').length).toBeGreaterThan(1000)

    // A video of the street: a stretch of the developed shedding, 1 s of simulated time.
    await page.getByTestId('video-from').fill('800')
    await page.getByTestId('video-from').press('Enter')
    await page.getByTestId('video-to').fill('850')
    await page.getByTestId('video-to').press('Enter')
    const video = await saved(page, 'video-record')
    expect(video.name).toMatch(/-frames800-850\.(mp4|webm)$/)
    await expect(page.getByTestId('export-message')).toHaveText(/Saved 51 frames as /)
    if (video.name.endsWith('.mp4')) {
      expect(video.bytes.subarray(4, 8).toString(), 'MP4 header').toBe('ftyp')
    } else {
      expect([...video.bytes.subarray(0, 4)], 'WebM (EBML) header').toEqual([
        0x1a, 0x45, 0xdf, 0xa3,
      ])
    }
    expect(video.bytes.length, 'a video of 51 frames').toBeGreaterThan(20_000)
    await expect(position).toHaveText(/^1000 \/ 1000 /) // the timeline is where it was

    const forceRows = forcesCsv.trim().split('\n').length - 1
    console.log(
      `Files: PNG ${png.bytes.length} B, ${rows.length - 1} coefficient rows, forces.csv ${forceRows} rows, ${video.name} ${video.bytes.length} B`,
    )
    test.info().annotations.push({
      type: 'duration',
      description: `${((Date.now() - started) / 1000).toFixed(0)} s from the gallery to the video`,
    })
  },
)

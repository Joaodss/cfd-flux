// Phase 5 viewer: a short cylinder run through the job view — live frames, the solid mask, the
// colour map, and the timeline over stored frames (scrub, step, play). Against a real cfd-server.
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { expect, test } from '@playwright/test'
import { pixel, saved as savedFile, SOLID } from './helpers.ts'

const scene = JSON.parse(
  readFileSync(
    fileURLToPath(new URL('../../../scenes/cylinder-re100.json', import.meta.url)),
    'utf8',
  ),
)

/** The parts of the development-only viewer hook the test uses. */
interface ViewerHook {
  snapshot(): string
  streamlineSegments: number
  measureSim(): { particles: number; dye: number }
}

const CYLINDER: [number, number] = [41, 41] // centre of the cylinder's disk
const WAKE: [number, number] = [70, 45] // just behind it

test('a run is shown live, then scrubbed and played back from stored frames', async ({
  page,
  request,
}) => {
  // 0.5 s of simulated time, a frame every 0.02 s: the wake starts to form.
  const r = await request.post('/api/simulations', {
    data: { scene, options: { endTime: 0.5, outputInterval: 0.02 } },
  })
  expect(r.status()).toBe(201)
  const { id } = (await r.json()) as { id: string }

  await page.goto(`/#/jobs/${id}`)
  await expect(page.getByTestId('job-status')).toHaveText('completed', { timeout: 60_000 })
  // Frames arrived (live, or the last stored one once the run is over).
  await expect(page.getByTestId('viewer-frame')).toHaveText(/frame \d+/)
  const position = page.getByTestId('timeline-position')
  await expect(position).toHaveText(/^(\d+) \/ \1 /) // at the last frame
  const last = Number((await position.textContent())!.split(' ')[0])
  expect(last).toBeGreaterThanOrEqual(20)

  // The cylinder is drawn as a solid; the wake is coloured by the field.
  expect(await pixel(page, ...CYLINDER)).toEqual(SOLID)
  const wakeLast = await pixel(page, ...WAKE)
  expect(wakeLast).not.toEqual(SOLID)

  // Scrub to the first frame (fetched from the server): a fluid at rest looks different.
  await page.getByTestId('timeline-slider').fill('0')
  await expect(position).toHaveText(new RegExp(`^0 / ${last} `))
  await expect(page.getByTestId('viewer-frame')).toHaveText(/frame 0 /)
  expect(await pixel(page, ...WAKE)).not.toEqual(wakeLast)
  expect(await pixel(page, ...CYLINDER)).toEqual(SOLID)

  // Step with the arrow keys.
  await page.keyboard.press('ArrowRight')
  await expect(position).toHaveText(new RegExp(`^1 / ${last} `))

  // Play to the end: playback stops at the last frame.
  await page.getByTestId('timeline-play').click()
  await expect(position).toHaveText(new RegExp(`^${last} / ${last} `), { timeout: 30_000 })
  await expect(page.getByTestId('timeline-play')).toBeVisible()
  expect(await pixel(page, ...WAKE)).toEqual(wakeLast)

  // Overlays: streamlines come from the decode worker; arrows are drawn by the GPU from the
  // velocity texture (the image changes with each).
  const image = () =>
    page.evaluate(() => (globalThis as unknown as { __viewer: ViewerHook }).__viewer.snapshot())
  const segments = () =>
    page.evaluate(
      () => (globalThis as unknown as { __viewer: ViewerHook }).__viewer.streamlineSegments,
    )
  const plain = await image()
  await page.getByTestId('viewer-vectors').check()
  await expect(page.getByTestId('viewer-arrow-reference')).toHaveText(/m.s$/)
  const arrows = await image()
  expect(arrows).not.toEqual(plain)
  await page.getByTestId('viewer-streamlines').check()
  await expect.poll(() => segments(), { timeout: 30_000 }).toBeGreaterThan(100)
  const both = await image()
  expect(both).not.toEqual(arrows)
  await page.getByTestId('viewer-vectors').uncheck()
  await page.getByTestId('viewer-streamlines').uncheck()
  await expect.poll(() => segments()).toBe(0)
  expect(await image()).toEqual(plain)

  // Particles and dye live in float textures (EXT_color_buffer_float, also in SwiftShader):
  // particles are released at the inlet and carried by the last frame's flow; dye is injected
  // there and advected; clearing empties both.
  const sim = () =>
    page.evaluate(() => (globalThis as unknown as { __viewer: ViewerHook }).__viewer.measureSim())
  await expect(page.getByTestId('viewer-particles')).toBeEnabled()
  await page.getByTestId('viewer-particles').check()
  await page.getByTestId('viewer-dye').check()
  await expect.poll(async () => (await sim()).particles, { timeout: 30_000 }).toBeGreaterThan(200)
  await expect.poll(async () => (await sim()).dye, { timeout: 30_000 }).toBeGreaterThan(5)
  const flowing = await image()
  expect(flowing).not.toEqual(plain)
  // The density sets how many particles are in use: per fluid cell, so at most this many of the
  // grid's cells. Fewer are unreleased at once; more are released over the next passes.
  const cells = scene.grid.width * scene.grid.height
  const particles = async () => (await sim()).particles
  expect(await particles()).toBeLessThanOrEqual(0.15 * cells)
  await page.getByTestId('viewer-particle-density').selectOption('few')
  await expect.poll(particles, { timeout: 10_000 }).toBeLessThanOrEqual(0.05 * cells)
  await page.getByTestId('viewer-particle-density').selectOption('many')
  await expect.poll(particles, { timeout: 30_000 }).toBeGreaterThan(0.15 * cells)
  await page.getByTestId('viewer-particle-density').selectOption('normal')
  // A larger size draws the same particles bigger.
  await page.getByTestId('viewer-particle-size').selectOption('4')
  await expect.poll(image).not.toEqual(flowing)
  await page.getByTestId('viewer-particle-size').selectOption('2')
  await page.getByTestId('viewer-particles').uncheck()
  await page.getByTestId('viewer-dye').uncheck()
  expect(await image()).toEqual(plain)

  // Another field: vorticity, symmetric range around zero.
  await page.getByTestId('viewer-quantity').selectOption('vorticity')
  await expect(page.getByTestId('viewer-range-mode')).toHaveValue('symmetric')
  const ticks = (await page.getByTestId('viewer-legend').innerText()).split('\n')
  expect(Number(ticks[0])).toBeCloseTo(-Number(ticks[4]))

  // Export: a PNG of the whole domain at 2 px per cell with its footer, the CSVs, and a video of
  // five frames (H.264/VP9 in MP4 with WebCodecs, else a WebM recording).
  await page.getByTestId('job-tab-export').click()
  const saved = (button: string) => savedFile(page, button)
  await page.getByTestId('export-size').selectOption('2')
  const png = await saved('export-png')
  expect(png.name).toMatch(/-vorticity-frame\d{4}\.png$/)
  expect([...png.bytes.subarray(0, 4)]).toEqual([0x89, 0x50, 0x4e, 0x47])
  // IHDR: the domain at 2 px per cell, plus the 48 px footer.
  expect([png.bytes.readUInt32BE(16), png.bytes.readUInt32BE(20)]).toEqual([
    scene.grid.width * 2,
    scene.grid.height * 2 + 48,
  ])
  const fields = await saved('export-fields')
  expect(fields.bytes.toString().split('\n')[0]).toBe(
    'x_m,y_m,velocity_x,velocity_y,pressure,vorticity',
  )
  const coefficients = await saved('export-cd')
  expect(coefficients.bytes.toString().split('\n')[0]).toBe('step,time_s,cd,cl')
  expect(coefficients.bytes.toString().split('\n').length).toBeGreaterThan(50)
  // The video's clock is the flow's: frames every 0.02 s play at 50 frames/s in real time; at
  // × 2 every 2nd frame is used.
  const timing = page.getByTestId('video-timing')
  await expect(timing).toHaveText(/^1 s of video = 1 s of flow · 50 frames\/s · \d+ frames/)
  await page.getByTestId('video-speed').selectOption('2')
  await expect(timing).toHaveText(/= 2 s of flow · 50 frames\/s · every 2nd frame/)
  await page.getByTestId('video-speed').selectOption('1')
  await page.getByTestId('video-to').fill('4')
  await page.getByTestId('video-to').press('Enter')
  await expect(timing).toHaveText(/· 5 frames, 0\.1 s$/)
  const video = await saved('video-record')
  expect(video.name).toMatch(/-frames0-4\.(mp4|webm)$/)
  expect(video.bytes.length).toBeGreaterThan(1000)
  if (video.name.endsWith('.mp4')) expect(video.bytes.subarray(4, 8).toString()).toBe('ftyp')
  await expect(page.getByTestId('export-message')).toHaveText(/Saved 5 frames as /)
  // The timeline is where it was.
  await expect(position).toHaveText(new RegExp(`^${last} / ${last} `))

  // The Plots tab: the force coefficients of the body in the flow, the cylinder (not the channel's
  // walls; uPlot draws into a canvas); a click on the plot moves the playhead to the frame at that
  // time.
  await page.getByTestId('job-tab-plots').click()
  await expect(page.getByTestId('plots-element')).toHaveValue('Cylinder')
  const forces = page.getByTestId('plot-forces:Cylinder')
  await expect(forces.locator('canvas')).toBeVisible()
  await expect(page.locator('[data-testid="plot-diagnostics:mach"] canvas')).toBeVisible()
  const area = forces.locator('.u-over')
  const box = (await area.boundingBox())!
  await area.click({ position: { x: box.width * 0.25, y: box.height / 2 } })
  const [at] = (await position.textContent())!.split(' ')
  expect(Number(at)).toBeGreaterThan(0)
  expect(Number(at)).toBeLessThan(last)

  // The Statistics tab: the force statistics of the second half, compared with Schäfer–Turek
  // 2D-2 (the run is too short to shed: no St yet).
  await page.getByTestId('job-tab-stats').click()
  await expect(page.getByTestId('stats-window')).toHaveText('Window 0.25–0.5 s (second half)')
  await expect(page.getByTestId('stats-benchmark')).toHaveValue('schaferTurek2D2')
  await expect(page.getByTestId('stat-cdMax')).toHaveText(/^C_D,max\d/)
})

test('a lost WebGL context is restored and the frame drawn again', async ({ page, request }) => {
  const r = await request.post('/api/simulations', {
    data: { scene, options: { endTime: 0.2, outputInterval: 0.02 } },
  })
  expect(r.status()).toBe(201)
  const { id } = (await r.json()) as { id: string }
  await page.goto(`/#/jobs/${id}`)
  await expect(page.getByTestId('job-status')).toHaveText('completed', { timeout: 60_000 })
  await expect(page.getByTestId('timeline-position')).toHaveText(/^(\d+) \/ \1 /)

  // The field with arrows and streamlines: every kind of GPU resource the viewer holds.
  const image = () =>
    page.evaluate(() => (globalThis as unknown as { __viewer: ViewerHook }).__viewer.snapshot())
  const segments = () =>
    page.evaluate(
      () => (globalThis as unknown as { __viewer: ViewerHook }).__viewer.streamlineSegments,
    )
  await page.getByTestId('viewer-vectors').check()
  await page.getByTestId('viewer-streamlines').check()
  await expect.poll(() => segments(), { timeout: 30_000 }).toBeGreaterThan(100)
  const before = await image()
  expect(await pixel(page, ...CYLINDER)).toEqual(SOLID)

  // `WEBGL_lose_context` does what a driver reset does (the same context as the viewer's: a
  // canvas has one).
  const loseContext = (call: 'loseContext' | 'restoreContext') =>
    page.evaluate((call) => {
      const canvas = document.querySelector<HTMLCanvasElement>('[data-testid="viewer-canvas"]')!
      const w = globalThis as unknown as { __lose?: WEBGL_lose_context }
      w.__lose ??= canvas.getContext('webgl2')!.getExtension('WEBGL_lose_context')!
      w.__lose[call]()
    }, call)
  await loseContext('loseContext')

  // While it is lost, exports say so instead of failing on what the lost context returns.
  await page.getByTestId('job-tab-export').click()
  await page.getByTestId('export-size').selectOption('2')
  await page.getByTestId('export-png').click()
  await expect(page.getByTestId('export-message')).toHaveText(/graphics context was lost/)
  await page.getByTestId('video-record').click()
  await expect(page.getByTestId('export-message')).toHaveText(
    /^Video failed: The graphics context was lost/,
  )

  // Restored: the programs, textures and buffers are made again and the same picture comes back.
  await loseContext('restoreContext')
  await expect.poll(() => image(), { timeout: 30_000 }).toEqual(before)
  expect(await pixel(page, ...CYLINDER)).toEqual(SOLID)
  const png = await savedFile(page, 'export-png')
  expect([png.bytes.readUInt32BE(16), png.bytes.readUInt32BE(20)]).toEqual([
    scene.grid.width * 2,
    scene.grid.height * 2 + 48,
  ])
})

test('going from one run straight to another shows the second run', async ({ page, request }) => {
  // Two runs of different lengths, so their last frames differ.
  const start = async (endTime: number) => {
    const r = await request.post('/api/simulations', {
      data: { scene, options: { endTime, outputInterval: 0.02 } },
    })
    expect(r.status()).toBe(201)
    return ((await r.json()) as { id: string }).id
  }
  const a = await start(0.2)
  const b = await start(0.4)

  await page.goto(`/#/jobs/${a}`)
  await expect(page.getByTestId('job-status')).toHaveText('completed', { timeout: 60_000 })
  const position = page.getByTestId('timeline-position')
  await expect(position).toHaveText(/^(\d+) \/ \1 /)

  const done = async () => {
    const job = (await (await request.get(`/api/simulations/${b}`)).json()) as {
      status: string
      frames: number
    }
    return job.status === 'completed' ? job.frames : 0
  }
  await expect.poll(done, { timeout: 60_000 }).toBeGreaterThan(0)
  const last = (await done()) - 1

  // Only the hash changes (as back/forward or a link would): the view must be B's, not A's.
  await page.evaluate((id) => (window.location.hash = `#/jobs/${id}`), b)
  await expect(page.getByTestId('job-status')).toHaveText('completed')
  await expect(position).toHaveText(new RegExp(`^${last} / ${last} `))
  await expect(page.getByTestId('viewer-frame')).toHaveText(new RegExp(`frame ${last} `))
  await expect(page.getByText(/was not stored/)).toHaveCount(0)
})

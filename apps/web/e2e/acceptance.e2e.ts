// Phase 4 acceptance (docs/05-implementation-plan.md): draw the cylinder-in-a-channel case from
// scratch, check it validates, and submit it — through the UI, against a real cfd-server.
// The drawing must come out cell for cell as `cfd-core`'s example (Schäfer & Turek 2D-2).
import { expect, test, type Page } from '@playwright/test'

/** Page coordinates of the centre of cell (x, y) at the current view. */
async function cellPoint(page: Page, x: number, y: number): Promise<{ x: number; y: number }> {
  return page.evaluate(
    ([x, y]) => {
      const s = (globalThis as unknown as { __editor: { getState(): any } }).__editor.getState()
      const r = document.querySelector('[data-testid=editor-canvas]')!.getBoundingClientRect()
      const v = s.view
      return {
        x: r.left + v.offsetX + (x + 0.5) * v.zoom,
        y: r.top + v.offsetY + (s.layers.height - y - 0.5) * v.zoom,
      }
    },
    [x, y],
  )
}

/** A mouse drag from one cell to another (Shift held when `shift`). */
async function drag(page: Page, from: [number, number], to: [number, number], shift = false) {
  const a = await cellPoint(page, ...from)
  const b = await cellPoint(page, ...to)
  if (shift) await page.keyboard.down('Shift')
  await page.mouse.move(a.x, a.y)
  await page.mouse.down()
  await page.mouse.move(b.x, b.y, { steps: 5 })
  await page.mouse.up()
  if (shift) await page.keyboard.up('Shift')
}

/** Types a value into a number field and commits it (Enter). */
async function setField(page: Page, testId: string, value: string) {
  const field = page.getByTestId(testId)
  await field.fill(value)
  await field.press('Enter')
}

test('draw the cylinder in a channel from scratch and run it', async ({ page }) => {
  const started = Date.now()
  page.on('dialog', (d) => d.accept()) // "run with these warnings?" if there are any
  await page.goto('/')
  await expect(page.getByTestId('issues')).not.toHaveText(/Checking/)

  // Scene: a 442 × 84 grid of 5 mm cells (2.21 m × 0.42 m), ν = 1e-3 m²/s, a short run.
  await page.getByTestId('tab-scene').click()
  await setField(page, 'grid-width', '442')
  await setField(page, 'grid-height', '84')
  await setField(page, 'grid-cell-size', '0.005')
  await setField(page, 'fluid-1-viscosity', '1e-3')
  await page.getByTestId('tab-run').click()
  await setField(page, 'run-end-time', '0.3')
  await setField(page, 'run-output-interval', '0.1')
  await page.keyboard.press('f') // fit the new grid in the view

  // Channel walls: two filled rectangles, one cell high (one element: "Wall 1").
  await page.getByTestId('palette-wall').click()
  await page.getByTestId('tool-rect').click()
  await drag(page, [0, 0], [441, 0])
  await drag(page, [0, 83], [441, 83])

  // Inlet: the left column between the walls, a parabolic profile peaking at 1.5 m/s.
  await page.getByTestId('palette-inlet').click()
  await drag(page, [0, 1], [0, 82])
  await page.getByTestId('inlet-profile').selectOption('parabolic')
  await setField(page, 'inlet-velocity-x', '1.5')

  // Outlet: the right column (fixed pressure 0 Pa by default).
  await page.getByTestId('palette-outlet').click()
  await drag(page, [441, 1], [441, 82])

  // Cylinder: a new wall element, a 20 × 20 circle (Shift) centred 0.2 m from the inlet corner.
  await page.getByTestId('new-wall').click()
  await page.getByTestId('tool-ellipse').click()
  await drag(page, [31, 31], [50, 50], true)

  // The drawing is the example's, cell for cell (cell types and element ids).
  const same = await page.evaluate(async () => {
    // The app's own modules, as the Vite dev server serves them (paths only the page resolves).
    const examples = '/src/scene/examples.ts'
    const io = '/src/scene/io.ts'
    const { EXAMPLES } = await import(/* @vite-ignore */ examples)
    const { fromScene } = await import(/* @vite-ignore */ io)
    const example = await fromScene(
      EXAMPLES.find((e: { id: string }) => e.id === 'cylinder-re100').scene,
    )
    const drawn = (globalThis as unknown as { __editor: { getState(): any } }).__editor.getState()
    const eq = (a: ArrayLike<number>, b: ArrayLike<number>) =>
      a.length === b.length && Array.prototype.every.call(a, (v: number, i: number) => v === b[i])
    return {
      cellType: eq(drawn.layers.cellType, example.layers.cellType),
      elementId: eq(drawn.layers.elementId, example.layers.elementId),
      elements: drawn.table.elements.map((e: { id: number; kind: string }) => `${e.id}:${e.kind}`),
    }
  })
  expect(same).toEqual({
    cellType: true,
    elementId: true,
    elements: ['1:wall', '2:inlet', '3:outlet', '4:wall'],
  })

  // Validation by the server: no errors, and the estimate in the Run tab.
  await expect(page.getByTestId('issues')).toHaveText(/Ready/, { timeout: 15_000 })
  await page.getByTestId('tab-run').click()
  await expect(page.getByTestId('estimate-re')).toHaveText('615') // 1.5 m/s × 0.41 m / 1e-3

  // Submit, follow the run to its end, and go back to the drawing.
  await page.getByTestId('run').click()
  await expect(page).toHaveURL(/#\/jobs\/[\w-]+$/)
  await expect(page.getByTestId('job-status')).toHaveText('completed', { timeout: 60_000 })
  await page.getByTestId('back-to-editor').click()
  await expect(page.getByTestId('scene-name')).toBeVisible()

  test.info().annotations.push({
    type: 'duration',
    description: `drawn, validated and run in ${((Date.now() - started) / 1000).toFixed(1)} s`,
  })
})

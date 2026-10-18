import { describe, expect, it } from 'vitest'
import { cellToScreen, fitView, screenToCell, stepZoom, zoomAt } from '../src/editor/view.ts'

describe('view transform', () => {
  const v = { zoom: 4, offsetX: 10, offsetY: 20 }
  const H = 50

  it('maps the top-left screen corner of the domain to the top row of cells', () => {
    expect(screenToCell(v, H, 10, 20)).toEqual([0, H - 1])
    expect(screenToCell(v, H, 13.9, 23.9)).toEqual([0, H - 1])
    expect(screenToCell(v, H, 14, 24)).toEqual([1, H - 2])
    // The bottom-left cell's screen square ends at the domain's bottom edge.
    expect(screenToCell(v, H, 10, 20 + H * 4 - 0.1)).toEqual([0, 0])
    expect(screenToCell(v, H, 9, 20)).toEqual([-1, H - 1])
  })

  it('round-trips cell corners', () => {
    const [sx, sy] = cellToScreen(v, H, 7, 3)
    expect([sx, sy]).toEqual([10 + 28, 20 + (H - 3) * 4])
    // The point just inside cell (7, 3) from its bottom-left corner.
    expect(screenToCell(v, H, sx + 0.5, sy - 0.5)).toEqual([7, 3])
  })

  it('zooms about the pointer', () => {
    const z = zoomAt(v, 8, 100, 100)
    expect(z.zoom).toBe(8)
    expect(screenToCell(z, H, 100, 100)).toEqual(screenToCell(v, H, 100, 100))
    expect(zoomAt(v, 1000, 0, 0).zoom).toBe(32)
  })

  it('steps on a √2 ladder that hits powers of two exactly', () => {
    let z = 1
    z = stepZoom(z, 1)
    expect(z).toBeCloseTo(Math.SQRT2, 12)
    z = stepZoom(z, 1)
    expect(z).toBe(2)
    expect(stepZoom(2, -1)).toBeCloseTo(Math.SQRT2, 12)
    expect(stepZoom(1.5, 1)).toBe(2)
    expect(stepZoom(1.5, -1)).toBeCloseTo(Math.SQRT2, 12)
    expect(stepZoom(32, 1)).toBe(32)
    expect(stepZoom(1 / 8, -1)).toBe(1 / 8)
  })

  it('fits and centres the grid', () => {
    const f = fitView(442, 84, 1200, 800)
    expect(f.zoom).toBe(2) // 2.83 would need 1251 px
    expect(f.offsetX).toBe((1200 - 884) / 2)
    expect(fitView(4096, 4096, 1200, 800).zoom).toBeCloseTo(Math.SQRT2 / 8, 12)
  })
})

import { describe, expect, it } from 'vitest'
import { CellType, Layers, TILE } from '../src/editor/layers.ts'

describe('Layers', () => {
  it('starts all fluid, with every tile dirty', () => {
    const l = new Layers(130, 70)
    expect(l.tilesX).toBe(3)
    expect(l.tilesY).toBe(2)
    expect(l.cellType.every((v) => v === CellType.Fluid)).toBe(true)
    // More than half the tiles dirty: one rectangle for the whole grid.
    expect(l.takeDirty()).toEqual([{ x0: 0, y0: 0, x1: 130, y1: 70 }])
    expect(l.takeDirty()).toEqual([])
  })

  it('indexes rows from the bottom-left corner', () => {
    const l = new Layers(10, 5)
    l.set(3, 2, CellType.Solid, 7)
    expect(l.cellType[2 * 10 + 3]).toBe(CellType.Solid)
    expect(l.elementId[l.index(3, 2)]).toBe(7)
  })

  it('reports the tiles that changed, clipped to the grid', () => {
    const l = new Layers(3 * TILE + 10, 2 * TILE)
    l.takeDirty()
    const v = l.version
    l.set(5, 5, CellType.Solid, 1)
    l.set(3 * TILE + 2, TILE + 1, CellType.Inlet, 2)
    l.set(-1, 0, CellType.Solid, 1) // outside: ignored
    expect(l.version).toBeGreaterThan(v)
    expect(l.takeDirty()).toEqual([
      { x0: 0, y0: 0, x1: TILE, y1: TILE },
      { x0: 3 * TILE, y0: TILE, x1: 3 * TILE + 10, y1: 2 * TILE },
    ])
  })

  it('rejects layers of the wrong length', () => {
    expect(() => new Layers(4, 4, new Uint8Array(15))).toThrow(/wrong length/)
    expect(() => new Layers(0, 4)).toThrow(/invalid grid/)
  })
})

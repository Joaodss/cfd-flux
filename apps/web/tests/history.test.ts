import { describe, expect, it } from 'vitest'
import { History, Stroke } from '../src/editor/history.ts'
import { CellType, Layers, TILE } from '../src/editor/layers.ts'

function snapshot(l: Layers): [Uint8Array, Uint16Array] {
  return [l.cellType.slice(), l.elementId.slice()]
}

describe('Stroke', () => {
  it('keeps only the tiles whose content changed', () => {
    const l = new Layers(3 * TILE, 2 * TILE)
    const s = new Stroke(l)
    s.write(5, 5, CellType.Solid, 1)
    s.write(TILE + 1, 1, CellType.Fluid, 0) // touched, unchanged
    const tiles = s.finish()
    expect([...tiles.keys()]).toEqual([0])
    expect(tiles.get(0)).toEqual({ cellType: CellType.Fluid, elementId: 0 }) // uniform before
  })

  it('reverts to the content before the stroke, any number of times', () => {
    const l = new Layers(100, 100)
    l.set(10, 10, CellType.Inlet, 3)
    const before = snapshot(l)
    const s = new Stroke(l)
    for (let i = 0; i < 3; i++) {
      s.span(10, 0, 99, CellType.Solid, 1)
      s.revert()
      expect(snapshot(l)).toEqual(before)
    }
    s.span(10, -5, 200, CellType.Solid, 1) // clipped to the grid
    expect(l.cellType[l.index(99, 10)]).toBe(CellType.Solid)
    expect(s.finish().size).toBe(2) // tiles 0 and 1 on row 10
  })

  it('ignores writes outside the grid', () => {
    const l = new Layers(10, 10)
    const s = new Stroke(l)
    s.write(-1, 0, CellType.Solid, 1)
    s.span(10, 0, 5, CellType.Solid, 1)
    s.span(0, 20, 30, CellType.Solid, 1)
    expect(s.finish().size).toBe(0)
  })
})

describe('History', () => {
  function paint(l: Layers, h: History, x: number, y: number, ct: CellType, id: number) {
    const s = new Stroke(l)
    s.write(x, y, ct, id)
    h.push({ label: `paint ${x},${y}`, tiles: s.finish() })
  }

  it('undoes and redoes by swapping tiles', () => {
    const l = new Layers(200, 130)
    const h = new History()
    const s0 = snapshot(l)
    paint(l, h, 1, 1, CellType.Solid, 1)
    const s1 = snapshot(l)
    paint(l, h, 150, 100, CellType.Outlet, 2)
    const s2 = snapshot(l)
    expect(h.undoLabel).toBe('paint 150,100')
    h.undo(l)
    expect(snapshot(l)).toEqual(s1)
    h.undo(l)
    expect(snapshot(l)).toEqual(s0)
    expect(h.canUndo).toBe(false)
    expect(h.undo(l)).toBeNull()
    h.redo(l)
    expect(snapshot(l)).toEqual(s1)
    h.redo(l)
    expect(snapshot(l)).toEqual(s2)
    expect(h.canRedo).toBe(false)
  })

  it('drops the redo stack on a new operation', () => {
    const l = new Layers(10, 10)
    const h = new History()
    paint(l, h, 1, 1, CellType.Solid, 1)
    paint(l, h, 2, 2, CellType.Solid, 1)
    h.undo(l)
    paint(l, h, 3, 3, CellType.Solid, 1)
    expect(h.canRedo).toBe(false)
    expect(h.size.entries).toBe(2)
    h.undo(l)
    h.undo(l)
    expect(l.cellType.every((v) => v === CellType.Fluid)).toBe(true)
  })

  it('stores uniform tiles as one value', () => {
    const l = new Layers(4 * TILE, 4 * TILE)
    const s = new Stroke(l)
    for (let y = 0; y < l.height; y++) s.span(y, 0, l.width - 1, CellType.Solid, 1)
    const tiles = s.finish()
    expect(tiles.size).toBe(16)
    const h = new History()
    h.push({ label: 'fill', tiles })
    expect(h.size.bytes).toBeLessThan(16 * 100 + 300)
    h.undo(l)
    // After the swap the entry holds the solid tiles, still compressed.
    expect(h.size.bytes).toBeLessThan(16 * 100 + 300)
    expect(l.cellType.every((v) => v === CellType.Fluid)).toBe(true)
  })

  it('keeps within its byte budget and entry limit, dropping the oldest', () => {
    const l = new Layers(TILE, TILE)
    // One entry holding one full tile: 64 × 64 × (1 + 2) bytes plus small overheads.
    const tileCost = TILE * TILE * 3 + 64 + 256
    const h = new History(3 * tileCost, 100)
    for (let i = 0; i < 10; i++) {
      const s = new Stroke(l)
      s.write(i, i, CellType.Solid, i + 1) // non-uniform tile: a full copy each time
      h.push({ label: `${i}`, tiles: s.finish() })
    }
    expect(h.size.bytes).toBeLessThanOrEqual(3 * tileCost)
    expect(h.size.entries).toBeLessThan(10)
    const limited = new History(1e9, 4)
    for (let i = 0; i < 10; i++) paint(l, limited, i, 0, CellType.Solid, 1)
    expect(limited.size.entries).toBe(4)
    expect(limited.undoLabel).toBe('paint 9,0')
  })
})

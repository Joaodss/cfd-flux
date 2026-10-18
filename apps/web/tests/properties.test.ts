import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { beforeEach, describe, expect, it } from 'vitest'
import { CellType, Layers } from '../src/editor/layers.ts'
import { resizeLayers, resizeTable, scaleLayers, scaleTable } from '../src/editor/resize.ts'
import { useEditor } from '../src/editor/store.ts'
import { formatNumber, parseNumber } from '../src/lib/numbers.ts'
import { newDocument } from '../src/scene/defaults.ts'
import { FLUID_PRESETS, nextId, presetFluid, SOLID_PRESETS } from '../src/scene/library.ts'

const state = () => useEditor.getState()
const FLUID = { cellType: CellType.Fluid, elementId: 0 }

describe('number fields', () => {
  it('parse what people type, with the field rules', () => {
    expect(parseNumber('0.005')).toBe(0.005)
    expect(parseNumber(' 1e-3 ')).toBe(0.001)
    expect(parseNumber('0,5')).toBe(0.5)
    expect(parseNumber('')).toBe('enter a number')
    expect(parseNumber('abc')).toBe('not a number')
    expect(parseNumber('2.5', { integer: true })).toBe('must be a whole number')
    expect(parseNumber('0', { positive: true })).toBe('must be positive')
    expect(parseNumber('5000', { max: 4096 })).toBe('must be at most 4096')
  })

  it('format numbers so they read back exactly', () => {
    for (const v of [0.005, 1.516e-5, 293.15, 1e-7, 4096]) {
      expect(parseNumber(formatNumber(v))).toBe(v)
    }
  })
})

describe('library', () => {
  it('is the generated JSON of cfd_core::library', () => {
    const json = JSON.parse(
      readFileSync(
        fileURLToPath(new URL('../src/generated/library.json', import.meta.url)),
        'utf8',
      ),
    )
    expect(json.fluids).toEqual(FLUID_PRESETS)
    expect(SOLID_PRESETS.map((s) => s.key)).toContain('aluminium')
  })

  it('gives the default scene the same air as the Rust examples', () => {
    const channel = JSON.parse(
      readFileSync(fileURLToPath(new URL('../../../scenes/channel.json', import.meta.url)), 'utf8'),
    )
    expect(newDocument().table.fluids[0]).toEqual(channel.fluids[0])
    expect(presetFluid(FLUID_PRESETS[1], 4)).toMatchObject({ id: 4, preset: 'water' })
    expect(nextId([{ id: 1 }, { id: 5 }])).toBe(6)
    expect(nextId([{ id: 255 }])).toBeNull()
  })
})

describe('resize and scale', () => {
  function sample(): Layers {
    const l = new Layers(4, 3)
    l.set(0, 0, CellType.Solid, 1)
    l.set(3, 2, CellType.Inlet, 2)
    return l
  }

  it('crops and extends anchored at the bottom-left cell', () => {
    const big = resizeLayers(sample(), 6, 4, FLUID)
    expect([big.width, big.height]).toEqual([6, 4])
    expect(big.elementId[big.index(0, 0)]).toBe(1)
    expect(big.elementId[big.index(3, 2)]).toBe(2)
    expect(big.cellType[big.index(5, 3)]).toBe(CellType.Fluid)
    const small = resizeLayers(sample(), 2, 2, FLUID)
    expect(Array.from(small.elementId)).toEqual([1, 0, 0, 0])
  })

  it('scales every cell into a k × k block', () => {
    const s = scaleLayers(sample(), 2)
    expect([s.width, s.height]).toEqual([8, 6])
    expect([s.index(0, 0), s.index(1, 1)].map((i) => s.elementId[i])).toEqual([1, 1])
    expect(s.elementId[s.index(2, 0)]).toBe(0)
    expect(s.elementId[s.index(7, 5)]).toBe(2)
  })

  it('updates the table: grid, cell size and probes', () => {
    const { table } = newDocument(10, 10, 0.01)
    const t = { ...table, probes: [{ name: 'A', position: [8, 2] as [number, number] }] }
    expect(resizeTable(t, 5, 5).probes).toEqual([])
    const s = scaleTable(t, 3)
    expect(s.grid).toMatchObject({ width: 30, height: 30, cellSize: 0.01 / 3 })
    expect(s.probes![0].position).toEqual([25, 7])
  })
})

describe('property edits and grid changes in the history', () => {
  beforeEach(() => state().load(newDocument(40, 20)))

  it('a property edit is one undo entry', () => {
    state().updateTable('End time', (t) => void (t.run.endTime = 3))
    expect(state().table.run.endTime).toBe(3)
    expect(state().history.undoLabel).toBe('End time')
    state().undo()
    expect(state().table.run.endTime).toBe(10)
    state().redo()
    expect(state().table.run.endTime).toBe(3)
  })

  it('an edit that changes nothing adds no entry', () => {
    state().updateTable('Nothing', () => {})
    expect(state().history.canUndo).toBe(false)
  })

  it('resizing replaces the layers and undo puts them back, with the drawing', () => {
    const before = state().layers
    before.set(1, 1, CellType.Solid, 0)
    state().resizeGrid(60, 30)
    expect([state().layers.width, state().table.grid.width]).toEqual([60, 60])
    expect(state().layers.cellType[state().layers.index(1, 1)]).toBe(CellType.Solid)
    state().scaleDrawing(2)
    expect(state().table.grid).toMatchObject({ width: 120, height: 60, cellSize: 0.0025 })
    state().undo()
    state().undo()
    expect(state().layers).toBe(before)
    expect(state().table.grid).toMatchObject({ width: 40, height: 20, cellSize: 0.005 })
    state().redo()
    expect(state().layers.width).toBe(60)
  })
})

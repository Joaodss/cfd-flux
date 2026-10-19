import { beforeEach, describe, expect, it } from 'vitest'
import {
  cellCounts,
  defaultElement,
  nextElementId,
  nextName,
  validElement,
} from '../src/editor/elements.ts'
import { CellType } from '../src/editor/layers.ts'
import { useEditor } from '../src/editor/store.ts'
import {
  choosePalette,
  deleteElement,
  handleKey,
  newElement,
  pick,
  pointerDown,
  pointerMove,
  pointerUp,
  setTool,
} from '../src/editor/tools/controller.ts'
import { nextProbeName, probeAt } from '../src/editor/tools/probe.ts'
import type { Element } from '../src/generated/scene.ts'
import { newDocument } from '../src/scene/defaults.ts'

const NONE = { shift: false, alt: false }
const state = () => useEditor.getState()

function drag(from: [number, number], to: [number, number] = from, m = NONE) {
  pointerDown(from, m)
  pointerMove(to, m)
  pointerUp(to, m)
}

/** A key press (Node has no KeyboardEvent; the handler only reads these fields). */
function key(k: string) {
  handleKey({
    key: k,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
  } as KeyboardEvent)
}

const cell = (x: number, y: number) => {
  const l = state().layers
  return [l.cellType[l.index(x, y)], l.elementId[l.index(x, y)]]
}

describe('element helpers', () => {
  const wall = (id: number, name?: string): Element => ({
    id,
    name,
    kind: 'wall',
    velocity: { type: 'noSlip' },
    thermal: { type: 'adiabatic' },
  })

  it('pick the next id and name', () => {
    expect(nextElementId([])).toBe(1)
    expect(nextElementId([wall(3), wall(7)])).toBe(8)
    expect(nextElementId([wall(65535)])).toBeNull()
    expect(nextName([], 'wall')).toBe('Wall 1')
    expect(nextName([wall(1, 'Wall 1'), wall(2, 'Wall 4'), wall(3, 'Cylinder')], 'wall')).toBe(
      'Wall 5',
    )
  })

  it('give new elements the documented defaults', () => {
    const { table } = newDocument()
    expect(defaultElement(table, 'inlet', 2)).toEqual({
      id: 2,
      name: 'Inlet 1',
      kind: 'inlet',
      velocity: { type: 'uniform', value: [0.1, 0] },
      thermal: { type: 'adiabatic' },
      fluid: 1,
    })
    expect(defaultElement(table, 'outlet', 3)).toMatchObject({
      pressure: { type: 'pressure', value: 0 },
    })
    expect(defaultElement(table, 'heatSource', 4)).toMatchObject({
      thermal: { type: 'fixed', value: 350 },
    })
  })

  it('check that an active element still exists with its kind', () => {
    expect(validElement([wall(2)], 'wall', 2)).toBe(2)
    expect(validElement([wall(2)], 'inlet', 2)).toBeNull()
    expect(validElement([wall(2)], 'wall', 3)).toBeNull()
  })

  it('count cells per element', () => {
    const { layers } = newDocument(10, 10)
    layers.set(1, 1, CellType.Solid, 4)
    layers.set(2, 1, CellType.Solid, 4)
    const c = cellCounts(layers)
    expect(c[4]).toBe(2)
    expect(c[0]).toBe(98)
  })
})

describe('palette and automatic elements', () => {
  beforeEach(() => {
    state().load(newDocument(40, 20))
    setTool('pencil')
    choosePalette('wall')
  })

  it('creates an element on the first stroke of a kind, in the same undo entry', () => {
    drag([1, 1], [5, 1])
    const { table, history, activeElement, selectedElement } = state()
    expect(table.elements).toHaveLength(1)
    expect(table.elements[0]).toMatchObject({ id: 1, name: 'Wall 1', kind: 'wall' })
    expect(activeElement.wall).toBe(1)
    expect(selectedElement).toBe(1)
    expect(cell(3, 1)).toEqual([CellType.Solid, 1])
    expect(history.size.entries).toBe(1)
    // A second stroke reuses the element.
    drag([1, 3])
    expect(state().table.elements).toHaveLength(1)
    // Undo twice: the cells and the element are gone together.
    state().undo()
    state().undo()
    expect(state().table.elements).toHaveLength(0)
    expect(cell(3, 1)).toEqual([CellType.Fluid, 0])
    // Redo brings both back.
    state().redo()
    expect(state().table.elements).toHaveLength(1)
    expect(cell(3, 1)).toEqual([CellType.Solid, 1])
  })

  it('removes the element again when the stroke changed nothing', () => {
    choosePalette('inlet')
    setTool('bucket')
    drag([1, 1]) // fills the whole fluid domain: changes cells
    expect(state().table.elements).toHaveLength(1)
    newElement()
    drag([1, 1]) // same paint? no: a new inlet over inlet cells changes the ids
    expect(state().table.elements).toHaveLength(2)
    choosePalette('outlet')
    setTool('eraser')
    drag([1, 1]) // the eraser never creates elements
    expect(state().table.elements).toHaveLength(2)
    setTool('rect')
    choosePalette('outlet')
    pointerDown([50, 50], NONE) // outside the grid: no cells change
    pointerUp([60, 60], NONE)
    expect(state().table.elements.map((e) => e.kind)).toEqual(['inlet', 'inlet'])
  })

  it('makes a new element of the same kind after "New" (N)', () => {
    drag([1, 1])
    key('n')
    drag([1, 5])
    const walls = state().table.elements
    expect(walls.map((e) => e.name)).toEqual(['Wall 1', 'Wall 2'])
    expect(cell(1, 5)).toEqual([CellType.Solid, 2])
  })

  it('creates a new element when undo removed the active one', () => {
    drag([1, 1])
    state().undo()
    drag([2, 2])
    expect(state().table.elements.map((e) => e.id)).toEqual([1])
    expect(cell(2, 2)).toEqual([CellType.Solid, 1])
  })

  it('picks a cell with the eyedropper (I, or Alt+click)', () => {
    drag([1, 1])
    choosePalette('heatSource')
    drag([3, 3])
    pick([1, 1])
    expect(state().palette).toBe('wall')
    expect(state().activeElement.wall).toBe(1)
    pointerDown([10, 10], { shift: false, alt: true }) // Alt+click with the pencil
    pointerUp([10, 10], { shift: false, alt: true })
    expect(state().palette).toBe('fluid')
    expect(state().history.size.entries).toBe(2)
  })

  it('deletes an element: its cells become fluid, one undo entry', () => {
    drag([1, 1], [10, 1])
    choosePalette('inlet')
    drag([0, 5], [0, 9])
    deleteElement(1)
    expect(state().table.elements.map((e) => e.kind)).toEqual(['inlet'])
    expect(cell(5, 1)).toEqual([CellType.Fluid, 0])
    expect(state().history.undoLabel).toBe('Delete Wall 1')
    state().undo()
    expect(cell(5, 1)).toEqual([CellType.Solid, 1])
    expect(state().table.elements).toHaveLength(2)
  })
})

describe('probes', () => {
  beforeEach(() => {
    state().load(newDocument(40, 20))
    setTool('probe')
  })

  it('helpers find the nearest probe and name new ones', () => {
    const probes = [
      { name: 'Probe 1', position: [5, 5] as [number, number] },
      { name: 'Probe 7', position: [8, 5] as [number, number] },
    ]
    expect(probeAt(probes, [7, 5], 1)).toBe(1)
    expect(probeAt(probes, [6, 6], 1)).toBe(0)
    expect(probeAt(probes, [20, 5], 2)).toBeNull()
    expect(nextProbeName(probes)).toBe('Probe 8')
  })

  it('are placed, moved and deleted, each one undo entry', () => {
    expect(state().palette).toBe('probe')
    drag([10, 10])
    expect(state().table.probes).toEqual([{ name: 'Probe 1', position: [10, 10] }])
    expect(state().history.undoLabel).toBe('Add probe')
    drag([10, 10], [30, 4])
    expect(state().table.probes![0].position).toEqual([30, 4])
    expect(state().history.undoLabel).toBe('Move probe')
    drag([35, 15])
    expect(state().table.probes).toHaveLength(2)
    expect(state().selectedProbe).toBe(1)
    key('Delete')
    expect(state().table.probes!.map((p) => p.name)).toEqual(['Probe 1'])
    state().undo()
    state().undo()
    state().undo()
    expect(state().table.probes).toEqual([{ name: 'Probe 1', position: [10, 10] }])
  })

  it('stay inside the grid when dragged out of it', () => {
    drag([45, 5]) // a click outside the grid adds nothing
    expect(state().table.probes ?? []).toHaveLength(0)
    drag([5, 5], [100, -3]) // placed, then dragged out: clamped to the edge
    expect(state().table.probes![0].position).toEqual([39, 0])
  })
})

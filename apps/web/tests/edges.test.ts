// The domain edges (Q14 option B): runs along each edge, and "Make inlet" / "Make outlet" as one
// undo entry painting the same cells as a drag along the edge between walls.
import { beforeEach, describe, expect, it } from 'vitest'
import { edgeRuns, edgeSummary, WALL_EDGES } from '../src/editor/edges.ts'
import { Stroke } from '../src/editor/history.ts'
import { CellType, Layers } from '../src/editor/layers.ts'
import { useEditor } from '../src/editor/store.ts'
import {
  choosePalette,
  makeEdge,
  newElement,
  pointerDown,
  pointerMove,
  pointerUp,
  setTool,
} from '../src/editor/tools/controller.ts'
import { newDocument } from '../src/scene/defaults.ts'

const NONE = { shift: false, alt: false }
const state = () => useEditor.getState()

function drag(from: [number, number], to: [number, number] = from) {
  pointerDown(from, NONE)
  pointerMove(to, NONE)
  pointerUp(to, NONE)
}

const cell = (x: number, y: number) => {
  const l = state().layers
  return [l.cellType[l.index(x, y)], l.elementId[l.index(x, y)]]
}

/** The cells of column `x` as `type:id`, bottom to top. */
const column = (x: number) =>
  Array.from({ length: state().layers.height }, (_, y) => cell(x, y).join(':'))

describe('edge runs', () => {
  it('are the scene edge kind where nothing is drawn', () => {
    const l = new Layers(6, 4)
    expect(edgeRuns(l, WALL_EDGES, 'left')).toEqual([
      { kind: 'wall', elementId: 0, start: 0, end: 4 },
    ])
    const periodic = { ...WALL_EDGES, bottom: 'periodic', top: 'periodic' } as const
    expect(edgeRuns(l, periodic, 'top')).toEqual([
      { kind: 'periodic', elementId: 0, start: 0, end: 6 },
    ])
  })

  it('follow the drawn inlets, outlets, walls and heat sources on the edge cells', () => {
    const l = new Layers(6, 4)
    const s = new Stroke(l)
    s.write(5, 0, CellType.Solid, 1)
    s.write(5, 1, CellType.Outlet, 3)
    s.write(5, 2, CellType.Outlet, 3)
    s.write(5, 3, CellType.HeatSource, 4)
    s.write(2, 0, CellType.Inlet, 2)
    expect(edgeRuns(l, WALL_EDGES, 'right')).toEqual([
      { kind: 'wall', elementId: 1, start: 0, end: 1 },
      { kind: 'outlet', elementId: 3, start: 1, end: 3 },
      { kind: 'wall', elementId: 4, start: 3, end: 4 },
    ])
    // Fluid (3, 0) and (4, 0) are the edge's wall, (5, 0) a drawn one: both walls, but runs keep
    // element ids apart.
    expect(edgeRuns(l, WALL_EDGES, 'bottom').map((r) => [r.kind, r.elementId, r.start])).toEqual([
      ['wall', 0, 0],
      ['inlet', 2, 2],
      ['wall', 0, 3],
      ['wall', 1, 5],
    ])
    // What each button would paint: inlets and outlets skip walls and heat sources, walls skip
    // inlets, outlets and heat sources.
    expect(edgeSummary(l, WALL_EDGES, 'right').targets).toEqual({
      wall: { cells: 1, sole: 1 },
      inlet: { cells: 2, sole: null },
      outlet: { cells: 2, sole: 3 },
    })
    expect(edgeSummary(l, WALL_EDGES, 'bottom').targets).toEqual({
      wall: { cells: 5, sole: null },
      inlet: { cells: 5, sole: null },
      outlet: { cells: 5, sole: null },
    })
  })
})

describe('Make wall / Make inlet / Make outlet', () => {
  beforeEach(() => {
    state().load(newDocument(40, 20))
    state().setPanelTab('scene')
    setTool('pencil')
    choosePalette('wall')
  })

  it('paints the whole edge and creates the element in one undo entry', () => {
    makeEdge('left', 'inlet')
    const { table, history, activeElement, selectedElement, panelTab } = state()
    expect(table.elements).toEqual([
      expect.objectContaining({
        id: 1,
        name: 'Inlet 1',
        kind: 'inlet',
        velocity: { type: 'uniform', value: [0.1, 0] },
      }),
    ])
    expect(column(0).every((c) => c === `${CellType.Inlet}:1`)).toBe(true)
    expect(cell(1, 5)).toEqual([CellType.Fluid, 0])
    expect(history.size.entries).toBe(1)
    expect(history.undoLabel).toBe('Left edge: inlet')
    expect(activeElement.inlet).toBe(1)
    expect(selectedElement).toBe(1)
    expect(panelTab).toBe('scene') // stays on the Scene tab, for the next edge
    expect(edgeSummary(state().layers, WALL_EDGES, 'left').targets.inlet).toEqual({
      cells: 20,
      sole: 1,
    })

    state().undo()
    expect(state().table.elements).toHaveLength(0)
    expect(column(0).every((c) => c === `${CellType.Fluid}:0`)).toBe(true)
    state().redo()
    expect(state().table.elements).toHaveLength(1)
    expect(cell(0, 7)).toEqual([CellType.Inlet, 1])
  })

  it('points edge inlets into the domain', () => {
    makeEdge('right', 'inlet')
    makeEdge('bottom', 'inlet')
    makeEdge('top', 'inlet')
    const velocities = state().table.elements.map((e) => e.kind === 'inlet' && e.velocity)
    expect(velocities).toEqual([
      { type: 'uniform', value: [-0.1, 0] },
      { type: 'uniform', value: [0, 0.1] },
      { type: 'uniform', value: [0, -0.1] },
    ])
  })

  it('gives the same cells as the drags of the Phase 4 acceptance scenario', () => {
    // Every edge by the buttons: the channel walls, then the inlet and outlet between them.
    makeEdge('bottom', 'wall')
    makeEdge('top', 'wall')
    makeEdge('left', 'inlet')
    makeEdge('right', 'outlet')
    const byButtons = state().layers

    state().load(newDocument(40, 20))
    setTool('rect')
    drag([0, 0], [39, 0])
    drag([0, 19], [39, 19])
    choosePalette('inlet')
    drag([0, 1], [0, 18])
    choosePalette('outlet')
    drag([39, 1], [39, 18])
    const byDragging = state().layers

    expect(byButtons.cellType).toEqual(byDragging.cellType)
    expect(byButtons.elementId).toEqual(byDragging.elementId)
    expect(state().table.elements.map((e) => `${e.id}:${e.kind}`)).toEqual([
      '1:wall',
      '2:inlet',
      '3:outlet',
    ])
  })

  it('makes walls with the active wall element: one element for the channel walls', () => {
    makeEdge('bottom', 'wall')
    makeEdge('top', 'wall')
    expect(state().table.elements).toEqual([
      expect.objectContaining({ id: 1, name: 'Wall 1', kind: 'wall' }),
    ])
    for (let x = 0; x < 40; x++) {
      expect(cell(x, 0)).toEqual([CellType.Solid, 1]) // corners included
      expect(cell(x, 19)).toEqual([CellType.Solid, 1])
    }
    expect(state().history.size.entries).toBe(2)
    expect(state().history.undoLabel).toBe('Top edge: wall')
    expect(state().panelTab).toBe('scene')
    state().undo() // the top row only: the element was created by the bottom one
    expect(cell(5, 19)).toEqual([CellType.Fluid, 0])
    expect(state().table.elements).toHaveLength(1)
    state().undo()
    expect(state().table.elements).toHaveLength(0)
    expect(cell(5, 0)).toEqual([CellType.Fluid, 0])
  })

  it('starts another wall element after N (a moving lid), and paints over the old one', () => {
    makeEdge('bottom', 'wall')
    makeEdge('top', 'wall')
    newElement()
    makeEdge('top', 'wall')
    expect(state().table.elements.map((e) => e.name)).toEqual(['Wall 1', 'Wall 2'])
    expect(cell(7, 19)).toEqual([CellType.Solid, 2])
    expect(cell(7, 0)).toEqual([CellType.Solid, 1])
    expect(edgeSummary(state().layers, WALL_EDGES, 'top').targets.wall).toEqual({
      cells: 40,
      sole: 2,
    })
  })

  it('keeps inlets and outlets when making a wall', () => {
    makeEdge('left', 'inlet')
    makeEdge('bottom', 'wall')
    expect(cell(0, 0)).toEqual([CellType.Inlet, 1])
    expect(cell(1, 0)).toEqual([CellType.Solid, 2])
  })

  it('replaces an earlier inlet or outlet on the edge (which keeps its element)', () => {
    makeEdge('left', 'inlet')
    makeEdge('left', 'outlet')
    expect(column(0).every((c) => c === `${CellType.Outlet}:2`)).toBe(true)
    expect(state().table.elements.map((e) => e.kind)).toEqual(['inlet', 'outlet'])
    state().undo()
    expect(column(0).every((c) => c === `${CellType.Inlet}:1`)).toBe(true)
  })

  it('does nothing on a periodic edge or an edge drawn wall from end to end', () => {
    state().updateTable('periodic', (t) => {
      t.grid.edges = { ...WALL_EDGES, left: 'periodic', right: 'periodic' }
    })
    makeEdge('left', 'inlet')
    expect(state().table.elements).toHaveLength(0)
    expect(state().history.size.entries).toBe(1) // the edge change only

    setTool('rect')
    newElement()
    drag([0, 0], [39, 0]) // a wall along the bottom edge
    makeEdge('bottom', 'outlet')
    expect(state().table.elements.map((e) => e.kind)).toEqual(['wall'])
    expect(state().history.undoLabel).toBe('Rectangle')
  })
})

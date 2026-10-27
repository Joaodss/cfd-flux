// The four domain edges (Q14 option B): what each one is along its length — the scene's edge kind
// (`grid.edges`: wall or periodic) where nothing is drawn on the edge cells, else the drawn wall,
// inlet or outlet — and the "Make wall/inlet/outlet" actions, which paint the edge cells.
import type { DomainEdges } from '../generated/scene.ts'
import type { Stroke } from './history.ts'
import { CellType, type Layers } from './layers.ts'

export const SIDES = ['left', 'right', 'bottom', 'top'] as const
export type Side = (typeof SIDES)[number]

export const SIDE_NAMES: Record<Side, string> = {
  left: 'Left',
  right: 'Right',
  bottom: 'Bottom',
  top: 'Top',
}

/** The unit vector into the domain from each edge (an edge inlet blows along it). */
export const INWARD: Record<Side, [number, number]> = {
  left: [1, 0],
  right: [-1, 0],
  bottom: [0, 1],
  top: [0, -1],
}

/** The edges of a scene without `grid.edges`: walls all around (03 §3). */
export const WALL_EDGES: DomainEdges = { left: 'wall', right: 'wall', bottom: 'wall', top: 'wall' }

/** What the boundary is along a stretch of an edge. */
export type EdgeKindShown = 'wall' | 'periodic' | 'inlet' | 'outlet'

/** A stretch of equal boundary along an edge: cells `start ≤ i < end` (see `edgeCell`). */
export interface EdgeRun {
  kind: EdgeKindShown
  /** The drawn element (0 where nothing is drawn: the scene's edge kind applies). */
  elementId: number
  start: number
  end: number
}

/** Number of cells along an edge. */
export function edgeLength(layers: Layers, side: Side): number {
  return side === 'left' || side === 'right' ? layers.height : layers.width
}

/** The `i`-th cell along an edge: left and right go bottom to top, bottom and top left to right. */
export function edgeCell(layers: Layers, side: Side, i: number): [number, number] {
  switch (side) {
    case 'left':
      return [0, i]
    case 'right':
      return [layers.width - 1, i]
    case 'bottom':
      return [i, 0]
    case 'top':
      return [i, layers.height - 1]
  }
}

/**
 * The boundary along an edge as runs: an inlet or outlet cell is that element; a drawn wall or
 * heat source is a wall; a fluid or empty cell leaves the scene's edge kind in place.
 */
export function edgeRuns(layers: Layers, edges: DomainEdges, side: Side): EdgeRun[] {
  const runs: EdgeRun[] = []
  const n = edgeLength(layers, side)
  for (let i = 0; i < n; i++) {
    const c = layers.index(...edgeCell(layers, side, i))
    const ct = layers.cellType[c]
    let kind: EdgeKindShown
    let elementId = layers.elementId[c]
    if (ct === CellType.Inlet) kind = 'inlet'
    else if (ct === CellType.Outlet) kind = 'outlet'
    else if (ct === CellType.Solid || ct === CellType.HeatSource) kind = 'wall'
    else {
      kind = edges[side]
      elementId = 0
    }
    const last = runs[runs.length - 1]
    if (last && last.kind === kind && last.elementId === elementId) last.end = i + 1
    else runs.push({ kind, elementId, start: i, end: i + 1 })
  }
  return runs
}

/** What the Scene tab's buttons make of an edge. */
export type EdgeAction = 'wall' | 'inlet' | 'outlet'
export const EDGE_ACTIONS: readonly EdgeAction[] = ['wall', 'inlet', 'outlet']

/** The cell type each action paints. */
const ACTION_CELL: Record<EdgeAction, CellType> = {
  wall: CellType.Solid,
  inlet: CellType.Inlet,
  outlet: CellType.Outlet,
}

/**
 * Whether an edge action repaints a cell. Inlets and outlets take every cell but drawn walls and
 * heat sources (so walls along the neighbouring edges keep their corners); walls take fluid, empty
 * and wall cells (another wall element's too) but leave inlets, outlets and heat sources.
 */
export function repaints(action: EdgeAction, cellType: number): boolean {
  if (cellType === CellType.HeatSource) return false
  if (action === 'wall') return cellType !== CellType.Inlet && cellType !== CellType.Outlet
  return cellType !== CellType.Solid
}

/** Paints the cells of an edge that `action` repaints with `elementId` (an element of its kind). */
export function paintEdge(stroke: Stroke, side: Side, action: EdgeAction, elementId: number): void {
  const l = stroke.layers
  const n = edgeLength(l, side)
  for (let i = 0; i < n; i++) {
    const [x, y] = edgeCell(l, side, i)
    if (repaints(action, l.cellType[l.index(x, y)])) {
      stroke.write(x, y, ACTION_CELL[action], elementId)
    }
  }
}

/** What an action would paint on an edge. */
export interface EdgeTarget {
  /** Cells the action repaints. */
  cells: number
  /** The element of the action's kind already on every one of those cells, if there is one. */
  sole: number | null
}

/** What the Scene tab shows about an edge and whether its buttons can act. */
export interface EdgeSummary {
  runs: EdgeRun[]
  targets: Record<EdgeAction, EdgeTarget>
}

export function edgeSummary(layers: Layers, edges: DomainEdges, side: Side): EdgeSummary {
  const n = edgeLength(layers, side)
  const target = (action: EdgeAction): EdgeTarget => {
    let cells = 0
    let sole: number | null = null
    for (let i = 0; i < n; i++) {
      const c = layers.index(...edgeCell(layers, side, i))
      if (!repaints(action, layers.cellType[c])) continue
      const id = layers.cellType[c] === ACTION_CELL[action] ? layers.elementId[c] : 0
      sole = cells === 0 ? id : sole === id ? sole : 0
      cells++
    }
    return { cells, sole: sole || null }
  }
  return {
    runs: edgeRuns(layers, edges, side),
    targets: { wall: target('wall'), inlet: target('inlet'), outlet: target('outlet') },
  }
}
